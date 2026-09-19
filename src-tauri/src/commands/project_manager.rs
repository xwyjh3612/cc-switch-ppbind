#![allow(non_snake_case)]

use crate::database::ProjectProviderRoute;
use crate::project_manager::{self, ProjectGroup};
use crate::store::AppState;
use serde::Serialize;
use tauri::State;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDto {
    pub path_key: String,
    pub project_path: String,
    pub sessions: Vec<crate::session_manager::SessionMeta>,
    pub routes: Vec<ProjectProviderRoute>,
}

fn project_dto(group: ProjectGroup, routes: Vec<ProjectProviderRoute>) -> ProjectDto {
    ProjectDto {
        path_key: group.path_key,
        project_path: group.project_path,
        sessions: group.sessions,
        routes,
    }
}

#[tauri::command]
pub async fn list_projects(state: State<'_, AppState>) -> Result<Vec<ProjectDto>, String> {
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let sessions = crate::session_manager::scan_sessions();
        let groups = project_manager::group_sessions_by_project(sessions);
        let routes = project_manager::list_routes(&db).map_err(|e| e.to_string())?;
        Ok(groups
            .into_iter()
            .map(|group| {
                let group_routes = routes
                    .iter()
                    .filter(|route| route.project_path_key == group.path_key)
                    .cloned()
                    .collect();
                project_dto(group, group_routes)
            })
            .collect())
    })
    .await
    .map_err(|e| format!("Failed to list projects: {e}"))?
}

#[tauri::command]
pub fn list_project_routes(
    state: State<'_, AppState>,
) -> Result<Vec<ProjectProviderRoute>, String> {
    project_manager::list_routes(&state.db).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_project_provider(
    state: State<'_, AppState>,
    projectPath: String,
    appType: String,
    providerId: String,
) -> Result<ProjectProviderRoute, String> {
    if !matches!(appType.as_str(), "codex" | "claude") {
        return Err(format!("不支持的项目客户端: {appType}"));
    }
    if providerId.trim().is_empty() {
        return Err("供应商 ID 不能为空".to_string());
    }

    let providers = state
        .db
        .get_provider_by_id(&providerId, &appType)
        .map_err(|e| e.to_string())?;
    if providers.is_none() {
        return Err(format!("供应商不存在: {providerId}"));
    }
    project_manager::upsert_route(&state.db, &projectPath, &appType, &providerId)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn clear_project_provider(
    state: State<'_, AppState>,
    projectPath: String,
    appType: String,
) -> Result<bool, String> {
    if !matches!(appType.as_str(), "codex" | "claude") {
        return Err(format!("不支持的项目客户端: {appType}"));
    }
    project_manager::delete_route(&state.db, &projectPath, &appType).map_err(|e| e.to_string())
}
