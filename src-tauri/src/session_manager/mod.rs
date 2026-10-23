pub mod providers;
pub mod terminal;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use providers::{claude, codex, gemini, grokbuild, hermes, mcode, openclaw, opencode, pi};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    pub provider_id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_active_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_command: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteSessionRequest {
    pub provider_id: String,
    pub session_id: String,
    pub source_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteSessionOutcome {
    pub provider_id: String,
    pub session_id: String,
    pub source_path: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexEncryptedContentRepairResult {
    pub session_id: String,
    pub source_path: String,
    pub removed_items: usize,
    pub backup_path: Option<String>,
}

/// Remove opaque reasoning items that cannot be replayed by a different Codex
/// backend. The original rollout is copied to a repair backup before writing.
pub fn repair_codex_encrypted_content(
    provider_id: &str,
    session_id: &str,
    source_path: &str,
) -> Result<CodexEncryptedContentRepairResult, String> {
    let codex_dir = crate::codex_config::get_codex_config_dir();
    let roots = codex::session_roots();
    repair_codex_encrypted_content_with_roots(
        provider_id,
        session_id,
        source_path,
        &roots,
        &codex_dir.join("session-repairs"),
    )
}

fn repair_codex_encrypted_content_with_roots(
    provider_id: &str,
    session_id: &str,
    source_path: &str,
    roots: &[PathBuf],
    repair_root: &Path,
) -> Result<CodexEncryptedContentRepairResult, String> {
    if provider_id != "codex" {
        return Err("Encrypted-content repair is only supported for Codex sessions".to_string());
    }
    if session_id.trim().is_empty() {
        return Err("Session ID cannot be empty".to_string());
    }
    if !session_id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return Err("Codex session ID contains invalid characters".to_string());
    }

    let source = Path::new(source_path);
    if source.extension().and_then(|extension| extension.to_str()) != Some("jsonl") {
        return Err("Codex session source must be a JSONL file".to_string());
    }
    let validated_source = canonicalize_existing_path(source, "session source")?;
    let inside_root = roots.iter().filter(|root| root.exists()).any(|root| {
        root.canonicalize()
            .map(|resolved| validated_source.starts_with(resolved))
            .unwrap_or(false)
    });
    if !inside_root {
        return Err(format!(
            "Session source path is outside Codex session roots: {}",
            source.display()
        ));
    }

    let original = fs::read(&validated_source)
        .map_err(|e| format!("Failed to read Codex session {}: {e}", validated_source.display()))?;
    let text = String::from_utf8(original.clone())
        .map_err(|e| format!("Codex session is not valid UTF-8: {e}"))?;
    let mut retained = Vec::new();
    let mut removed_items = 0usize;
    let mut found_session_id = false;

    for (line_no, line) in text.split_inclusive('\n').enumerate() {
        let json_text = line.trim_end_matches(['\r', '\n']);
        let value: Value = serde_json::from_str(json_text).map_err(|e| {
            format!("Invalid JSON in Codex session at line {}: {e}", line_no + 1)
        })?;
        if value.get("type").and_then(Value::as_str) == Some("session_meta") {
            let file_session_id = value
                .get("payload")
                .and_then(|payload| payload.get("id"))
                .and_then(Value::as_str);
            if file_session_id != Some(session_id) {
                return Err(format!(
                    "Codex session ID does not match the requested ID: {}",
                    validated_source.display()
                ));
            }
            found_session_id = true;
        }
        let is_encrypted_reasoning = value.get("type").and_then(Value::as_str)
            == Some("response_item")
            && value
                .get("payload")
                .and_then(Value::as_object)
                .map(|payload| {
                    payload.get("type").and_then(Value::as_str) == Some("reasoning")
                        && payload
                            .get("encrypted_content")
                            .and_then(Value::as_str)
                            .map(|content| !content.is_empty())
                            .unwrap_or(false)
                })
                .unwrap_or(false);
        if is_encrypted_reasoning {
            removed_items += 1;
            continue;
        }
        retained.push(line);
    }
    // split_inclusive omits a final empty segment, so preserve an empty file and
    // retain the exact bytes of every non-removed line.
    if text.is_empty() {
        return Err("Codex session is empty".to_string());
    }
    if !found_session_id {
        return Err("Codex session metadata is missing".to_string());
    }

    let backup_path = if removed_items > 0 {
        let session_repair_root = repair_root.join(session_id);
        fs::create_dir_all(&session_repair_root).map_err(|e| {
            format!(
                "Failed to create repair backup directory {}: {e}",
                session_repair_root.display()
            )
        })?;
        let mut backup_file = None;
        let mut last_error = None;
        for _ in 0..16 {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let backup = session_repair_root.join(format!("{stamp}-original.jsonl"));
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&backup)
            {
                Ok(mut file) => {
                    if let Err(error) = file.write_all(&original).and_then(|_| file.sync_all()) {
                        let _ = fs::remove_file(&backup);
                        return Err(format!("Failed to back up Codex session: {error}"));
                    }
                    backup_file = Some((backup, file));
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    last_error = Some(error);
                }
                Err(error) => return Err(format!("Failed to back up Codex session: {error}")),
            }
        }
        let Some((backup, backup_file)) = backup_file else {
            return Err(format!(
                "Failed to create a unique Codex session backup: {}",
                last_error
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "too many name collisions".to_string())
            ));
        };
        drop(backup_file);
        if fs::read(&validated_source)
            .map_err(|e| format!("Failed to recheck Codex session before writing: {e}"))?
            != original
        {
            return Err("Codex session changed during repair; the backup was kept and the file was not modified".to_string());
        }
        let repaired = retained.concat().into_bytes();
        crate::config::atomic_write(&validated_source, &repaired)
            .map_err(|e| format!("Failed to write repaired Codex session: {e}"))?;
        Some(backup.to_string_lossy().into_owned())
    } else {
        None
    };

    Ok(CodexEncryptedContentRepairResult {
        session_id: session_id.to_string(),
        source_path: validated_source.to_string_lossy().into_owned(),
        removed_items,
        backup_path,
    })
}

