//! Project-scoped provider routing and project path helpers.
//!
//! A project is identified by the normalized working directory discovered from a
//! session. The database stores only explicit provider overrides; all other
//! requests continue to use the existing global provider selection.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, RwLock};
use std::time::{Duration, Instant};

use crate::database::{Database, ProjectProviderRoute, SessionProviderRoute};
use crate::error::AppError;
use crate::session_manager::SessionMeta;

/// Normalize a project path for stable lookup across Windows path spelling.
///
/// Existing paths are canonicalized when possible. Non-existing paths remain
/// absolute and are normalized for separators and trailing slashes.
pub fn normalize_project_path(path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }

    let path = Path::new(trimmed);
    let normalized = path.canonicalize().unwrap_or_else(|_| make_absolute(path));
    let mut value = normalized.to_string_lossy().replace('\\', "/");

    while value.len() > 1 && value.ends_with('/') {
        #[cfg(target_os = "windows")]
        if value.len() == 3 && value.as_bytes().get(1) == Some(&b':') {
            break;
        }
        value.pop();
    }

    #[cfg(target_os = "windows")]
    {
        value = value.to_ascii_lowercase();
    }

    Some(value)
}

fn make_absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

/// Build project groups from scanned sessions. Sessions without a directory are
/// intentionally omitted because they cannot be routed safely by project.
pub fn group_sessions_by_project(
    sessions: Vec<crate::session_manager::SessionMeta>,
) -> Vec<ProjectGroup> {
    use std::collections::BTreeMap;

    let mut groups: BTreeMap<String, ProjectGroup> = BTreeMap::new();
    for session in sessions {
        let Some(raw_dir) = session.project_dir.as_deref() else {
            continue;
        };
        let Some(path_key) = normalize_project_path(raw_dir) else {
            continue;
        };
        let entry = groups
            .entry(path_key.clone())
            .or_insert_with(|| ProjectGroup {
                path_key,
                project_path: raw_dir.to_string(),
                sessions: Vec::new(),
            });
        if session.project_dir.as_deref() != Some(entry.project_path.as_str()) {
            entry.project_path = raw_dir.to_string();
        }
        entry.sessions.push(session);
    }

    let mut result: Vec<_> = groups.into_values().collect();
    for group in &mut result {
        group.sessions.sort_by(|a, b| {
            b.last_active_at
                .or(b.created_at)
                .unwrap_or(0)
                .cmp(&a.last_active_at.or(a.created_at).unwrap_or(0))
        });
    }
    result.sort_by(|a, b| {
        let a_ts = a
            .sessions
            .first()
            .and_then(|s| s.last_active_at.or(s.created_at))
            .unwrap_or(0);
        let b_ts = b
            .sessions
            .first()
            .and_then(|s| s.last_active_at.or(s.created_at))
            .unwrap_or(0);
        b_ts.cmp(&a_ts)
    });
    result
}

#[derive(Debug, Clone)]
pub struct ProjectGroup {
    pub path_key: String,
    pub project_path: String,
    pub sessions: Vec<SessionMeta>,
}

#[derive(Clone)]
struct CachedSessionProject {
    project_dir: Option<String>,
    expires_at: Instant,
}

static SESSION_PROJECT_CACHE: LazyLock<RwLock<HashMap<(String, String), CachedSessionProject>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

const SESSION_PROJECT_POSITIVE_TTL: Duration = Duration::from_secs(30 * 60);
const SESSION_PROJECT_NEGATIVE_TTL: Duration = Duration::from_secs(5);

/// Resolve the project directory for a client-provided session ID.
///
/// Codex stores the authoritative `thread_id -> cwd` mapping in its state DB
/// before sending the first upstream request. Prefer that lookup over scanning
/// rollout files, which may not have been flushed yet. Negative lookups are
/// cached briefly so a burst of sub-requests does not repeatedly scan history.
pub fn lookup_session_project_dir(app_type: &str, session_id: &str) -> Option<String> {
    if session_id.trim().is_empty() || !matches!(app_type, "codex" | "claude") {
        return None;
    }

    let lookup_id = if app_type == "codex" {
        session_id.strip_prefix("codex_").unwrap_or(session_id)
    } else {
        session_id
    };
    let cache_key = (app_type.to_string(), lookup_id.to_string());
    if let Ok(cache) = SESSION_PROJECT_CACHE.read() {
        if let Some(entry) = cache.get(&cache_key) {
            if Instant::now() < entry.expires_at {
                return entry.project_dir.clone();
            }
        }
    }

    let project_dir = lookup_session_project_dir_uncached(app_type, lookup_id);
    let ttl = if project_dir.is_some() {
        SESSION_PROJECT_POSITIVE_TTL
    } else {
        SESSION_PROJECT_NEGATIVE_TTL
    };
    if let Ok(mut cache) = SESSION_PROJECT_CACHE.write() {
        cache.insert(
            cache_key,
            CachedSessionProject {
                project_dir: project_dir.clone(),
                expires_at: Instant::now() + ttl,
            },
        );
    }
    project_dir
}

