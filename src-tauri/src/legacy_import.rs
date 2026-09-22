//! Compatibility guard and one-time, copy-only import from the official CC
//! Switch data directory.
//!
//! PPBind must never migrate or modify the official application's database in
//! place. This module is intentionally run before PPBind opens its own database:
//! if the user opts in, the official SQLite database is copied through SQLite's
//! online-backup API into a temporary PPBind database, validated, and then
//! atomically moved into place.

use crate::config::{get_app_config_dir, get_app_db_path, get_legacy_app_db_path};
use crate::error::AppError;
use crate::proxy::types::DEFAULT_PROXY_PORT;
use rusqlite::{backup::Backup, Connection, OpenFlags};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

const OFFICIAL_PROCESS_NAME: &str = "cc-switch.exe";
const AUXILIARY_FILES: &[&str] = &["settings.json", "model-pricing.json"];

/// Prevent PPBind from starting while the official CC Switch is running.
///
/// Both applications manage the same live Codex/Claude configuration and
/// proxy settings. Running them together can cause port conflicts or requests
/// to be routed through the wrong application.
pub fn ensure_official_app_not_running(app: &AppHandle) -> Result<(), AppError> {
    if !is_official_app_running()? {
        return Ok(());
    }

    let title = localized("PPBind 无法启动", "PPBind Cannot Start");
    let message = localized(
        "检测到官方 CC Switch 正在运行。\n\nPPBind 与官方 CC Switch 会管理同一份 Codex / Claude 实时配置，同时运行可能导致代理端口冲突或请求路由异常。请先完全退出官方 CC Switch，然后重新打开 PPBind。\n\nPPBind 不会自动结束官方程序。",
        "Official CC Switch is running.\n\nPPBind and official CC Switch manage the same live Codex / Claude configuration. Running both apps can cause proxy port conflicts or incorrect request routing. Please exit official CC Switch completely, then open PPBind again.\n\nPPBind will not terminate the official application.",
    );
    app.dialog()
        .message(&message)
        .title(title)
        .kind(MessageDialogKind::Error)
        .buttons(MessageDialogButtons::Ok)
        .blocking_show();

    Err(AppError::Message(
        "官方 CC Switch 正在运行，已中止 PPBind 启动".to_string(),
    ))
}

/// Import official CC Switch data on PPBind's first launch.
///
/// The function is a no-op when PPBind already has a database or when no
/// official database exists. It never kills the official application.
pub fn run_if_needed(app: &AppHandle) -> Result<(), AppError> {
    let destination_db = get_app_db_path();
    if destination_db.exists() {
        return Ok(());
    }

    let legacy_db = get_legacy_app_db_path();
    if !legacy_db.is_file() {
        return Ok(());
    }

    let title = localized("导入官方 CC Switch 数据", "Import Official CC Switch Data");
    let message = localized(
        "PPBind 检测到官方 CC Switch 数据。\n\n是否复制一份到 PPBind？\n\n复制是只读操作，不会修改或迁移官方数据库。选择“否”将使用全新的 PPBind 数据目录。",
        "PPBind found official CC Switch data.\n\nCopy it into PPBind?\n\nThis is copy-only and will not modify or migrate the official database. Choose No to start with fresh PPBind data.",
    );
    let should_import = app
        .dialog()
        .message(&message)
        .title(title)
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::YesNo)
        .blocking_show();

    if !should_import {
        return Ok(());
    }

    let app_config_dir = get_app_config_dir();
    if let Err(error) = import_legacy_data(&legacy_db, &destination_db, &app_config_dir) {
        eprintln!("PPBind first-run import failed: {error}");
        app.dialog()
            .message(&localized(
                "导入官方 CC Switch 数据失败。\n\nPPBind 尚未创建自己的数据库，也没有修改官方数据。请确认官方 CC Switch 已退出后重试。",
                "Failed to import official CC Switch data.\n\nPPBind did not create its own database and did not modify the official data. Please make sure official CC Switch is closed and try again.",
            ))
            .title(localized("导入失败", "Import Failed"))
            .kind(MessageDialogKind::Error)
            .buttons(MessageDialogButtons::Ok)
            .blocking_show();
        return Err(error);
    }

    Ok(())
}