pub fn scan_sessions() -> Vec<SessionMeta> {
    let (r1, r2, r3, r4, r5, r6, r7, r8) = std::thread::scope(|s| {
        let h1 = s.spawn(codex::scan_sessions);
        let h2 = s.spawn(claude::scan_sessions);
        let h3 = s.spawn(opencode::scan_sessions);
        let h4 = s.spawn(openclaw::scan_sessions);
        let h5 = s.spawn(gemini::scan_sessions);
        let h6 = s.spawn(hermes::scan_sessions);
        let h7 = s.spawn(grokbuild::scan_sessions);
        let h8 = s.spawn(pi::scan_sessions);
        (
            h1.join().unwrap_or_default(),
            h2.join().unwrap_or_default(),
            h3.join().unwrap_or_default(),
            h4.join().unwrap_or_default(),
            h5.join().unwrap_or_default(),
            h6.join().unwrap_or_default(),
            h7.join().unwrap_or_default(),
            h8.join().unwrap_or_default(),
        )
    });

    let mut sessions = Vec::new();
    sessions.extend(r1);
    sessions.extend(r2);
    sessions.extend(r3);
    sessions.extend(r4);
    sessions.extend(r5);
    sessions.extend(r6);
    sessions.extend(r7);
    sessions.extend(r8);
    sessions.extend(mcode::scan_sessions());

    sessions.sort_by(|a, b| {
        let a_ts = a.last_active_at.or(a.created_at).unwrap_or(0);
        let b_ts = b.last_active_at.or(b.created_at).unwrap_or(0);
        b_ts.cmp(&a_ts)
    });

    sessions
}

pub fn load_messages(provider_id: &str, source_path: &str) -> Result<Vec<SessionMessage>, String> {
    if provider_id == "mcode" {
        return mcode::load_messages(source_path);
    }
    // SQLite sessions use a "sqlite:" prefixed source_path
    if provider_id == "opencode" && source_path.starts_with("sqlite:") {
        return opencode::load_messages_sqlite(source_path);
    }
    if provider_id == "hermes" && source_path.starts_with("sqlite:") {
        return hermes::load_messages_sqlite(source_path);
    }

    let path = Path::new(source_path);
    match provider_id {
        "codex" => codex::load_messages(path),
        "claude" => claude::load_messages(path),
        "opencode" => opencode::load_messages(path),
        "openclaw" => openclaw::load_messages(path),
        "gemini" => gemini::load_messages(path),
        "grokbuild" => grokbuild::load_messages(path),
        "hermes" => hermes::load_messages(path),
        "pi" => pi::load_messages(path),
        _ => Err(format!("Unsupported provider: {provider_id}")),
    }
}