fn lookup_session_project_dir_uncached(app_type: &str, lookup_id: &str) -> Option<String> {
    if app_type == "codex" {
        let config_dir = crate::codex_config::get_codex_config_dir();
        let config_text = crate::codex_config::read_codex_config_text().unwrap_or_default();
        let state_db_paths = crate::codex_state_db::codex_state_db_paths(&config_dir, &config_text);
        if let Some(cwd) =
            crate::codex_state_db::lookup_codex_thread_cwd(&state_db_paths, lookup_id)
        {
            let project_dir = cwd.to_string_lossy().to_string();
            log::debug!(
                "Resolved Codex session {} project from state DB: {}",
                lookup_id,
                project_dir
            );
            return Some(project_dir);
        }
    }

    crate::session_manager::scan_sessions()
        .into_iter()
        .find(|session| session.provider_id == app_type && session.session_id == lookup_id)
        .and_then(|session| session.project_dir)
}
const LEGACY_FORCE_MODEL_OPTIONS_SETTING_KEY: &str = "project_force_model_options";
const CODEX_FORCE_MODEL_OPTIONS_SETTING_KEY: &str = "project_force_model_options_codex";
const CLAUDE_FORCE_MODEL_OPTIONS_SETTING_KEY: &str = "project_force_model_options_claude";

#[derive(Debug, Clone)]
pub struct RouteOverride {
    pub provider_id: Option<String>,
    pub force_model: Option<String>,
}

/// Codex proxy requests prefix the client session ID; local history does not.
/// Store and resolve session routes under one canonical ID.
pub fn normalize_session_id(app_type: &str, session_id: &str) -> String {
    let session_id = session_id.trim();
    if app_type == "codex" {
        session_id
            .strip_prefix("codex_")
            .unwrap_or(session_id)
            .to_string()
    } else {
        session_id.to_string()
    }
}

fn normalize_force_model_option(model: &str) -> Result<String, AppError> {
    let model = model.trim();
    if model.is_empty() {
        return Err(AppError::InvalidInput("模型名称不能为空".to_string()));
    }
    if model.len() > 200 || model.contains(['\r', '\n']) {
        return Err(AppError::InvalidInput("模型名称格式不正确".to_string()));
    }
    Ok(model.to_string())
}

fn force_model_options_setting_key(app_type: &str) -> Result<&'static str, AppError> {
    match app_type {
        "codex" => Ok(CODEX_FORCE_MODEL_OPTIONS_SETTING_KEY),
        "claude" => Ok(CLAUDE_FORCE_MODEL_OPTIONS_SETTING_KEY),
        _ => Err(AppError::InvalidInput(format!(
            "不支持的项目客户端: {app_type}"
        ))),
    }
}

fn parse_force_models(raw: &str) -> Vec<String> {
    let parsed = match serde_json::from_str::<Vec<String>>(raw) {
        Ok(parsed) => parsed,
        Err(error) => {
            log::warn!("解析项目强制模型列表失败，将使用空列表: {error}");
            Vec::new()
        }
    };

    let mut models = Vec::new();
    for model in parsed {
        let Ok(model) = normalize_force_model_option(&model) else {
            continue;
        };
        if !models.contains(&model) {
            models.push(model);
        }
    }
    models
}

pub fn list_force_models(db: &Database, app_type: &str) -> Result<Vec<String>, AppError> {
    let setting_key = force_model_options_setting_key(app_type)?;
    if let Some(raw) = db.get_setting(setting_key)? {
        return Ok(parse_force_models(&raw));
    }

    // 兼容旧版共用列表：每个客户端首次读取时复制一份，之后各自独立维护。
    let models = db
        .get_setting(LEGACY_FORCE_MODEL_OPTIONS_SETTING_KEY)?
        .map(|raw| parse_force_models(&raw))
        .unwrap_or_default();
    save_force_models(db, app_type, &models)?;
    Ok(models)
}

