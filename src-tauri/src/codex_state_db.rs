//! Locating Codex's per-thread state SQLite databases.
//!
//! Codex stores thread metadata in `state_5.sqlite`, normally inside the Codex
//! config dir (`CODEX_HOME` / `~/.codex`). The SQLite location can be moved with
//! the `sqlite_home` key in `config.toml` or the `CODEX_SQLITE_HOME` env var;
//! when set, a second DB lives there. Both history migration and the session
//! list's title lookup need the same resolution, so it lives here once.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension};
use toml_edit::DocumentMut;

use crate::config::get_home_dir;

/// Filename of Codex's per-thread state database. Codex bumps the version
/// number across releases; update this single source of truth when a new state
/// DB version ships.
pub(crate) const CODEX_STATE_DB_FILENAME: &str = "state_5.sqlite";

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
}