/// Copy the official database and optional auxiliary data without overwriting
/// anything in the official directory.
pub(crate) fn import_legacy_data(
    legacy_db: &Path,
    destination_db: &Path,
    destination_config_dir: &Path,
) -> Result<(), AppError> {
    fs::create_dir_all(destination_config_dir)
        .map_err(|error| AppError::io(destination_config_dir, error))?;

    copy_database_snapshot(legacy_db, destination_db)?;

    let legacy_config_dir = legacy_db.parent().ok_or_else(|| {
        AppError::Config(format!(
            "无法确定官方数据库所在目录: {}",
            legacy_db.display()
        ))
    })?;

    for warning in copy_auxiliary_data(legacy_config_dir, destination_config_dir) {
        eprintln!("PPBind import warning: {warning}");
    }

    Ok(())
}

/// Use SQLite online backup so committed WAL contents are included. A raw file
/// copy would be unsafe and could produce a torn snapshot.
pub(crate) fn copy_database_snapshot(
    source_path: &Path,
    destination_path: &Path,
) -> Result<(), AppError> {
    if destination_path.exists() {
        return Ok(());
    }

    let parent = destination_path.parent().ok_or_else(|| {
        AppError::Config(format!(
            "无法确定 PPBind 数据库目录: {}",
            destination_path.display()
        ))
    })?;
    fs::create_dir_all(parent).map_err(|error| AppError::io(parent, error))?;

    let temp_path = temporary_import_path(destination_path);
    if temp_path.exists() {
        fs::remove_file(&temp_path).map_err(|error| AppError::io(&temp_path, error))?;
    }

    let result = (|| {
        let source = Connection::open_with_flags(source_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| AppError::Database(error.to_string()))?;
        let mut destination =
            Connection::open(&temp_path).map_err(|error| AppError::Database(error.to_string()))?;

        {
            let backup = Backup::new(&source, &mut destination)
                .map_err(|error| AppError::Database(error.to_string()))?;
            backup
                .run_to_completion(5, Duration::from_millis(25), None)
                .map_err(|error| AppError::Database(error.to_string()))?;
        }

        drop(destination);
        drop(source);

        validate_database_snapshot(&temp_path)?;

        normalize_imported_proxy_port(&temp_path)?;
        if destination_path.exists() {
            return Ok(());
        }

        fs::rename(&temp_path, destination_path)
            .map_err(|error| AppError::io(destination_path, error))?;
        Ok(())
    })();

    if result.is_err() && temp_path.exists() {
        let _ = fs::remove_file(&temp_path);
    }

    result
}

fn validate_database_snapshot(path: &Path) -> Result<(), AppError> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| AppError::Database(error.to_string()))?;

    let check: String = connection
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|error| AppError::Database(error.to_string()))?;
    if check != "ok" {
        return Err(AppError::Database(format!(
            "导入的数据库完整性检查失败: {check}"
        )));
    }

    let version: i32 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| AppError::Database(error.to_string()))?;
    if version <= 0 {
        return Err(AppError::Database(
            "官方数据库缺少有效的 schema 版本，已取消导入".to_string(),
        ));
    }

    Ok(())
}

/// Keep PPBind independent from the official CC Switch proxy listener.
///
/// Official databases normally contain port 15721. Import is copy-only, so only
/// the PPBind snapshot is adjusted before it becomes the live database.
fn normalize_imported_proxy_port(path: &Path) -> Result<(), AppError> {
    let connection =
        Connection::open(path).map_err(|error| AppError::Database(error.to_string()))?;
    let has_proxy_table: bool = connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_master
                WHERE type = 'table' AND name = 'proxy_config'
            )",
            [],
            |row| row.get(0),
        )
        .map_err(|error| AppError::Database(error.to_string()))?;

    if has_proxy_table {
        connection
            .execute(
                "UPDATE proxy_config SET listen_port = ?1 WHERE listen_port = 15721",
                [DEFAULT_PROXY_PORT],
            )
            .map_err(|error| AppError::Database(error.to_string()))?;
    }

    Ok(())
}
fn temporary_import_path(destination_path: &Path) -> PathBuf {
    destination_path.with_extension("db.importing")
}

