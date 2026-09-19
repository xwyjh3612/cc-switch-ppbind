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
    pub updated_at: i64,
}

impl Database {
    pub fn list_project_provider_routes(&self) -> Result<Vec<ProjectProviderRoute>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT project_path_key, project_path, app_type, provider_id, enabled, updated_at
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
                    updated_at: row.get(5)?,
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
                "SELECT project_path_key, project_path, app_type, provider_id, enabled, updated_at
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
                updated_at: row.get(5)?,
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
        let conn = lock_conn!(self.conn);
        conn.execute(
            "INSERT INTO project_provider_routes
             (project_path_key, project_path, app_type, provider_id, enabled, updated_at)
             VALUES (?1, ?2, ?3, ?4, 1, ?5)
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
        Ok(ProjectProviderRoute {
            project_path_key: project_path_key.to_string(),
            project_path: project_path.to_string(),
            app_type: app_type.to_string(),
            provider_id: provider_id.to_string(),
            enabled: true,
            updated_at,
        })
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
