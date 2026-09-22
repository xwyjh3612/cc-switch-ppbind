//! 会话级供应商与模型覆盖关系 DAO

use crate::database::{lock_conn, Database};
use crate::error::AppError;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionProviderRoute {
    pub app_type: String,
    pub session_id: String,
    pub project_path_key: String,
    pub provider_id: Option<String>,
    pub force_model: Option<String>,
    pub updated_at: i64,
}

impl Database {
    pub fn list_session_provider_routes(&self) -> Result<Vec<SessionProviderRoute>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT app_type, session_id, project_path_key, provider_id, force_model, updated_at
                 FROM session_provider_routes
                 ORDER BY project_path_key, app_type, session_id",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(SessionProviderRoute {
                    app_type: row.get(0)?,
                    session_id: row.get(1)?,
                    project_path_key: row.get(2)?,
                    provider_id: row.get(3)?,
                    force_model: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            })
            .map_err(|e| AppError::Database(e.to_string()))?;
        rows.map(|row| row.map_err(|e| AppError::Database(e.to_string())))
            .collect()
    }

    pub fn get_session_provider_route(
        &self,
        app_type: &str,
        session_id: &str,
    ) -> Result<Option<SessionProviderRoute>, AppError> {
        let conn = lock_conn!(self.conn);
        conn.query_row(
            "SELECT app_type, session_id, project_path_key, provider_id, force_model, updated_at
             FROM session_provider_routes
             WHERE app_type = ?1 AND session_id = ?2",
            params![app_type, session_id],
            |row| {
                Ok(SessionProviderRoute {
                    app_type: row.get(0)?,
                    session_id: row.get(1)?,
                    project_path_key: row.get(2)?,
                    provider_id: row.get(3)?,
                    force_model: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(|e| AppError::Database(e.to_string()))
    }

    pub fn upsert_session_provider_route(
        &self,
        app_type: &str,
        session_id: &str,
        project_path_key: &str,
        provider_id: Option<&str>,
        force_model: Option<&str>,
    ) -> Result<Option<SessionProviderRoute>, AppError> {
        let provider_id = provider_id.map(str::trim).filter(|value| !value.is_empty());
        let force_model = force_model.map(str::trim).filter(|value| !value.is_empty());
        if provider_id.is_none() && force_model.is_none() {
            self.delete_session_provider_route(app_type, session_id)?;
            return Ok(None);
        }

        let updated_at = chrono::Utc::now().timestamp_millis();
        let conn = lock_conn!(self.conn);
        conn.execute(
            "INSERT INTO session_provider_routes
             (app_type, session_id, project_path_key, provider_id, force_model, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(app_type, session_id) DO UPDATE SET
               project_path_key = excluded.project_path_key,
               provider_id = excluded.provider_id,
               force_model = excluded.force_model,
               updated_at = excluded.updated_at",
            params![
                app_type,
                session_id,
                project_path_key,
                provider_id,
                force_model,
                updated_at
            ],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        drop(conn);

        self.get_session_provider_route(app_type, session_id)
    }

    pub fn delete_session_provider_route(
        &self,
        app_type: &str,
        session_id: &str,
    ) -> Result<bool, AppError> {
        let conn = lock_conn!(self.conn);
        let affected = conn
            .execute(
                "DELETE FROM session_provider_routes WHERE app_type = ?1 AND session_id = ?2",
                params![app_type, session_id],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(affected > 0)
    }
}