fn copy_auxiliary_data(source_dir: &Path, destination_dir: &Path) -> Vec<String> {
    let mut warnings = Vec::new();

    for name in AUXILIARY_FILES {
        let source = source_dir.join(name);
        let destination = destination_dir.join(name);
        if let Err(error) = copy_file_if_absent(&source, &destination) {
            warnings.push(error);
        }
    }

    let source_skills = source_dir.join("skills");
    let destination_skills = destination_dir.join("skills");
    if source_skills.is_dir() {
        if let Err(error) = copy_tree_if_absent(&source_skills, &destination_skills) {
            warnings.push(format!(
                "复制 skills 目录失败 ({} -> {}): {}",
                source_skills.display(),
                destination_skills.display(),
                error
            ));
        }
    }

    warnings
}

fn copy_file_if_absent(source: &Path, destination: &Path) -> Result<(), String> {
    if !source.is_file() || destination.exists() {
        return Ok(());
    }

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建目录 {} 失败: {error}", parent.display()))?;
    }

    fs::copy(source, destination).map(|_| ()).map_err(|error| {
        format!(
            "复制文件失败 ({} -> {}): {error}",
            source.display(),
            destination.display()
        )
    })
}

fn copy_tree_if_absent(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|error| format!("创建目录 {} 失败: {error}", destination.display()))?;

    let entries = fs::read_dir(source)
        .map_err(|error| format!("读取目录 {} 失败: {error}", source.display()))?;

    for entry in entries {
        let entry = entry.map_err(|error| format!("读取目录项失败: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("读取类型失败 ({}): {error}", entry.path().display()))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());

        // Skip links to avoid following paths outside the official data folder.
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            copy_tree_if_absent(&source_path, &destination_path)?;
        } else if file_type.is_file() && !destination_path.exists() {
            copy_file_if_absent(&source_path, &destination_path)?;
        }
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn is_official_app_running() -> Result<bool, AppError> {
    let output = std::process::Command::new("tasklist")
        .args([
            "/FI",
            &format!("IMAGENAME eq {OFFICIAL_PROCESS_NAME}"),
            "/FO",
            "CSV",
            "/NH",
        ])
        .output()
        .map_err(|error| AppError::Message(format!("检查官方 CC Switch 进程失败: {error}")))?;

    Ok(tasklist_output_contains_process(
        &String::from_utf8_lossy(&output.stdout),
        OFFICIAL_PROCESS_NAME,
    ))
}

#[cfg(target_os = "windows")]
fn tasklist_output_contains_process(output: &str, process_name: &str) -> bool {
    let expected = format!("\"{process_name}\",");
    output.lines().any(|line| {
        line.trim_start()
            .to_ascii_lowercase()
            .starts_with(&expected.to_ascii_lowercase())
    })
}

#[cfg(not(target_os = "windows"))]
fn is_official_app_running() -> Result<bool, AppError> {
    let output = std::process::Command::new("pgrep")
        .args(["-f", "CC Switch.app/Contents/MacOS/CC Switch"])
        .output();
    match output {
        Ok(output) => Ok(output.status.success()),
        Err(_) => Ok(false),
    }
}

fn localized(zh: &'static str, en: &'static str) -> String {
    let locale = sys_locale::get_locale().unwrap_or_default();
    if locale.to_ascii_lowercase().starts_with("zh") {
        zh.to_string()
    } else {
        en.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_database(path: &Path, marker: &str) {
        let connection = Connection::open(path).expect("open database");
        connection
            .execute_batch(&format!(
                "PRAGMA user_version = 21; CREATE TABLE marker(value TEXT); INSERT INTO marker VALUES ('{marker}');"
            ))
            .expect("create database");
    }

    #[test]
    fn database_import_copies_and_leaves_source_unchanged() {
        let temp = TempDir::new().expect("tempdir");
        let source = temp.path().join("cc-switch.db");
        let destination = temp.path().join("ppbind").join("ppbind.db");
        create_database(&source, "official");
        let before = fs::read(&source).expect("read source before import");

        copy_database_snapshot(&source, &destination).expect("import database");

        assert_eq!(fs::read(&source).expect("read source after import"), before);
        let connection = Connection::open(&destination).expect("open imported database");
        let marker: String = connection
            .query_row("SELECT value FROM marker", [], |row| row.get(0))
            .expect("read marker");
        assert_eq!(marker, "official");
    }

    #[test]
    fn database_import_moves_official_proxy_port_only_in_snapshot() {
        let temp = TempDir::new().expect("tempdir");
        let source = temp.path().join("cc-switch.db");
        let destination = temp.path().join("ppbind").join("ppbind.db");
        create_database(&source, "official");
        {
            let connection = Connection::open(&source).expect("open source");
            connection
                .execute_batch(
                    "CREATE TABLE proxy_config(
                        app_type TEXT PRIMARY KEY,
                        listen_port INTEGER NOT NULL
                    );
                    INSERT INTO proxy_config(app_type, listen_port)
                    VALUES ('claude', 15721), ('codex', 15721);",
                )
                .expect("create proxy config");
        }

        copy_database_snapshot(&source, &destination).expect("import database");

        let source_connection = Connection::open(&source).expect("open source");
        let source_port: u16 = source_connection
            .query_row(
                "SELECT listen_port FROM proxy_config WHERE app_type = 'claude'",
                [],
                |row| row.get(0),
            )
            .expect("read source port");
        assert_eq!(source_port, 15721);

        let destination_connection = Connection::open(&destination).expect("open destination");
        let destination_port: u16 = destination_connection
            .query_row(
                "SELECT listen_port FROM proxy_config WHERE app_type = 'claude'",
                [],
                |row| row.get(0),
            )
            .expect("read destination port");
        assert_eq!(destination_port, DEFAULT_PROXY_PORT);
    }
    #[test]
    fn database_import_is_noop_when_destination_exists() {
        let temp = TempDir::new().expect("tempdir");
        let source = temp.path().join("cc-switch.db");
        let destination = temp.path().join("ppbind.db");
        create_database(&source, "official");
        create_database(&destination, "ppbind");

        copy_database_snapshot(&source, &destination).expect("noop import");

        let connection = Connection::open(&destination).expect("open destination");
        let marker: String = connection
            .query_row("SELECT value FROM marker", [], |row| row.get(0))
            .expect("read marker");
        assert_eq!(marker, "ppbind");
    }

    #[test]
    fn auxiliary_copy_skips_excluded_and_existing_files() {
        let temp = TempDir::new().expect("tempdir");
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::create_dir_all(source.join("skills").join("demo")).expect("create source");
        fs::create_dir_all(source.join("logs")).expect("create logs");
        fs::create_dir_all(&destination).expect("create destination");

        fs::write(source.join("settings.json"), "official").expect("write settings");
        fs::write(source.join("cc-switch.db"), "db").expect("write db");
        fs::write(source.join("logs").join("app.log"), "log").expect("write log");
        fs::write(source.join("skills").join("demo").join("SKILL.md"), "skill")
            .expect("write skill");
        fs::write(destination.join("settings.json"), "ppbind").expect("write existing settings");

        let warnings = copy_auxiliary_data(&source, &destination);

        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert_eq!(
            fs::read_to_string(destination.join("settings.json")).expect("read settings"),
            "ppbind"
        );
        assert!(destination
            .join("skills")
            .join("demo")
            .join("SKILL.md")
            .is_file());
        assert!(!destination.join("cc-switch.db").exists());
        assert!(!destination.join("logs").exists());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn tasklist_parser_matches_only_exact_process_name() {
        let output = "\"cc-switch.exe\",\"1234\",\"Console\",\"1\",\"12,345 K\"\r\n";
        assert!(tasklist_output_contains_process(
            output,
            OFFICIAL_PROCESS_NAME
        ));
        assert!(!tasklist_output_contains_process(
            "\"other.exe\",\"1234\",\"Console\",\"1\",\"12,345 K\"\r\n",
            OFFICIAL_PROCESS_NAME
        ));
    }
}