fn save_force_models(db: &Database, app_type: &str, models: &[String]) -> Result<(), AppError> {
    let setting_key = force_model_options_setting_key(app_type)?;
    let json = serde_json::to_string(models)
        .map_err(|error| AppError::Database(format!("序列化强制模型列表失败: {error}")))?;
    db.set_setting(setting_key, &json)
}

pub fn add_force_model(
    db: &Database,
    app_type: &str,
    model: &str,
) -> Result<Vec<String>, AppError> {
    let model = normalize_force_model_option(model)?;
    let mut models = list_force_models(db, app_type)?;
    if !models.contains(&model) {
        models.push(model);
        save_force_models(db, app_type, &models)?;
    }
    Ok(models)
}

pub fn delete_force_model(
    db: &Database,
    app_type: &str,
    model: &str,
) -> Result<Vec<String>, AppError> {
    let model = normalize_force_model_option(model)?;
    let mut models = list_force_models(db, app_type)?;
    models.retain(|item| item != &model);
    save_force_models(db, app_type, &models)?;
    Ok(models)
}

pub fn list_routes(db: &Database) -> Result<Vec<ProjectProviderRoute>, AppError> {
    db.list_project_provider_routes()
}

pub fn upsert_route(
    db: &Database,
    project_path: &str,
    app_type: &str,
    provider_id: &str,
) -> Result<ProjectProviderRoute, AppError> {
    let path_key = normalize_project_path(project_path)
        .ok_or_else(|| AppError::InvalidInput("项目目录不能为空".to_string()))?;
    if app_type.trim().is_empty() || provider_id.trim().is_empty() {
        return Err(AppError::InvalidInput("客户端和供应商不能为空".to_string()));
    }
    db.upsert_project_provider_route(&path_key, project_path.trim(), app_type, provider_id.trim())
}

pub fn set_force_model(
    db: &Database,
    project_path: &str,
    app_type: &str,
    enabled: bool,
    force_model: Option<&str>,
) -> Result<ProjectProviderRoute, AppError> {
    let path_key = normalize_project_path(project_path)
        .ok_or_else(|| AppError::InvalidInput("项目目录不能为空".to_string()))?;
    let force_model = match force_model {
        Some(model) if !model.trim().is_empty() => Some(normalize_force_model_option(model)?),
        _ => None,
    };
    if enabled && force_model.is_none() {
        return Err(AppError::InvalidInput(
            "开启强制路由模型前请先选择模型".to_string(),
        ));
    }
    db.set_project_force_model_route(&path_key, app_type, enabled, force_model.as_deref())
}

pub fn set_session_route(
    db: &Database,
    project_path: &str,
    app_type: &str,
    session_id: &str,
    provider_id: Option<&str>,
    force_model: Option<&str>,
) -> Result<Option<SessionProviderRoute>, AppError> {
    let path_key = normalize_project_path(project_path)
        .ok_or_else(|| AppError::InvalidInput("项目目录不能为空".to_string()))?;
    let session_id = normalize_session_id(app_type, session_id);
    if session_id.is_empty() {
        return Err(AppError::InvalidInput("会话 ID 不能为空".to_string()));
    }
    if session_id.len() > 500 || session_id.contains(['\r', '\n']) {
        return Err(AppError::InvalidInput("会话 ID 格式不正确".to_string()));
    }

    let provider_id = provider_id.map(str::trim).filter(|value| !value.is_empty());
    if let Some(provider_id) = provider_id {
        if db
            .get_provider_by_id(provider_id, app_type)
            .map_err(|e| AppError::Database(e.to_string()))?
            .is_none()
        {
            return Err(AppError::InvalidInput(format!(
                "供应商不存在: {provider_id}"
            )));
        }
    }

    let force_model = match force_model {
        Some(model) if !model.trim().is_empty() => Some(normalize_force_model_option(model)?),
        _ => None,
    };

    db.upsert_session_provider_route(
        app_type,
        &session_id,
        &path_key,
        provider_id,
        force_model.as_deref(),
    )
}

pub fn clear_session_route(
    db: &Database,
    app_type: &str,
    session_id: &str,
) -> Result<bool, AppError> {
    let session_id = normalize_session_id(app_type, session_id);
    if session_id.is_empty() {
        return Err(AppError::InvalidInput("会话 ID 不能为空".to_string()));
    }
    db.delete_session_provider_route(app_type, &session_id)
}

