//! 项目目录与供应商覆盖关系 DAO

use crate::database::{lock_conn, Database};
use crate::error::AppError;
use rusqlite::params;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectProviderRoute {
    pub project_path_key: String,
    pub project_path: String,
    pub app_type: String,
    pub provider_id: String,
    pub enabled: bool,
    pub force_model_enabled: bool,
    pub force_model: Option<String>,
    pub updated_at: i64,
}

impl Database {
    pub fn list_project_provider_routes(&self) -> Result<Vec<ProjectProviderRoute>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT project_path_key, project_path, app_type, provider_id, enabled,
                        force_model_enabled, force_model, updated_at
                 FROM project_provider_routes
                 ORDER BY project_path_key, app_type",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ProjectProviderRoute {
                    project_path_key: row.get(0)?,
                    project_path: row.get(1)?,
                    app_type: row.get(2)?,
                    provider_id: row.get(3)?,
                    enabled: row.get::<_, i64>(4)? != 0,
                    force_model_enabled: row.get::<_, i64>(5)? != 0,
                    force_model: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })
            .map_err(|e| AppError::Database(e.to_string()))?;
        rows.map(|row| row.map_err(|e| AppError::Database(e.to_string())))
            .collect()
    }

    pub fn get_project_provider_route(
        &self,
        project_path_key: &str,
        app_type: &str,
    ) -> Result<Option<ProjectProviderRoute>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT project_path_key, project_path, app_type, provider_id, enabled,
                        force_model_enabled, force_model, updated_at
                 FROM project_provider_routes
                 WHERE project_path_key = ?1 AND app_type = ?2",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        match stmt.query_row(params![project_path_key, app_type], |row| {
            Ok(ProjectProviderRoute {
                project_path_key: row.get(0)?,
                project_path: row.get(1)?,
                app_type: row.get(2)?,
                provider_id: row.get(3)?,
                enabled: row.get::<_, i64>(4)? != 0,
                force_model_enabled: row.get::<_, i64>(5)? != 0,
                force_model: row.get(6)?,
                updated_at: row.get(7)?,
            })
        }) {
            Ok(route) => Ok(Some(route)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(AppError::Database(e.to_string())),
        }
    }

    pub fn upsert_project_provider_route(
        &self,
        project_path_key: &str,
        project_path: &str,
        app_type: &str,
        provider_id: &str,
    ) -> Result<ProjectProviderRoute, AppError> {
        let updated_at = chrono::Utc::now().timestamp_millis();
        {
            let conn = lock_conn!(self.conn);
            conn.execute(
                "INSERT INTO project_provider_routes
                 (project_path_key, project_path, app_type, provider_id, enabled,
                  force_model_enabled, force_model, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 1, 0, NULL, ?5)
                 ON CONFLICT(project_path_key, app_type) DO UPDATE SET
                   project_path = excluded.project_path,
                   provider_id = excluded.provider_id,
                   enabled = 1,
                   updated_at = excluded.updated_at",
                params![
                    project_path_key,
                    project_path,
                    app_type,
                    provider_id,
                    updated_at
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        }

        self.get_project_provider_route(project_path_key, app_type)?
            .ok_or_else(|| AppError::Database("项目供应商路由写入后读取失败".to_string()))
    }

    pub fn set_project_force_model_route(
        &self,
        project_path_key: &str,
        app_type: &str,
        force_model_enabled: bool,
        force_model: Option<&str>,
    ) -> Result<ProjectProviderRoute, AppError> {
        let updated_at = chrono::Utc::now().timestamp_millis();
        let force_model = force_model.map(str::trim).filter(|value| !value.is_empty());
        {
            let conn = lock_conn!(self.conn);
            let affected = conn
                .execute(
                    "UPDATE project_provider_routes
                     SET force_model_enabled = ?1,
                         force_model = ?2,
                         updated_at = ?3
                     WHERE project_path_key = ?4 AND app_type = ?5",
                    params![
                        if force_model_enabled { 1 } else { 0 },
                        force_model,
                        updated_at,
                        project_path_key,
                        app_type
                    ],
                )
                .map_err(|e| AppError::Database(e.to_string()))?;

            if affected == 0 {
                return Err(AppError::InvalidInput("请先为该项目选择供应商".to_string()));
            }
        }

        self.get_project_provider_route(project_path_key, app_type)?
            .ok_or_else(|| AppError::Database("项目强制模型写入后读取失败".to_string()))
    }

    pub fn delete_project_provider_route(
        &self,
        project_path_key: &str,
        app_type: &str,
    ) -> Result<bool, AppError> {
        let conn = lock_conn!(self.conn);
        let affected = conn
            .execute(
                "DELETE FROM project_provider_routes WHERE project_path_key = ?1 AND app_type = ?2",
                params![project_path_key, app_type],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(affected > 0)
    }
}
