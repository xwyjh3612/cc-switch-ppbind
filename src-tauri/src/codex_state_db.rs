//! Locating Codex's per-thread state SQLite databases.
//!
//! Codex stores thread metadata in `state_5.sqlite`, normally inside the Codex
//! config dir (`CODEX_HOME` / `~/.codex`). The SQLite location can be moved with
//! the `sqlite_home` key in `config.toml` or the `CODEX_SQLITE_HOME` env var;
//! when set, a second DB lives there. Both history migration and the session
//! list's title lookup need the same resolution, so it lives here once.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension};
use toml_edit::DocumentMut;

use crate::config::get_home_dir;

/// Filename of Codex's per-thread state database. Codex bumps the version
/// number across releases; update this single source of truth when a new state
/// DB version ships.
pub(crate) const CODEX_STATE_DB_FILENAME: &str = "state_5.sqlite";

/// Prefix used by Codex's versioned log database (`logs_2.sqlite`, ...).
const CODEX_LOG_DB_PREFIX: &str = "logs_";

/// Env var that overrides the Codex SQLite state directory.
const CODEX_SQLITE_HOME_ENV: &str = "CODEX_SQLITE_HOME";

/// Resolve every candidate `state_5.sqlite` path: the config-dir DB plus, when
/// Codex is configured to keep its SQLite state elsewhere, that DB too.
///
/// `config_dir` is the Codex config dir (`~/.codex`); `config_text` is the raw
/// `config.toml` contents, used to detect a `sqlite_home` override.
pub(crate) fn codex_state_db_paths(config_dir: &Path, config_text: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    push_unique_path(&mut paths, config_dir.join(CODEX_STATE_DB_FILENAME));
    // Codex lets SQLite state move away from CODEX_HOME; config takes precedence.
    if let Some(sqlite_home) = sqlite_home_from_codex_config(config_text) {
        push_unique_path(&mut paths, sqlite_home.join(CODEX_STATE_DB_FILENAME));
    } else if let Some(sqlite_home) = sqlite_home_from_env() {
        push_unique_path(&mut paths, sqlite_home.join(CODEX_STATE_DB_FILENAME));
    }
    paths
}

/// Resolve Codex log database candidates from newest schema version to oldest.
///
/// The log DB is only a fallback for ephemeral threads that never appear in
/// `state_5.sqlite` (for example Codex's internal title-generation session).
/// It has a `thread_id` index, so looking up one ephemeral thread is cheap.
pub(crate) fn codex_log_db_paths(config_dir: &Path, config_text: &str) -> Vec<PathBuf> {
    let mut dirs = vec![config_dir.to_path_buf()];
    if let Some(sqlite_home) = sqlite_home_from_codex_config(config_text) {
        push_unique_path(&mut dirs, sqlite_home);
    } else if let Some(sqlite_home) = sqlite_home_from_env() {
        push_unique_path(&mut dirs, sqlite_home);
    }

    let mut candidates = Vec::new();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let Some(version) = codex_log_db_version(name) else {
                continue;
            };
            candidates.push((version, path));
        }
    }

    candidates.sort_by(|left, right| right.0.cmp(&left.0));
    let mut paths = Vec::new();
    for (_, path) in candidates {
        push_unique_path(&mut paths, path);
    }
    paths
}

/// Look up a Codex thread's working directory from its state database.
///
/// Codex writes thread metadata before the first upstream request, so this is
/// more reliable than rollout-file scanning for project-scoped routing. Read
/// failures are intentionally best-effort: callers keep their existing
/// discovery fallback.
pub(crate) fn lookup_codex_thread_cwd(db_paths: &[PathBuf], thread_id: &str) -> Option<PathBuf> {
    let thread_id = thread_id.trim();
    if thread_id.is_empty() {
        return None;
    }

    for db_path in db_paths {
        if !db_path.exists() {
            continue;
        }

        let conn = match Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) {
            Ok(conn) => conn,
            Err(err) => {
                log::debug!(
                    "Failed to open Codex state database for cwd lookup {}: {err}",
                    db_path.display()
                );
                continue;
            }
        };

        // Codex may still be committing metadata while the proxy receives the
        // request. A short busy timeout avoids treating that as a cache miss.
        let _ = conn.busy_timeout(Duration::from_millis(750));
        let cwd = conn
            .query_row(
                "SELECT cwd FROM threads WHERE id = ?1 LIMIT 1",
                [thread_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .ok()
            .flatten()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        if cwd.is_some() {
            return cwd;
        }
    }

    None
}

/// Look up an ephemeral Codex thread's cwd from its local diagnostic logs.
///
/// Codex logs the sampling span with `cwd=<absolute path>` before issuing the
/// upstream request. This covers internal helper threads that are absent from
/// `threads`, without guessing which project the request belongs to.
pub(crate) fn lookup_codex_thread_cwd_from_logs(
    db_paths: &[PathBuf],
    thread_id: &str,
) -> Option<PathBuf> {
    let thread_id = thread_id.trim();
    if thread_id.is_empty() {
        return None;
    }

    for db_path in db_paths {
        if !db_path.exists() {
            continue;
        }

        let conn = match Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) {
            Ok(conn) => conn,
            Err(err) => {
                log::debug!(
                    "Failed to open Codex log database for cwd lookup {}: {err}",
                    db_path.display()
                );
                continue;
            }
        };
        let _ = conn.busy_timeout(Duration::from_millis(250));

        let mut stmt = match conn.prepare(
            "SELECT feedback_log_body
             FROM logs
             WHERE thread_id = ?1
               AND feedback_log_body LIKE '%run_sampling_request%'
               AND instr(feedback_log_body, 'cwd=') > 0
             ORDER BY ts DESC, ts_nanos DESC, id DESC
             LIMIT 16",
        ) {
            Ok(stmt) => stmt,
            Err(err) => {
                log::debug!(
                    "Failed to prepare Codex log cwd lookup {}: {err}",
                    db_path.display()
                );
                continue;
            }
        };

        let rows = match stmt.query_map([thread_id], |row| row.get::<_, String>(0)) {
            Ok(rows) => rows,
            Err(err) => {
                log::debug!(
                    "Failed to query Codex log cwd lookup {}: {err}",
                    db_path.display()
                );
                continue;
            }
        };
        for body in rows.flatten() {
            if let Some(cwd) = extract_cwd_from_codex_log(&body) {
                return Some(cwd);
            }
        }
    }

    None
}