pub fn list_session_routes(db: &Database) -> Result<Vec<SessionProviderRoute>, AppError> {
    db.list_session_provider_routes()
}

pub fn delete_route(db: &Database, project_path: &str, app_type: &str) -> Result<bool, AppError> {
    let path_key = normalize_project_path(project_path)
        .ok_or_else(|| AppError::InvalidInput("项目目录不能为空".to_string()))?;
    db.delete_project_provider_route(&path_key, app_type)
}

pub fn delete_routes_by_app_type(db: &Database, app_type: &str) -> Result<usize, AppError> {
    force_model_options_setting_key(app_type)?;
    db.delete_project_provider_routes_by_app_type(app_type)
}

#[cfg(test)]
pub fn resolve_route_override(
    db: &Database,
    app_type: &str,
    session_id: &str,
) -> Result<Option<RouteOverride>, AppError> {
    resolve_route_override_with_workspaces(db, app_type, session_id, &[])
}

/// Resolve session/project routing using fresh workspace roots from the request
/// before falling back to local session-history discovery.
pub fn resolve_route_override_with_workspaces(
    db: &Database,
    app_type: &str,
    session_id: &str,
    workspace_paths: &[String],
) -> Result<Option<RouteOverride>, AppError> {
    if !matches!(app_type, "codex" | "claude") {
        return Ok(None);
    }

    let session_id = normalize_session_id(app_type, session_id);
    let mut provider_id = None;
    let mut force_model = None;
    let mut session_project_path_key = None;

    if !session_id.is_empty() {
        if let Some(session_route) = db.get_session_provider_route(app_type, &session_id)? {
            if let Some(candidate) = session_route
                .provider_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                if db.get_provider_by_id(candidate, app_type)?.is_some() {
                    provider_id = Some(candidate.to_string());
                }
            }
            force_model = session_route
                .force_model
                .map(|model| model.trim().to_string())
                .filter(|model| !model.is_empty());
            session_project_path_key = Some(session_route.project_path_key);
        }
    }

    // A session may override only the provider or only the model. Fill any
    // missing part from the project route, then let the caller fall back to the
    // global/provider default for whatever remains absent.
    if provider_id.is_none() || force_model.is_none() {
        let path_key = if session_project_path_key.is_some() {
            session_project_path_key
        } else if let Some(path_key) =
            find_project_path_for_workspaces(db, app_type, workspace_paths)?
        {
            Some(path_key)
        } else {
            lookup_session_project_dir(app_type, &session_id)
                .and_then(|project_dir| normalize_project_path(&project_dir))
        };

        if let Some(path_key) = path_key.as_deref() {
            if let Some(route) = db
                .get_project_provider_route(path_key, app_type)?
                .filter(|route| route.enabled)
            {
                if provider_id.is_none()
                    && db
                        .get_provider_by_id(&route.provider_id, app_type)?
                        .is_some()
                {
                    provider_id = Some(route.provider_id);
                }
                if force_model.is_none() {
                    force_model = route
                        .force_model_enabled
                        .then_some(route.force_model)
                        .flatten()
                        .map(|model| model.trim().to_string())
                        .filter(|model| !model.is_empty());
                }
            }
        }
    }

    if provider_id.is_none() && force_model.is_none() {
        return Ok(None);
    }

    Ok(Some(RouteOverride {
        provider_id,
        force_model,
    }))
}

