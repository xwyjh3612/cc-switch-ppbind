//! Project-scoped provider routing and project path helpers.
//!
//! A project is identified by the normalized working directory discovered from a
//! session. The database stores only explicit provider overrides; all other
//! requests continue to use the existing global provider selection.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, RwLock};

use crate::database::{Database, ProjectProviderRoute};
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

static SESSION_PROJECT_CACHE: LazyLock<RwLock<HashMap<(String, String), String>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Resolve the project directory for a client-provided session ID.
///
/// This is intentionally best-effort. The first request of a new session may
/// arrive before its local history metadata is flushed; callers should fall
/// back to the global provider in that case.
pub fn lookup_session_project_dir(app_type: &str, session_id: &str) -> Option<String> {
    if session_id.trim().is_empty() || !matches!(app_type, "codex" | "claude") {
        return None;
    }

    let cache_key = (app_type.to_string(), session_id.to_string());
    if let Ok(cache) = SESSION_PROJECT_CACHE.read() {
        if let Some(project_dir) = cache.get(&cache_key) {
            return Some(project_dir.clone());
        }
    }

    let lookup_id = session_id.strip_prefix("codex_").unwrap_or(session_id);
    let project_dir = crate::session_manager::scan_sessions()
        .into_iter()
        .find(|session| session.provider_id == app_type && session.session_id == lookup_id)
        .and_then(|session| session.project_dir);
    if let Some(project_dir) = project_dir.as_ref() {
        if let Ok(mut cache) = SESSION_PROJECT_CACHE.write() {
            cache.insert(cache_key, project_dir.clone());
        }
    }
    project_dir
}

const FORCE_MODEL_OPTIONS_SETTING_KEY: &str = "project_force_model_options";

#[derive(Debug, Clone)]
pub struct ProjectRouteOverride {
    pub provider_id: String,
    pub force_model: Option<String>,
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

pub fn list_force_models(db: &Database) -> Result<Vec<String>, AppError> {
    let Some(raw) = db.get_setting(FORCE_MODEL_OPTIONS_SETTING_KEY)? else {
        return Ok(Vec::new());
    };
    let parsed = match serde_json::from_str::<Vec<String>>(&raw) {
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
    Ok(models)
}

fn save_force_models(db: &Database, models: &[String]) -> Result<(), AppError> {
    let json = serde_json::to_string(models)
        .map_err(|error| AppError::Database(format!("序列化强制模型列表失败: {error}")))?;
    db.set_setting(FORCE_MODEL_OPTIONS_SETTING_KEY, &json)
}

pub fn add_force_model(db: &Database, model: &str) -> Result<Vec<String>, AppError> {
    let model = normalize_force_model_option(model)?;
    let mut models = list_force_models(db)?;
    if !models.contains(&model) {
        models.push(model);
        save_force_models(db, &models)?;
    }
    Ok(models)
}

pub fn delete_force_model(db: &Database, model: &str) -> Result<Vec<String>, AppError> {
    let model = normalize_force_model_option(model)?;
    let mut models = list_force_models(db)?;
    models.retain(|item| item != &model);
    save_force_models(db, &models)?;
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

pub fn delete_route(db: &Database, project_path: &str, app_type: &str) -> Result<bool, AppError> {
    let path_key = normalize_project_path(project_path)
        .ok_or_else(|| AppError::InvalidInput("项目目录不能为空".to_string()))?;
    db.delete_project_provider_route(&path_key, app_type)
}

pub fn resolve_route_override(
    db: &Database,
    app_type: &str,
    session_id: &str,
) -> Result<Option<ProjectRouteOverride>, AppError> {
    let Some(project_dir) = lookup_session_project_dir(app_type, session_id) else {
        return Ok(None);
    };
    let Some(path_key) = normalize_project_path(&project_dir) else {
        return Ok(None);
    };
    let Some(route) = db
        .get_project_provider_route(&path_key, app_type)?
        .filter(|route| route.enabled)
    else {
        return Ok(None);
    };

    // A deleted provider must not make an otherwise valid project request fail;
    // stale overrides safely fall back to the existing global route.
    if db
        .get_provider_by_id(&route.provider_id, app_type)?
        .is_none()
    {
        return Ok(None);
    }

    let force_model = route
        .force_model_enabled
        .then_some(route.force_model)
        .flatten()
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty());

    Ok(Some(ProjectRouteOverride {
        provider_id: route.provider_id,
        force_model,
    }))
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
    fn force_model_options_are_global_and_deduplicated() {
        let db = Database::memory().expect("create memory db");

        assert_eq!(list_force_models(&db).unwrap(), Vec::<String>::new());
        assert_eq!(
            add_force_model(&db, " gpt-5 ").unwrap(),
            vec!["gpt-5".to_string()]
        );
        assert_eq!(
            add_force_model(&db, "gpt-5").unwrap(),
            vec!["gpt-5".to_string()]
        );
        assert_eq!(
            add_force_model(&db, "claude-sonnet").unwrap(),
            vec!["gpt-5".to_string(), "claude-sonnet".to_string()]
        );
        assert_eq!(
            delete_force_model(&db, "gpt-5").unwrap(),
            vec!["claude-sonnet".to_string()]
        );
    }
}
