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
use rusqlite::{
    backup::{Backup, StepResult},
    Connection, OpenFlags,
};
use std::fs;
#[cfg(target_os = "windows")]
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc,
};
use std::time::Duration;
use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
const OFFICIAL_PROCESS_NAME: &str = "cc-switch.exe";
const AUXILIARY_FILES: &[&str] = &["settings.json", "model-pricing.json"];
const IMPORT_CANCELLED_MESSAGE: &str = "PPBind 数据导入已取消";
const BACKUP_PAGES_PER_STEP: i32 = 1024;
const BACKUP_BUSY_RETRY_DELAY: Duration = Duration::from_millis(10);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
enum ImportStage {
    Preparing = 0,
    CopyingDatabase = 1,
    CheckingDatabase = 2,
    FinalizingDatabase = 3,
    ScanningFiles = 4,
    CopyingFiles = 5,
    Finished = 6,
}
impl ImportStage {
    fn from_code(code: u32) -> Self {
        match code {
            1 => Self::CopyingDatabase,
            2 => Self::CheckingDatabase,
            3 => Self::FinalizingDatabase,
            4 => Self::ScanningFiles,
            5 => Self::CopyingFiles,
            6 => Self::Finished,
            _ => Self::Preparing,
        }
    }
    fn label(self, zh: bool) -> &'static str {
        match (self, zh) {
            (Self::Preparing, true) => "正在准备导入",
            (Self::Preparing, false) => "Preparing import",
            (Self::CopyingDatabase, true) => "正在复制官方数据库",
            (Self::CopyingDatabase, false) => "Copying official database",
            (Self::CheckingDatabase, true) => "正在检查数据库完整性",
            (Self::CheckingDatabase, false) => "Checking database integrity",
            (Self::FinalizingDatabase, true) => "正在完成数据导入",
            (Self::FinalizingDatabase, false) => "Finalizing import",
            (Self::ScanningFiles, true) => "正在统计设置和技能文件",
            (Self::ScanningFiles, false) => "Scanning settings and skills",
            (Self::CopyingFiles, true) => "正在复制设置和技能文件",
            (Self::CopyingFiles, false) => "Copying settings and skills",
            (Self::Finished, true) => "导入完成",
            (Self::Finished, false) => "Import complete",
        }
    }
}
#[derive(Clone)]
struct ImportProgress {
    percent: Arc<AtomicU32>,
    stage: Arc<AtomicU32>,
    cancelled: Arc<AtomicBool>,
}
impl Default for ImportProgress {
    fn default() -> Self {
        Self {
            percent: Arc::new(AtomicU32::new(0)),
            stage: Arc::new(AtomicU32::new(ImportStage::Preparing as u32)),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}
impl ImportProgress {
    fn report(&self, percent: u32, stage: ImportStage) -> Result<(), AppError> {
        self.check_cancelled()?;
        self.percent.store(percent.min(100), Ordering::SeqCst);
        self.stage.store(stage as u32, Ordering::SeqCst);
        Ok(())
    }
    fn check_cancelled(&self) -> Result<(), AppError> {
        if self.cancelled.load(Ordering::SeqCst) {
            Err(AppError::Message(IMPORT_CANCELLED_MESSAGE.to_string()))
        } else {
            Ok(())
        }
    }
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
    fn finish(&self) {
        self.percent.store(100, Ordering::SeqCst);
        self.stage
            .store(ImportStage::Finished as u32, Ordering::SeqCst);
    }
    fn percent(&self) -> u32 {
        self.percent.load(Ordering::SeqCst)
    }
    fn stage(&self) -> ImportStage {
        ImportStage::from_code(self.stage.load(Ordering::SeqCst))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImportOutcome {
    Completed,
    Cancelled,
}
fn is_import_cancelled(error: &AppError) -> bool {
    matches!(error, AppError::Message(message) if message == IMPORT_CANCELLED_MESSAGE)
}
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
pub fn run_if_needed(app: &AppHandle) -> Result<bool, AppError> {
    let destination_db = get_app_db_path();
    if destination_db.exists() {
        return Ok(false);
    }
    let legacy_db = get_legacy_app_db_path();
    if !legacy_db.is_file() {
        return Ok(false);
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
        return Ok(false);
    }
    let app_config_dir = get_app_config_dir();
    match run_import_with_progress(&legacy_db, &destination_db, &app_config_dir) {
        Ok(ImportOutcome::Completed) => Ok(true),
        Ok(ImportOutcome::Cancelled) => Ok(false),
        Err(error) => {
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
            Err(error)
        }
    }
}
fn import_legacy_data_with_progress(
    legacy_db: &Path,
    destination_db: &Path,
    destination_config_dir: &Path,
    progress: &ImportProgress,
) -> Result<ImportOutcome, AppError> {
    if let Err(error) = progress.report(1, ImportStage::Preparing) {
        return if is_import_cancelled(&error) {
            Ok(ImportOutcome::Cancelled)
        } else {
            Err(error)
        };
    }
    fs::create_dir_all(destination_config_dir)
        .map_err(|error| AppError::io(destination_config_dir, error))?;
    let parent = destination_db.parent().ok_or_else(|| {
        AppError::Config(format!(
            "无法确定 PPBind 数据库目录: {}",
            destination_db.display()
        ))
    })?;
    fs::create_dir_all(parent).map_err(|error| AppError::io(parent, error))?;
    let temp_path = temporary_import_path(destination_db);
    let legacy_config_dir = legacy_db.parent().ok_or_else(|| {
        AppError::Config(format!(
            "无法确定官方数据库所在目录: {}",
            legacy_db.display()
        ))
    })?;
    let result = (|| {
        prepare_database_snapshot(legacy_db, &temp_path, progress)?;
        progress.report(92, ImportStage::ScanningFiles)?;
        for warning in copy_auxiliary_data(legacy_config_dir, destination_config_dir, progress)? {
            eprintln!("PPBind import warning: {warning}");
        }
        progress.report(99, ImportStage::FinalizingDatabase)?;
        progress.check_cancelled()?;
        commit_database_snapshot(&temp_path, destination_db)?;
        progress.finish();
        Ok(())
    })();
    match result {
        Ok(()) => Ok(ImportOutcome::Completed),
        Err(error) if is_import_cancelled(&error) => {
            if temp_path.exists() {
                let _ = fs::remove_file(&temp_path);
            }
            Ok(ImportOutcome::Cancelled)
        }
        Err(error) => {
            if temp_path.exists() {
                let _ = fs::remove_file(&temp_path);
            }
            Err(error)
        }
    }
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
    let progress = ImportProgress::default();
    let result = (|| {
        prepare_database_snapshot(source_path, &temp_path, &progress)?;
        commit_database_snapshot(&temp_path, destination_path)
    })();
    if result.is_err() && temp_path.exists() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}
fn prepare_database_snapshot(
    source_path: &Path,
    temp_path: &Path,
    progress: &ImportProgress,
) -> Result<(), AppError> {
    if temp_path.exists() {
        fs::remove_file(temp_path).map_err(|error| AppError::io(temp_path, error))?;
    }
    let result = (|| {
        progress.report(3, ImportStage::CopyingDatabase)?;
        let source = Connection::open_with_flags(source_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| AppError::Database(error.to_string()))?;
        let mut destination =
            Connection::open(temp_path).map_err(|error| AppError::Database(error.to_string()))?;
        {
            let backup = Backup::new(&source, &mut destination)
                .map_err(|error| AppError::Database(error.to_string()))?;
            loop {
                progress.check_cancelled()?;
                let step = backup
                    .step(BACKUP_PAGES_PER_STEP)
                    .map_err(|error| AppError::Database(error.to_string()))?;
                let backup_progress = backup.progress();
                let percent = if backup_progress.pagecount > 0 {
                    let done =
                        (backup_progress.pagecount - backup_progress.remaining).max(0) as f64;
                    let total = backup_progress.pagecount as f64;
                    (5.0 + (done / total) * 80.0).clamp(5.0, 85.0) as u32
                } else {
                    5
                };
                progress.report(percent, ImportStage::CopyingDatabase)?;
                match step {
                    StepResult::Done => break,
                    StepResult::Busy | StepResult::Locked => {
                        std::thread::sleep(BACKUP_BUSY_RETRY_DELAY)
                    }
                    StepResult::More => {}
                    _ => {}
                }
            }
        }
        drop(destination);
        drop(source);
        progress.report(87, ImportStage::CheckingDatabase)?;
        validate_database_snapshot(temp_path)?;
        progress.report(91, ImportStage::FinalizingDatabase)?;
        normalize_imported_proxy_port(temp_path)?;
        Ok(())
    })();
    if result.is_err() && temp_path.exists() {
        let _ = fs::remove_file(temp_path);
    }
    result
}
fn commit_database_snapshot(temp_path: &Path, destination_path: &Path) -> Result<(), AppError> {
    if destination_path.exists() {
        let _ = fs::remove_file(temp_path);
        return Ok(());
    }
    fs::rename(temp_path, destination_path).map_err(|error| AppError::io(destination_path, error))
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
/// Keep the imported proxy listener on the official CC Switch port.
///
/// Older PPBind builds used 15722. Normalize that legacy value in the copied
/// snapshot, while preserving the official 15721 port.
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
                "UPDATE proxy_config SET listen_port = ?1 WHERE listen_port = 15722",
                [DEFAULT_PROXY_PORT],
            )
            .map_err(|error| AppError::Database(error.to_string()))?;
    }
    Ok(())
}
fn temporary_import_path(destination_path: &Path) -> PathBuf {
    destination_path.with_extension("db.importing")
}
#[derive(Debug)]
struct AuxiliaryCopyTask {
    source: PathBuf,
    destination: PathBuf,
    size: u64,
}
fn copy_auxiliary_data(
    source_dir: &Path,
    destination_dir: &Path,
    progress: &ImportProgress,
) -> Result<Vec<String>, AppError> {
    progress.report(92, ImportStage::ScanningFiles)?;
    let mut warnings = Vec::new();
    let mut tasks = Vec::new();
    for name in AUXILIARY_FILES {
        collect_file_task_if_absent(
            &source_dir.join(name),
            &destination_dir.join(name),
            &mut tasks,
            &mut warnings,
            progress,
        )?;
    }
    let source_skills = source_dir.join("skills");
    if source_skills.is_dir() {
        collect_tree_tasks(
            &source_skills,
            &destination_dir.join("skills"),
            &mut tasks,
            &mut warnings,
            progress,
        )?;
    }
    let total_units = tasks.iter().fold(tasks.len() as u64, |total, task| {
        total.saturating_add(task.size)
    });
    let mut copied_units = 0u64;
    let mut buffer = vec![0u8; 1024 * 1024];
    progress.report(93, ImportStage::CopyingFiles)?;
    for task in tasks {
        progress.check_cancelled()?;
        let before = copied_units;
        match copy_file_with_progress(&task, total_units, &mut copied_units, &mut buffer, progress)
        {
            Ok(()) => {}
            Err(error) if is_import_cancelled(&error) => return Err(error),
            Err(error) => {
                let bytes_already_copied = copied_units.saturating_sub(before);
                copied_units =
                    copied_units.saturating_add(task.size.saturating_sub(bytes_already_copied));
                warnings.push(format!(
                    "复制文件失败 ({} -> {}): {error}",
                    task.source.display(),
                    task.destination.display()
                ));
            }
        }
        // Count each completed (or failed) file as one unit so zero-byte files
        // and many tiny skill files still move the progress bar.
        copied_units = copied_units.saturating_add(1);
        report_auxiliary_progress(progress, copied_units, total_units)?;
    }
    progress.report(99, ImportStage::CopyingFiles)?;
    Ok(warnings)
}
fn collect_file_task_if_absent(
    source: &Path,
    destination: &Path,
    tasks: &mut Vec<AuxiliaryCopyTask>,
    warnings: &mut Vec<String>,
    progress: &ImportProgress,
) -> Result<(), AppError> {
    progress.check_cancelled()?;
    if destination.exists() {
        return Ok(());
    }
    let metadata = match fs::symlink_metadata(source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            warnings.push(format!("读取文件信息失败 ({}): {error}", source.display()));
            return Ok(());
        }
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Ok(());
    }
    tasks.push(AuxiliaryCopyTask {
        source: source.to_path_buf(),
        destination: destination.to_path_buf(),
        size: metadata.len(),
    });
    Ok(())
}
fn collect_tree_tasks(
    source: &Path,
    destination: &Path,
    tasks: &mut Vec<AuxiliaryCopyTask>,
    warnings: &mut Vec<String>,
    progress: &ImportProgress,
) -> Result<(), AppError> {
    progress.check_cancelled()?;
    let entries = match fs::read_dir(source) {
        Ok(entries) => entries,
        Err(error) => {
            warnings.push(format!("读取目录 {} 失败: {error}", source.display()));
            return Ok(());
        }
    };
    for entry in entries {
        progress.check_cancelled()?;
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                warnings.push(format!("读取目录项失败: {error}"));
                continue;
            }
        };
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                warnings.push(format!(
                    "读取类型失败 ({}): {error}",
                    entry.path().display()
                ));
                continue;
            }
        };
        // Never follow links outside the official data directory.
        if file_type.is_symlink() {
            continue;
        }
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if file_type.is_dir() {
            collect_tree_tasks(&source_path, &destination_path, tasks, warnings, progress)?;
        } else if file_type.is_file() && !destination_path.exists() {
            let metadata = match fs::symlink_metadata(&source_path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    warnings.push(format!(
                        "读取文件信息失败 ({}): {error}",
                        source_path.display()
                    ));
                    continue;
                }
            };
            tasks.push(AuxiliaryCopyTask {
                source: source_path,
                destination: destination_path,
                size: metadata.len(),
            });
        }
    }
    Ok(())
}
fn copy_file_with_progress(
    task: &AuxiliaryCopyTask,
    total_units: u64,
    copied_units: &mut u64,
    buffer: &mut [u8],
    progress: &ImportProgress,
) -> Result<(), AppError> {
    use std::io::{ErrorKind, Read, Write};
    if task.destination.exists() {
        return Ok(());
    }
    if let Some(parent) = task.destination.parent() {
        fs::create_dir_all(parent).map_err(|error| AppError::io(parent, error))?;
    }
    let mut input =
        std::fs::File::open(&task.source).map_err(|error| AppError::io(&task.source, error))?;
    let mut output = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&task.destination)
    {
        Ok(output) => output,
        Err(error) if error.kind() == ErrorKind::AlreadyExists => return Ok(()),
        Err(error) => return Err(AppError::io(&task.destination, error)),
    };
    let result = (|| {
        loop {
            progress.check_cancelled()?;
            let read = input
                .read(buffer)
                .map_err(|error| AppError::io(&task.source, error))?;
            if read == 0 {
                break;
            }
            output
                .write_all(&buffer[..read])
                .map_err(|error| AppError::io(&task.destination, error))?;
            *copied_units = copied_units.saturating_add(read as u64);
            report_auxiliary_progress(progress, *copied_units, total_units)?;
        }
        output
            .flush()
            .map_err(|error| AppError::io(&task.destination, error))?;
        Ok(())
    })();
    if result.is_err() {
        drop(output);
        let _ = fs::remove_file(&task.destination);
        return result;
    }
    if let Ok(metadata) = fs::metadata(&task.source) {
        let _ = fs::set_permissions(&task.destination, metadata.permissions());
    }
    Ok(())
}
fn report_auxiliary_progress(
    progress: &ImportProgress,
    copied_units: u64,
    total_units: u64,
) -> Result<(), AppError> {
    let percent = if total_units == 0 {
        99
    } else {
        93 + copied_units
            .saturating_mul(6)
            .checked_div(total_units)
            .unwrap_or(6)
            .min(6) as u32
    };
    progress.report(percent, ImportStage::CopyingFiles)
}
#[cfg(target_os = "windows")]
#[derive(Default)]
struct TaskDialogState {
    progress: Arc<AtomicU32>,
    stage: Arc<AtomicU32>,
    cancelled: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
    last_percent: AtomicU32,
    last_stage: AtomicU32,
    close_posted: AtomicBool,
    chinese: bool,
}
#[cfg(target_os = "windows")]
fn run_import_with_progress(
    legacy_db: &Path,
    destination_db: &Path,
    destination_config_dir: &Path,
) -> Result<ImportOutcome, AppError> {
    use windows_sys::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows_sys::Win32::UI::Controls::{
        TaskDialogIndirect, TASKDIALOGCONFIG, TDE_CONTENT, TDF_CALLBACK_TIMER,
        TDF_SHOW_PROGRESS_BAR, TDF_SIZE_TO_CONTENT, TDM_SET_ELEMENT_TEXT, TDM_SET_PROGRESS_BAR_POS,
        TDM_SET_PROGRESS_BAR_RANGE, TDN_BUTTON_CLICKED, TDN_CREATED, TDN_TIMER,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        PostMessageW, SendMessageW, IDCANCEL, WM_CLOSE,
    };
    unsafe extern "system" fn callback(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        _lparam: LPARAM,
        callback_data: isize,
    ) -> windows_sys::core::HRESULT {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let state = &*(callback_data as *const TaskDialogState);
            match message as i32 {
                TDN_CREATED => {
                    SendMessageW(hwnd, TDM_SET_PROGRESS_BAR_RANGE as u32, 0, 100);
                }
                TDN_BUTTON_CLICKED if wparam as i32 == IDCANCEL => {
                    // Ignore user close requests while the import is running.
                    // Once finished, allow the programmatic WM_CLOSE to complete.
                    if state.finished.load(Ordering::SeqCst) {
                        return 0;
                    }
                    return 1;
                }
                TDN_TIMER => {
                    if state.finished.load(Ordering::SeqCst) {
                        let percent = 100;
                        if state.last_percent.swap(percent, Ordering::SeqCst) != percent {
                            SendMessageW(
                                hwnd,
                                TDM_SET_PROGRESS_BAR_POS as u32,
                                percent as usize,
                                0,
                            );
                        }
                        let stage_code = ImportStage::Finished as u32;
                        if state.last_stage.swap(stage_code, Ordering::SeqCst) != stage_code {
                            let text =
                                format!("{}  ·  100%", ImportStage::Finished.label(state.chinese));
                            let wide: Vec<u16> = std::ffi::OsStr::new(&text)
                                .encode_wide()
                                .chain(std::iter::once(0))
                                .collect();
                            SendMessageW(
                                hwnd,
                                TDM_SET_ELEMENT_TEXT as u32,
                                TDE_CONTENT as usize,
                                wide.as_ptr() as LPARAM,
                            );
                        }
                        if !state.close_posted.swap(true, Ordering::SeqCst) {
                            PostMessageW(hwnd, WM_CLOSE, 0, 0);
                        }
                        return 0;
                    }
                    let percent = state.progress.load(Ordering::SeqCst).min(100);
                    let percent_changed =
                        state.last_percent.swap(percent, Ordering::SeqCst) != percent;
                    if percent_changed {
                        SendMessageW(hwnd, TDM_SET_PROGRESS_BAR_POS as u32, percent as usize, 0);
                    }
                    let stage_code = state.stage.load(Ordering::SeqCst);
                    let stage_changed =
                        state.last_stage.swap(stage_code, Ordering::SeqCst) != stage_code;
                    let cancelled = state.cancelled.load(Ordering::SeqCst);
                    let text = if cancelled {
                        if state.chinese {
                            "正在取消导入，请稍候…".to_string()
                        } else {
                            "Cancelling import, please wait…".to_string()
                        }
                    } else {
                        let stage = ImportStage::from_code(stage_code);
                        let label = stage.label(state.chinese);
                        format!("{label}  ·  {percent}%")
                    };
                    if cancelled || percent_changed || stage_changed {
                        let wide: Vec<u16> = std::ffi::OsStr::new(&text)
                            .encode_wide()
                            .chain(std::iter::once(0))
                            .collect();
                        SendMessageW(
                            hwnd,
                            TDM_SET_ELEMENT_TEXT as u32,
                            TDE_CONTENT as usize,
                            wide.as_ptr() as LPARAM,
                        );
                    }
                }
                _ => {}
            }
            0
        }))
        .unwrap_or(0)
    }
    let worker_progress = ImportProgress::default();
    let worker_progress_for_thread = worker_progress.clone();
    let worker_finished = Arc::new(AtomicBool::new(false));
    let worker_finished_for_thread = worker_finished.clone();
    let legacy_db = legacy_db.to_path_buf();
    let destination_db = destination_db.to_path_buf();
    let destination_config_dir = destination_config_dir.to_path_buf();
    let worker = std::thread::Builder::new()
        .name("ppbind-legacy-import".to_string())
        .spawn(move || {
            let result = import_legacy_data_with_progress(
                &legacy_db,
                &destination_db,
                &destination_config_dir,
                &worker_progress_for_thread,
            );
            worker_progress_for_thread.finish();
            worker_finished_for_thread.store(true, Ordering::SeqCst);
            result
        })
        .map_err(|error| AppError::Message(format!("启动导入线程失败: {error}")))?;
    let state = TaskDialogState {
        progress: worker_progress.percent.clone(),
        stage: worker_progress.stage.clone(),
        cancelled: worker_progress.cancelled.clone(),
        finished: worker_finished,
        last_percent: AtomicU32::new(u32::MAX),
        last_stage: AtomicU32::new(u32::MAX),
        close_posted: AtomicBool::new(false),
        chinese: sys_locale::get_locale()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .starts_with("zh"),
    };
    // The dialog has no parent because PPBind's main window is still hidden.
    let title: Vec<u16> = std::ffi::OsStr::new("PPBind")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let main_instruction = if state.chinese {
        "正在导入官方 CC Switch 数据"
    } else {
        "Importing official CC Switch data"
    };
    let main_instruction: Vec<u16> = std::ffi::OsStr::new(main_instruction)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let initial_content = if state.chinese {
        "正在准备导入  ·  0%"
    } else {
        "Preparing import  ·  0%"
    };
    let initial_content: Vec<u16> = std::ffi::OsStr::new(initial_content)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut config = TASKDIALOGCONFIG::default();
    config.cbSize = std::mem::size_of::<TASKDIALOGCONFIG>() as u32;
    config.hwndParent = std::ptr::null_mut();
    config.hInstance = std::ptr::null_mut();
    config.dwFlags = TDF_SHOW_PROGRESS_BAR | TDF_CALLBACK_TIMER | TDF_SIZE_TO_CONTENT;
    // Import is intentionally not user-cancellable. It normally completes in
    // around a second and must close itself before PPBind continues startup.
    config.dwCommonButtons = 0;
    config.pszWindowTitle = title.as_ptr();
    config.pszMainInstruction = main_instruction.as_ptr();
    config.pszContent = initial_content.as_ptr();
    config.pfCallback = Some(callback);
    config.lpCallbackData = (&state as *const TaskDialogState) as isize;
    let mut button = 0;
    let dialog_result = unsafe {
        TaskDialogIndirect(
            &config,
            &mut button,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    let worker_result = worker
        .join()
        .map_err(|_| AppError::Message("导入线程异常退出".to_string()))??;
    if dialog_result < 0 {
        return Err(AppError::Message(format!(
            "显示导入进度窗口失败 (HRESULT: {dialog_result})"
        )));
    }
    Ok(worker_result)
}
#[cfg(not(target_os = "windows"))]
fn run_import_with_progress(
    legacy_db: &Path,
    destination_db: &Path,
    destination_config_dir: &Path,
) -> Result<ImportOutcome, AppError> {
    import_legacy_data_with_progress(
        legacy_db,
        destination_db,
        destination_config_dir,
        &ImportProgress::default(),
    )
}
#[cfg(target_os = "windows")]
fn is_official_app_running() -> Result<bool, AppError> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(AppError::Message(format!(
                "检查官方 CC Switch 进程失败: {}",
                std::io::Error::last_os_error()
            )));
        }
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut found = false;
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                if process_entry_name(&entry).eq_ignore_ascii_case(OFFICIAL_PROCESS_NAME) {
                    found = true;
                    break;
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
        Ok(found)
    }
}
#[cfg(target_os = "windows")]
fn process_entry_name(
    entry: &windows_sys::Win32::System::Diagnostics::ToolHelp::PROCESSENTRY32W,
) -> String {
    let end = entry
        .szExeFile
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(entry.szExeFile.len());
    String::from_utf16_lossy(&entry.szExeFile[..end])
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
    fn database_import_preserves_official_port_and_normalizes_legacy_ppbind_port() {
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
                    VALUES ('claude', 15721), ('codex', 15722);",
                )
                .expect("create proxy config");
        }
        copy_database_snapshot(&source, &destination).expect("import database");
        let source_connection = Connection::open(&source).expect("open source");
        let source_ports: Vec<u16> = source_connection
            .prepare("SELECT listen_port FROM proxy_config ORDER BY app_type")
            .expect("prepare source query")
            .query_map([], |row| row.get(0))
            .expect("query source ports")
            .collect::<Result<_, _>>()
            .expect("read source ports");
        assert_eq!(source_ports, vec![15721, 15722]);
        let destination_connection = Connection::open(&destination).expect("open destination");
        let destination_ports: Vec<u16> = destination_connection
            .prepare("SELECT listen_port FROM proxy_config ORDER BY app_type")
            .expect("prepare destination query")
            .query_map([], |row| row.get(0))
            .expect("query destination ports")
            .collect::<Result<_, _>>()
            .expect("read destination ports");
        assert_eq!(
            destination_ports,
            vec![DEFAULT_PROXY_PORT, DEFAULT_PROXY_PORT]
        );
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
        fs::create_dir_all(source.join("backups")).expect("create backups");
        fs::create_dir_all(&destination).expect("create destination");
        fs::write(source.join("settings.json"), "official").expect("write settings");
        fs::write(source.join("cc-switch.db"), "db").expect("write db");
        fs::write(source.join("logs").join("app.log"), "log").expect("write log");
        fs::write(source.join("backups").join("backup.db"), "backup").expect("write backup");
        fs::write(source.join("skills").join("demo").join("SKILL.md"), "skill")
            .expect("write skill");
        fs::write(destination.join("settings.json"), "ppbind").expect("write existing settings");
        let progress = ImportProgress::default();
        let warnings =
            copy_auxiliary_data(&source, &destination, &progress).expect("copy auxiliary data");
        assert_eq!(progress.percent(), 99);
        assert_eq!(progress.stage(), ImportStage::CopyingFiles);
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
        assert!(!destination.join("backups").exists());
    }
    #[test]
    fn import_progress_reaches_completion() {
        let temp = TempDir::new().expect("tempdir");
        let source = temp.path().join("official").join("cc-switch.db");
        let destination = temp.path().join("ppbind").join("ppbind.db");
        let config = temp.path().join("ppbind");
        fs::create_dir_all(source.parent().expect("source parent")).expect("create source dir");
        create_database(&source, "official");
        let progress = ImportProgress::default();
        let outcome = import_legacy_data_with_progress(&source, &destination, &config, &progress)
            .expect("import with progress");
        assert_eq!(outcome, ImportOutcome::Completed);
        assert_eq!(progress.percent(), 100);
        assert_eq!(progress.stage(), ImportStage::Finished);
        assert!(destination.is_file());
    }
    #[test]
    fn cancelled_import_does_not_commit_database() {
        let temp = TempDir::new().expect("tempdir");
        let source = temp.path().join("official").join("cc-switch.db");
        let destination = temp.path().join("ppbind").join("ppbind.db");
        let config = temp.path().join("ppbind");
        fs::create_dir_all(source.parent().expect("source parent")).expect("create source dir");
        create_database(&source, "official");
        let progress = ImportProgress::default();
        progress.cancel();
        let outcome = import_legacy_data_with_progress(&source, &destination, &config, &progress)
            .expect("cancelled import");
        assert_eq!(outcome, ImportOutcome::Cancelled);
        assert!(!destination.exists());
        assert!(!temporary_import_path(&destination).exists());
    }
    #[cfg(target_os = "windows")]
    #[test]
    fn process_entry_name_reads_nt_unicode_and_stops_at_nul() {
        let mut entry =
            windows_sys::Win32::System::Diagnostics::ToolHelp::PROCESSENTRY32W::default();
        let name: Vec<u16> = "cc-switch.exe"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        entry.szExeFile[..name.len()].copy_from_slice(&name);
        assert_eq!(process_entry_name(&entry), OFFICIAL_PROCESS_NAME);
    }
}