fn codex_log_db_version(name: &str) -> Option<u32> {
    let stem = name.strip_suffix(".sqlite")?;
    if stem == "logs" {
        return Some(0);
    }
    stem.strip_prefix(CODEX_LOG_DB_PREFIX)?.parse().ok()
}

fn extract_cwd_from_codex_log(body: &str) -> Option<PathBuf> {
    let start = body.find("cwd=")? + "cwd=".len();
    let rest = &body[start..];
    let end = rest.find(['}', '\r', '\n']).unwrap_or(rest.len());
    let raw = rest[..end].trim().trim_matches('"');
    if raw.is_empty() {
        return None;
    }
    let path = PathBuf::from(raw);
    path.is_absolute().then_some(path)
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.contains(&path) {
        paths.push(path);
    }
}

fn sqlite_home_from_codex_config(config_text: &str) -> Option<PathBuf> {
    let doc = config_text.parse::<DocumentMut>().ok()?;
    let raw = doc.get("sqlite_home")?.as_str()?.trim();
    if raw.is_empty() {
        return None;
    }
    Some(resolve_user_path(raw))
}

fn sqlite_home_from_env() -> Option<PathBuf> {
    let raw = std::env::var(CODEX_SQLITE_HOME_ENV).ok()?;
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    Some(resolve_user_path(raw))
}

fn resolve_user_path(raw: &str) -> PathBuf {
    if raw == "~" {
        return get_home_dir();
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return get_home_dir().join(rest);
    }
    if let Some(rest) = raw.strip_prefix("~\\") {
        return get_home_dir().join(rest);
    }
    PathBuf::from(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn includes_config_sqlite_home() {
        let temp = tempdir().expect("tempdir");
        let sqlite_home = temp.path().join("sqlite-home");
        // 用 TOML 字面量字符串(单引号)承载路径：Windows 路径含反斜杠，basic string(双引号)
        // 会把 `\U`/`\s` 等当作非法转义导致解析失败。
        let config_text = format!("sqlite_home = '{}'\n", sqlite_home.display());

        let paths = codex_state_db_paths(temp.path(), &config_text);

        assert_eq!(
            paths,
            vec![
                temp.path().join(CODEX_STATE_DB_FILENAME),
                sqlite_home.join(CODEX_STATE_DB_FILENAME),
            ]
        );
    }

    #[test]
    fn looks_up_thread_cwd() {
        let temp = tempdir().expect("tempdir");
        let db_path = temp.path().join(CODEX_STATE_DB_FILENAME);
        let conn = Connection::open(&db_path).expect("open test db");
        conn.execute(
            "CREATE TABLE threads (id TEXT PRIMARY KEY, cwd TEXT NOT NULL)",
            [],
        )
        .expect("create threads table");
        conn.execute(
            "INSERT INTO threads (id, cwd) VALUES (?1, ?2)",
            ("thread-1", r"\\?\D:\AiCli\conversation\测活"),
        )
        .expect("insert thread");
        drop(conn);

        assert_eq!(
            lookup_codex_thread_cwd(std::slice::from_ref(&db_path), "thread-1"),
            Some(PathBuf::from(r"\\?\D:\AiCli\conversation\测活"))
        );
        assert_eq!(lookup_codex_thread_cwd(&[], "thread-1"), None);
    }

    #[test]
    fn looks_up_ephemeral_thread_cwd_from_logs() {
        let temp = tempdir().expect("tempdir");
        let db_path = temp.path().join("logs_2.sqlite");
        let conn = Connection::open(&db_path).expect("open test db");
        conn.execute(
            "CREATE TABLE logs (
                id INTEGER PRIMARY KEY,
                ts INTEGER NOT NULL,
                ts_nanos INTEGER NOT NULL,
                thread_id TEXT,
                feedback_log_body TEXT
            )",
            [],
        )
        .expect("create logs table");
        conn.execute(
            "INSERT INTO logs (id, ts, ts_nanos, thread_id, feedback_log_body)
             VALUES (1, 1, 0, ?1, ?2)",
            (
                "ephemeral-thread",
                r"run_sampling_request{turn_id=t1 model=gpt-5.6-sol cwd=D:\AiCli\conversation\测活}:try_run_sampling_request",
            ),
        )
        .expect("insert log");
        drop(conn);

        assert_eq!(
            lookup_codex_thread_cwd_from_logs(std::slice::from_ref(&db_path), "ephemeral-thread"),
            Some(PathBuf::from(r"D:\AiCli\conversation\测活"))
        );
        assert_eq!(
            lookup_codex_thread_cwd_from_logs(&[], "ephemeral-thread"),
            None
        );
    }
}