pub fn delete_session(
    provider_id: &str,
    session_id: &str,
    source_path: &str,
) -> Result<bool, String> {
    if provider_id == "mcode" {
        return Err(
            "Delete this session in MCode so its runtime state and history are removed together"
                .into(),
        );
    }
    // SQLite sessions bypass the file-based deletion path
    if provider_id == "opencode" && source_path.starts_with("sqlite:") {
        return opencode::delete_session_sqlite(session_id, source_path);
    }
    if provider_id == "hermes" && source_path.starts_with("sqlite:") {
        return hermes::delete_session_sqlite(session_id, source_path);
    }

    let roots = provider_roots(provider_id)?;
    delete_session_with_roots(provider_id, session_id, Path::new(source_path), &roots)
}

pub fn delete_sessions(requests: &[DeleteSessionRequest]) -> Vec<DeleteSessionOutcome> {
    collect_delete_session_outcomes(requests, |request| {
        delete_session(
            &request.provider_id,
            &request.session_id,
            &request.source_path,
        )
    })
}

fn delete_session_with_roots(
    provider_id: &str,
    session_id: &str,
    source_path: &Path,
    roots: &[PathBuf],
) -> Result<bool, String> {
    let validated_source = canonicalize_existing_path(source_path, "session source")?;

    let mut saw_existing_root = false;
    for root in roots {
        if !root.exists() {
            continue;
        }

        saw_existing_root = true;
        let validated_root = canonicalize_existing_path(root, "session root")?;
        if validated_source.starts_with(&validated_root) {
            return match provider_id {
                "codex" => codex::delete_session(&validated_root, &validated_source, session_id),
                "claude" => claude::delete_session(&validated_root, &validated_source, session_id),
                "opencode" => {
                    opencode::delete_session(&validated_root, &validated_source, session_id)
                }
                "openclaw" => {
                    openclaw::delete_session(&validated_root, &validated_source, session_id)
                }
                "gemini" => gemini::delete_session(&validated_root, &validated_source, session_id),
                "grokbuild" => {
                    grokbuild::delete_session(&validated_root, &validated_source, session_id)
                }
                "hermes" => hermes::delete_session(&validated_root, &validated_source, session_id),
                "pi" => pi::delete_session(&validated_root, &validated_source, session_id),
                _ => Err(format!("Unsupported provider: {provider_id}")),
            };
        }
    }

    if !saw_existing_root {
        return Err(format!(
            "Session root not found for provider {provider_id}: {}",
            roots
                .first()
                .map(|root| root.display().to_string())
                .unwrap_or_else(|| "<none>".to_string())
        ));
    }

    Err(format!(
        "Session source path is outside provider roots: {}",
        source_path.display()
    ))
}

fn provider_roots(provider_id: &str) -> Result<Vec<PathBuf>, String> {
    let roots = match provider_id {
        "codex" => codex::session_roots(),
        "claude" => vec![crate::config::get_claude_config_dir().join("projects")],
        "opencode" => vec![opencode::get_opencode_data_dir()],
        "openclaw" => vec![crate::openclaw_config::get_openclaw_dir().join("agents")],
        "gemini" => vec![crate::gemini_config::get_gemini_dir().join("tmp")],
        "grokbuild" => grokbuild::session_roots(),
        "hermes" => vec![crate::hermes_config::get_hermes_dir().join("sessions")],
        "pi" => pi::session_roots(),
        _ => return Err(format!("Unsupported provider: {provider_id}")),
    };

    Ok(roots)
}

fn canonicalize_existing_path(path: &Path, label: &str) -> Result<PathBuf, String> {
    if !path.exists() {
        return Err(format!("{label} not found: {}", path.display()));
    }

    path.canonicalize()
        .map_err(|e| format!("Failed to resolve {label} {}: {e}", path.display()))
}