fn find_project_path_for_workspaces(
    db: &Database,
    app_type: &str,
    workspace_paths: &[String],
) -> Result<Option<String>, AppError> {
    if workspace_paths.is_empty() {
        return Ok(None);
    }

    let routes = db.list_project_provider_routes()?;
    let mut best_match: Option<(usize, String)> = None;

    for workspace_path in workspace_paths {
        let Some(workspace_key) = normalize_project_path(workspace_path) else {
            continue;
        };

        for route in routes
            .iter()
            .filter(|route| route.enabled && route.app_type == app_type)
        {
            let route_key = route.project_path_key.trim_end_matches('/');
            if route_key.is_empty() {
                continue;
            }

            let is_match = workspace_key == route_key
                || workspace_key
                    .strip_prefix(route_key)
                    .is_some_and(|suffix| suffix.starts_with('/'));
            if !is_match {
                continue;
            }

            let is_better = best_match
                .as_ref()
                .map_or(true, |(best_len, _)| route_key.len() > *best_len);
            if is_better {
                best_match = Some((route_key.len(), route.project_path_key.clone()));
            }
        }
    }

    Ok(best_match.map(|(_, path_key)| path_key))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(
        provider_id: &str,
        session_id: &str,
        project_dir: Option<&str>,
        last_active_at: i64,
    ) -> SessionMeta {
        SessionMeta {
            provider_id: provider_id.to_string(),
            session_id: session_id.to_string(),
            title: None,
            summary: None,
            project_dir: project_dir.map(str::to_string),
            created_at: Some(last_active_at),
            last_active_at: Some(last_active_at),
            last_model: None,
            source_path: None,
            resume_command: None,
        }
    }

    #[test]
    fn normalizes_slashes_and_trailing_separator() {
        let normalized = normalize_project_path("./foo/bar/").unwrap();
        assert!(!normalized.ends_with('/'));
        assert!(normalized.contains("foo/bar"));
    }

    #[test]
    fn groups_sessions_from_multiple_clients_by_project_directory() {
        let root = std::env::current_dir()
            .unwrap()
            .join("project-manager-test");
        let root_text = root.to_string_lossy().to_string();
        let alternate = format!("{}/", root_text.replace('\\', "/"));
        let groups = group_sessions_by_project(vec![
            session("codex", "codex-session", Some(&root_text), 10),
            session("claude", "claude-session", Some(&alternate), 20),
            session("codex", "other-session", Some("./another-project"), 5),
            session("gemini", "no-project", None, 30),
        ]);

        assert_eq!(groups.len(), 2);
        let project = groups
            .iter()
            .find(|group| group.path_key == normalize_project_path(&root_text).unwrap())
            .unwrap();
        assert_eq!(project.sessions.len(), 2);
        assert_eq!(project.sessions[0].session_id, "claude-session");
    }
    #[test]
    fn force_model_options_are_scoped_and_legacy_is_copied() {
        let db = Database::memory().expect("create memory db");
        db.set_setting(
            LEGACY_FORCE_MODEL_OPTIONS_SETTING_KEY,
            r#"["legacy-model", "legacy-model"]"#,
        )
        .unwrap();

        assert_eq!(
            list_force_models(&db, "codex").unwrap(),
            vec!["legacy-model".to_string()]
        );
        assert_eq!(
            add_force_model(&db, "codex", " gpt-5 ").unwrap(),
            vec!["legacy-model".to_string(), "gpt-5".to_string()]
        );
        assert_eq!(
            list_force_models(&db, "claude").unwrap(),
            vec!["legacy-model".to_string()]
        );
        assert_eq!(
            add_force_model(&db, "claude", "claude-sonnet").unwrap(),
            vec!["legacy-model".to_string(), "claude-sonnet".to_string()]
        );
        assert_eq!(
            delete_force_model(&db, "codex", "legacy-model").unwrap(),
            vec!["gpt-5".to_string()]
        );
        assert_eq!(
            list_force_models(&db, "claude").unwrap(),
            vec!["legacy-model".to_string(), "claude-sonnet".to_string()]
        );
    }

    #[test]
    fn normalizes_extended_codex_cwd_for_project_lookup() {
        assert_eq!(
            normalize_project_path(r"\\?\D:\AiCli\conversation\测活").unwrap(),
            "//?/d:/aicli/conversation/测活"
        );
    }

    #[test]
    fn normalizes_codex_proxy_session_prefix() {
        assert_eq!(normalize_session_id("codex", "codex_abc-123"), "abc-123");
        assert_eq!(normalize_session_id("codex", "abc-123"), "abc-123");
        assert_eq!(normalize_session_id("claude", "session-abc"), "session-abc");
    }

    #[test]
    fn changing_project_provider_resets_session_routes() {
        let db = Database::memory().expect("create memory db");
        db.upsert_project_provider_route(
            "c:/workspace/demo",
            "C:/workspace/demo",
            "codex",
            "provider-a",
        )
        .unwrap();
        db.upsert_session_provider_route(
            "codex",
            "session-1",
            "c:/workspace/demo",
            Some("provider-a"),
            Some("gpt-5.6"),
        )
        .unwrap();

        db.upsert_project_provider_route(
            "c:/workspace/demo",
            "C:/workspace/demo",
            "codex",
            "provider-b",
        )
        .unwrap();

        assert!(db
            .get_session_provider_route("codex", "session-1")
            .unwrap()
            .is_none());
    }

    #[test]
    fn changing_project_force_model_resets_session_routes() {
        let db = Database::memory().expect("create memory db");
        db.upsert_project_provider_route(
            "c:/workspace/demo",
            "C:/workspace/demo",
            "codex",
            "provider-a",
        )
        .unwrap();
        db.upsert_session_provider_route(
            "codex",
            "session-1",
            "c:/workspace/demo",
            Some("provider-a"),
            None,
        )
        .unwrap();

        db.set_project_force_model_route("c:/workspace/demo", "codex", true, Some("gpt-5.6"))
            .unwrap();

        assert!(db
            .get_session_provider_route("codex", "session-1")
            .unwrap()
            .is_none());
    }

    #[test]
    fn resolved_session_route_has_priority_over_project_route() {
        use crate::provider::Provider;

        let db = Database::memory().expect("create memory db");
        db.save_provider(
            "codex",
            &Provider::with_id(
                "session-provider".to_string(),
                "Session Provider".to_string(),
                serde_json::json!({}),
                None,
            ),
        )
        .unwrap();
        db.upsert_project_provider_route(
            "c:/workspace/demo",
            "C:/workspace/demo",
            "codex",
            "project-provider",
        )
        .unwrap();
        db.upsert_session_provider_route(
            "codex",
            "session-1",
            "c:/workspace/demo",
            Some("session-provider"),
            Some("gpt-5.6"),
        )
        .unwrap();

        let resolved = resolve_route_override(&db, "codex", "codex_session-1")
            .unwrap()
            .expect("session route should resolve");

        assert_eq!(resolved.provider_id.as_deref(), Some("session-provider"));
        assert_eq!(resolved.force_model.as_deref(), Some("gpt-5.6"));
    }

    #[test]
    fn session_model_override_inherits_project_provider_without_history_lookup() {
        use crate::provider::Provider;

        let db = Database::memory().expect("create memory db");
        db.save_provider(
            "codex",
            &Provider::with_id(
                "project-provider".to_string(),
                "Project Provider".to_string(),
                serde_json::json!({}),
                None,
            ),
        )
        .unwrap();
        db.upsert_project_provider_route(
            "c:/workspace/demo",
            "C:/workspace/demo",
            "codex",
            "project-provider",
        )
        .unwrap();
        db.upsert_session_provider_route(
            "codex",
            "session-1",
            "c:/workspace/demo",
            None,
            Some("gpt-5.6"),
        )
        .unwrap();

        let resolved = resolve_route_override(&db, "codex", "session-1")
            .unwrap()
            .expect("session model override should resolve");

        assert_eq!(resolved.provider_id.as_deref(), Some("project-provider"));
        assert_eq!(resolved.force_model.as_deref(), Some("gpt-5.6"));
    }

    #[test]
    fn workspace_path_resolves_project_route_without_history_lookup() {
        use crate::provider::Provider;

        let db = Database::memory().expect("create memory db");
        db.save_provider(
            "codex",
            &Provider::with_id(
                "project-provider".to_string(),
                "Project Provider".to_string(),
                serde_json::json!({}),
                None,
            ),
        )
        .unwrap();

        let workspace = std::env::current_dir()
            .unwrap()
            .join("workspace-route-test")
            .join("subdir");
        let workspace_text = workspace.to_string_lossy().to_string();
        let project_root = workspace.parent().unwrap().to_string_lossy().to_string();
        upsert_route(&db, &project_root, "codex", "project-provider").unwrap();
        set_force_model(&db, &project_root, "codex", true, Some("deepseek-v4-flash")).unwrap();

        let resolved = resolve_route_override_with_workspaces(
            &db,
            "codex",
            "codex_brand-new-session",
            &[workspace_text],
        )
        .unwrap()
        .expect("workspace path should resolve project route");

        assert_eq!(resolved.provider_id.as_deref(), Some("project-provider"));
        assert_eq!(resolved.force_model.as_deref(), Some("deepseek-v4-flash"));
    }

    #[test]
    fn resets_only_routes_for_the_selected_client() {
        let db = Database::memory().expect("create memory db");
        upsert_route(&db, "C:/workspace/demo", "codex", "codex-provider").unwrap();
        upsert_route(&db, "C:/workspace/demo", "claude", "claude-provider").unwrap();

        assert_eq!(delete_routes_by_app_type(&db, "codex").unwrap(), 1);
        assert!(db
            .get_project_provider_route("c:/workspace/demo", "codex")
            .unwrap()
            .is_none());
        assert!(db
            .get_project_provider_route("c:/workspace/demo", "claude")
            .unwrap()
            .is_some());
    }
}