fn collect_delete_session_outcomes<F>(
    requests: &[DeleteSessionRequest],
    mut deleter: F,
) -> Vec<DeleteSessionOutcome>
where
    F: FnMut(&DeleteSessionRequest) -> Result<bool, String>,
{
    requests
        .iter()
        .map(|request| match deleter(request) {
            Ok(true) => DeleteSessionOutcome {
                provider_id: request.provider_id.clone(),
                session_id: request.session_id.clone(),
                source_path: request.source_path.clone(),
                success: true,
                error: None,
            },
            Ok(false) => DeleteSessionOutcome {
                provider_id: request.provider_id.clone(),
                session_id: request.session_id.clone(),
                source_path: request.source_path.clone(),
                success: false,
                error: Some("Session was not deleted".to_string()),
            },
            Err(error) => DeleteSessionOutcome {
                provider_id: request.provider_id.clone(),
                session_id: request.session_id.clone(),
                source_path: request.source_path.clone(),
                success: false,
                error: Some(error),
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_codex_session(path: &Path, session_id: &str) {
        std::fs::write(
            path,
            format!(
                "{{\"timestamp\":\"2026-03-06T21:50:12Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{session_id}\",\"cwd\":\"/tmp/project\"}}}}\n\
                 {{\"timestamp\":\"2026-03-06T21:50:13Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":\"hello\"}}}}\n",
            ),
        )
        .expect("write source");
    }

    #[test]
    fn repair_codex_encrypted_content_backs_up_and_removes_encrypted_reasoning() {
        let temp = tempdir().expect("tempdir");
        let sessions = temp.path().join("sessions");
        let repair_root = temp.path().join("repairs");
        std::fs::create_dir_all(&sessions).expect("sessions");
        let path = sessions.join("session.jsonl");
        let original = concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"session-1\"}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":\"hello\"}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"reasoning\",\"encrypted_content\":\"opaque\"}}\n",
        );
        std::fs::write(&path, original).expect("write");

        let result = repair_codex_encrypted_content_with_roots(
            "codex",
            "session-1",
            path.to_str().unwrap(),
            &[sessions],
            &repair_root,
        )
        .expect("repair");
        assert_eq!(result.removed_items, 1);
        let backup_path = result.backup_path.expect("backup");
        assert_eq!(std::fs::read(backup_path).expect("read backup"), original.as_bytes());
        let repaired = std::fs::read_to_string(&path).expect("read repaired");
        assert!(repaired.contains("\"role\":\"user\""));
        assert!(!repaired.contains("encrypted_content"));
    }

    #[test]
    fn repair_codex_encrypted_content_rejects_mismatched_session_id() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"other\"}}\n",
        )
        .expect("write");
        let error = repair_codex_encrypted_content_with_roots(
            "codex",
            "session-1",
            path.to_str().unwrap(),
            &[temp.path().to_path_buf()],
            &temp.path().join("repairs"),
        )
        .expect_err("mismatch should fail");
        assert!(error.contains("does not match"));
    }

    #[test]
    fn accepts_source_path_under_any_allowed_provider_root() {
        let active_root = tempdir().expect("active root");
        let archived_root = tempdir().expect("archived root");
        let source = archived_root.path().join("session.jsonl");
        write_codex_session(&source, "archived-session");

        let deleted = delete_session_with_roots(
            "codex",
            "archived-session",
            &source,
            &[
                active_root.path().to_path_buf(),
                archived_root.path().to_path_buf(),
            ],
        )
        .expect("delete archived session");

        assert!(deleted);
        assert!(!source.exists());
    }

    #[test]
    fn rejects_source_path_outside_provider_root() {
        let root = tempdir().expect("tempdir");
        let outside = tempdir().expect("tempdir");
        let source = outside.path().join("session.jsonl");
        std::fs::write(&source, "{}").expect("write source");

        let err =
            delete_session_with_roots("codex", "session-1", &source, &[root.path().to_path_buf()])
                .expect_err("expected outside-root path to be rejected");

        assert!(err.contains("outside provider roots"));
    }

    #[test]
    fn rejects_missing_source_path() {
        let root = tempdir().expect("tempdir");
        let missing = root.path().join("missing.jsonl");

        let err =
            delete_session_with_roots("codex", "session-1", &missing, &[root.path().to_path_buf()])
                .expect_err("expected missing source path to fail");

        assert!(err.contains("session source not found"));
    }

    #[test]
    fn batch_delete_collects_successes_and_failures_in_order() {
        let requests = vec![
            DeleteSessionRequest {
                provider_id: "codex".to_string(),
                session_id: "s1".to_string(),
                source_path: "/tmp/s1".to_string(),
            },
            DeleteSessionRequest {
                provider_id: "claude".to_string(),
                session_id: "s2".to_string(),
                source_path: "/tmp/s2".to_string(),
            },
            DeleteSessionRequest {
                provider_id: "gemini".to_string(),
                session_id: "s3".to_string(),
                source_path: "/tmp/s3".to_string(),
            },
        ];

        let outcomes = collect_delete_session_outcomes(&requests, |request| {
            match request.session_id.as_str() {
                "s1" => Ok(true),
                "s2" => Err("boom".to_string()),
                _ => Ok(false),
            }
        });

        assert_eq!(outcomes.len(), 3);
        assert!(outcomes[0].success);
        assert_eq!(outcomes[0].error, None);
        assert!(!outcomes[1].success);
        assert_eq!(outcomes[1].error.as_deref(), Some("boom"));
        assert!(!outcomes[2].success);
        assert_eq!(
            outcomes[2].error.as_deref(),
            Some("Session was not deleted")
        );
    }
}
