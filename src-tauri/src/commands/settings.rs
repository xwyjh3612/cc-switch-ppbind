#![allow(non_snake_case)]

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncWriteExt;

const PPBIND_REPO: &str = "xwyjh3612/cc-switch-ppbind";
const PPBIND_RELEASES_URL: &str = "https://github.com/xwyjh3612/cc-switch-ppbind/releases";
const PPBIND_UPDATE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Deserialize)]
struct GithubReleaseAsset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubRelease {
    tag_name: String,
    body: Option<String>,
    published_at: Option<String>,
    html_url: Option<String>,
    assets: Vec<GithubReleaseAsset>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PpbindUpdateInfo {
    current_version: String,
    available_version: String,
    notes: Option<String>,
    pub_date: Option<String>,
    release_url: String,
    download_url: String,
    asset_name: String,
    sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct UpdateDownloadProgress {
    downloaded: u64,
    total: Option<u64>,
}

fn ppbind_github_token() -> Option<String> {
    ["PPBIND_GITHUB_TOKEN", "GITHUB_TOKEN"]
        .iter()
        .find_map(|key| std::env::var(key).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn configure_github_request(
    request: reqwest::RequestBuilder,
    include_auth: bool,
) -> reqwest::RequestBuilder {
    let request = request
        .header("User-Agent", "PPBind")
        .header("Accept", "application/vnd.github+json");
    if include_auth {
        if let Some(token) = ppbind_github_token() {
            return request.bearer_auth(token);
        }
    }
    request
}

async fn fetch_ppbind_latest_release() -> Result<GithubRelease, String> {
    let client = reqwest::Client::builder()
        .timeout(PPBIND_UPDATE_TIMEOUT)
        .build()
        .map_err(|e| format!("初始化更新客户端失败: {e}"))?;
    let response = configure_github_request(
        client.get(format!(
            "https://api.github.com/repos/{PPBIND_REPO}/releases/latest"
        )),
        true,
    )
    .send()
    .await
    .map_err(|e| format!("连接 GitHub 检查 PPBind 更新失败: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let hint = if status == reqwest::StatusCode::NOT_FOUND {
            "。请确认 PPBind Release 仓库已公开发布；自用构建也可设置 PPBIND_GITHUB_TOKEN 访问 private Release"
        } else {
            ""
        };
        return Err(format!("检查 PPBind 更新失败（HTTP {status}）{hint}"));
    }

    response
        .json::<GithubRelease>()
        .await
        .map_err(|e| format!("解析 PPBind 更新信息失败: {e}"))
}

fn parse_version(value: &str) -> Option<(u64, u64, u64, Option<String>)> {
    let value = value.trim().trim_start_matches(['v', 'V']);
    let (core, prerelease) = match value.split_once('-') {
        Some((core, prerelease)) => (core, Some(prerelease.to_string())),
        None => (value, None),
    };
    let core = core.split('+').next().unwrap_or(core);
    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next().unwrap_or("0").parse::<u64>().ok()?;
    let patch = parts.next().unwrap_or("0").parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch, prerelease))
}

fn compare_prerelease(left: &str, right: &str) -> Ordering {
    let left_parts = left.split('.').collect::<Vec<_>>();
    let right_parts = right.split('.').collect::<Vec<_>>();
    for index in 0..left_parts.len().max(right_parts.len()) {
        match (left_parts.get(index), right_parts.get(index)) {
            (Some(left), Some(right)) => {
                let ordering = match (left.parse::<u64>(), right.parse::<u64>()) {
                    (Ok(left), Ok(right)) => left.cmp(&right),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => left.cmp(right),
                };
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
            (Some(_), None) => return Ordering::Greater,
            (None, Some(_)) => return Ordering::Less,
            (None, None) => break,
        }
    }
    Ordering::Equal
}

fn compare_versions(left: &str, right: &str) -> Option<Ordering> {
    let (left_major, left_minor, left_patch, left_pre) = parse_version(left)?;
    let (right_major, right_minor, right_patch, right_pre) = parse_version(right)?;
    let core_order =
        (left_major, left_minor, left_patch).cmp(&(right_major, right_minor, right_patch));
    if core_order != Ordering::Equal {
        return Some(core_order);
    }
    Some(match (left_pre, right_pre) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(left), Some(right)) => compare_prerelease(&left, &right),
    })
}

fn update_asset_score(name: &str) -> Option<u8> {
    let lower = name.to_ascii_lowercase();
    let native_arm = std::env::consts::ARCH.eq_ignore_ascii_case("aarch64");
    let native_x64 = std::env::consts::ARCH.eq_ignore_ascii_case("x86_64");

    #[cfg(target_os = "windows")]
    {
        let arch_score = if (native_x64 && (lower.contains("x64") || lower.contains("x86_64")))
            || (native_arm && (lower.contains("arm64") || lower.contains("aarch64")))
        {
            0
        } else if lower.contains("x64")
            || lower.contains("x86_64")
            || lower.contains("arm64")
            || lower.contains("aarch64")
        {
            1
        } else {
            2
        };
        if lower.ends_with(".exe") && lower.contains("setup") {
            return Some(arch_score);
        }
        if lower.ends_with(".msi") {
            return Some(10 + arch_score);
        }
        return None;
    }

    #[cfg(target_os = "macos")]
    {
        let _ = (native_arm, native_x64);
        if lower.ends_with(".dmg") {
            return Some(0);
        }
        if lower.ends_with(".app.tar.gz") {
            return Some(1);
        }
        if lower.ends_with(".zip") {
            return Some(2);
        }
        return None;
    }

    #[cfg(target_os = "linux")]
    {
        let _ = (native_arm, native_x64);
        if lower.ends_with(".appimage") {
            return Some(0);
        }
        if lower.ends_with(".deb") {
            return Some(1);
        }
        if lower.ends_with(".rpm") {
            return Some(2);
        }
        return None;
    }

    #[allow(unreachable_code)]
    None
}

fn select_update_asset<'a>(assets: &'a [GithubReleaseAsset]) -> Option<&'a GithubReleaseAsset> {
    assets
        .iter()
        .filter_map(|asset| update_asset_score(&asset.name).map(|score| (score, asset)))
        .min_by_key(|(score, _)| *score)
        .map(|(_, asset)| asset)
}

async fn resolve_ppbind_update() -> Result<Option<PpbindUpdateInfo>, String> {
    let release = fetch_ppbind_latest_release().await?;
    let current_version = env!("CARGO_PKG_VERSION");
    if compare_versions(&release.tag_name, current_version) != Some(Ordering::Greater) {
        return Ok(None);
    }

    let asset = select_update_asset(&release.assets).ok_or_else(|| {
        format!(
            "PPBind {} 没有适用于当前系统（{}）的安装包",
            release.tag_name,
            std::env::consts::OS
        )
    })?;
    let sha256 = asset
        .digest
        .as_deref()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .map(str::to_ascii_lowercase);

    Ok(Some(PpbindUpdateInfo {
        current_version: current_version.to_string(),
        available_version: release.tag_name.trim_start_matches(['v', 'V']).to_string(),
        notes: release.body.filter(|notes| !notes.trim().is_empty()),
        pub_date: release.published_at,
        release_url: release
            .html_url
            .unwrap_or_else(|| PPBIND_RELEASES_URL.to_string()),
        download_url: asset.browser_download_url.clone(),
        asset_name: asset.name.clone(),
        sha256,
    }))
}

fn merge_settings_for_save(
    mut incoming: crate::settings::AppSettings,
    existing: &crate::settings::AppSettings,
) -> crate::settings::AppSettings {
    match (&mut incoming.webdav_sync, &existing.webdav_sync) {
        // incoming 没有 webdav → 保留现有
        (None, _) => {
            incoming.webdav_sync = existing.webdav_sync.clone();
        }
        // incoming 有 webdav 但密码为空，且现有有密码 → 填回现有密码
        // （get_settings_for_frontend 总是清空密码，所以通过 save_settings
        //   传入的空密码意味着"保持现有"而非"用户主动清空"）
        (Some(incoming_sync), Some(existing_sync))
            if incoming_sync.password.is_empty() && !existing_sync.password.is_empty() =>
        {
            incoming_sync.password = existing_sync.password.clone();
        }
        _ => {}
    }
    match (&mut incoming.s3_sync, &existing.s3_sync) {
        // incoming 没有 s3 → 保留现有
        (None, _) => {
            incoming.s3_sync = existing.s3_sync.clone();
        }
        // incoming 有 s3 但密钥为空，且现有有密钥 → 填回现有密钥
        (Some(incoming_sync), Some(existing_sync))
            if incoming_sync.secret_access_key.is_empty()
                && !existing_sync.secret_access_key.is_empty() =>
        {
            incoming_sync.secret_access_key = existing_sync.secret_access_key.clone();
        }
        _ => {}
    }
    // local_migrations 是纯后端状态（迁移完成标记），前端没有合法的修改场景，
    // 无条件取现有值。若按 incoming 透传：后端清掉 marker（如关闭统一会话
    // 开关）后、前端 query 缓存刷新前的一次全量保存会把旧 marker 重放回来，
    // 重新开启时被"复活"的标记挡住而漏迁。
    incoming.local_migrations = existing.local_migrations.clone();
    incoming
}

/// 获取设置
#[tauri::command]
pub async fn get_settings() -> Result<crate::settings::AppSettings, String> {
    Ok(crate::settings::get_settings_for_frontend())
}

/// 保存设置
#[tauri::command]
pub async fn save_settings(
    state: tauri::State<'_, crate::store::AppState>,
    settings: crate::settings::AppSettings,
) -> Result<bool, String> {
    let existing = crate::settings::get_settings();
    let merged = merge_settings_for_save(settings, &existing);
    let unify_codex_changed =
        merged.unify_codex_session_history != existing.unify_codex_session_history;
    let unify_codex_enabled = merged.unify_codex_session_history;
    crate::settings::update_settings(merged).map_err(|e| e.to_string())?;

    // 统一会话开关变更时立即重写当前官方 Codex 供应商的 live 配置，
    // 不必等下一次切换才生效。
    if unify_codex_changed {
        // live 重写失败时回滚设置并把保存整体报失败：若设置保持已切换状态，
        // live 仍跑旧桶，后续的历史迁移/还原会让会话再次分裂（开启=历史
        // 迁走而新会话仍写 openai 桶；关闭=会话还原而 live 仍写 custom）。
        // 报错让前端 saved=false 短路还原；回滚是整次保存的事务语义
        // （本开关的保存只携带开关相关字段）。
        if let Err(err) =
            crate::services::provider::reapply_current_codex_official_live(state.inner())
        {
            log::warn!("统一 Codex 会话历史开关变更后重写 live 配置失败，回滚设置: {err}");
            if let Err(rollback_err) = crate::settings::update_settings(existing) {
                log::error!("回滚统一会话开关设置失败: {rollback_err}");
            }
            return Err(format!(
                "统一 Codex 会话历史开关未生效（live 配置重写失败）: {err}"
            ));
        }

        if unify_codex_enabled {
            // 后台执行存量迁移（openai 桶 → custom 桶；仅当用户勾选了迁入既有
            // 会话，函数内部自门控）。大会话目录可能要读数秒，不能阻塞设置保存；
            // 失败时不写完成标记，下次启动自动重试。
            tauri::async_runtime::spawn_blocking(|| {
                match crate::codex_history_migration::maybe_migrate_codex_official_history_to_unified_bucket() {
                    Ok(outcome) => {
                        if let Some(reason) = outcome.skipped_reason {
                            log::debug!("○ Codex official history unify migration skipped: {reason}");
                        } else {
                            log::info!(
                                "✓ Codex official history unify migration completed: jsonl_files={}, state_rows={}",
                                outcome.migrated_jsonl_files,
                                outcome.migrated_state_rows
                            );
                        }
                    }
                    Err(e) => {
                        log::warn!("✗ Codex official history unify migration failed: {e}");
                    }
                }
            });
        } else {
            // 清除标记与迁移意愿，让重新开启并再次勾选时能补迁
            // 关闭期间落入 openai 桶的官方会话。
            if let Err(err) = crate::settings::clear_codex_official_history_unify_migration() {
                log::warn!("清除统一会话迁移标记失败: {err}");
            }
            if let Err(err) = crate::settings::clear_codex_unify_migrate_existing() {
                log::warn!("清除统一会话迁移意愿失败: {err}");
            }
        }
    }
    Ok(true)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexUnifyHistoryRestoreResult {
    pub restored_jsonl_files: usize,
    pub restored_state_rows: usize,
    /// 还原被跳过的原因（如当前目录没有账本），前端据此提示而非报"成功 0 项"。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<String>,
}

/// 是否存在统一会话开关的迁移备份（决定关闭弹窗里是否显示"恢复备份"勾选）。
#[tauri::command]
pub async fn has_codex_unify_history_backup() -> Result<bool, String> {
    Ok(crate::codex_history_migration::has_codex_official_history_unify_backup())
}

/// 按迁移备份账本把当时迁入共享桶的官方会话还原回 "openai" 桶。
/// 由关闭统一会话开关的确认弹窗触发；幂等，可安全重试。
#[tauri::command]
pub async fn restore_codex_unified_history() -> Result<CodexUnifyHistoryRestoreResult, String> {
    let outcome = tauri::async_runtime::spawn_blocking(|| {
        crate::codex_history_migration::restore_codex_official_history_from_backups()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    if let Some(reason) = &outcome.skipped_reason {
        log::debug!("○ Codex official history restore skipped: {reason}");
    } else {
        log::info!(
            "✓ Codex official history restored from backups: jsonl_files={}, state_rows={}",
            outcome.restored_jsonl_files,
            outcome.restored_state_rows
        );
    }

    Ok(CodexUnifyHistoryRestoreResult {
        restored_jsonl_files: outcome.restored_jsonl_files,
        restored_state_rows: outcome.restored_state_rows,
        skipped_reason: outcome.skipped_reason,
    })
}

/// 重启应用程序（当 app_config_dir 变更后使用）
#[tauri::command]
pub async fn restart_app(app: AppHandle) -> Result<bool, String> {
    crate::save_window_state_before_exit(&app);

    // 在后台延迟重启，让函数有时间返回响应
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        // app.restart() 走 RESTART_EXIT_CODE 路径，ExitRequested 处理器会直接
        // 放行给 Tauri 默认 re-exec，不执行代理/Live 清理。但本命令用于
        // app_config_dir 变更后的重启：新实例会切到新数据库，拿不到旧库里的
        // Live 备份，无法恢复被接管的 Live 配置。因此必须趁旧实例的事件循环
        // 仍存活，在这里同步完成恢复（保留代理状态，新实例启动时自动重新接管）。
        crate::cleanup_before_exit(&app).await;
        app.restart();
    });
    Ok(true)
}

#[tauri::command]
pub async fn check_ppbind_update(_app: AppHandle) -> Result<Option<PpbindUpdateInfo>, String> {
    resolve_ppbind_update().await
}

async fn download_ppbind_update(
    app: &AppHandle,
    info: &PpbindUpdateInfo,
) -> Result<PathBuf, String> {
    let expected_sha256 = info
        .sha256
        .as_deref()
        .ok_or_else(|| "更新包缺少 SHA-256 校验值，已拒绝安装".to_string())?;
    let file_name = Path::new(&info.asset_name)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "更新包文件名无效".to_string())?;
    let update_dir = std::env::temp_dir().join("ppbind-updates");
    tokio::fs::create_dir_all(&update_dir)
        .await
        .map_err(|e| format!("创建更新目录失败: {e}"))?;
    let target = update_dir.join(file_name);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| format!("初始化下载客户端失败: {e}"))?;
    let response = configure_github_request(client.get(&info.download_url), true)
        .send()
        .await
        .map_err(|e| format!("下载 PPBind {} 失败: {e}", info.available_version))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("下载 PPBind 更新失败（HTTP {status}）"));
    }

    let total = response.content_length();
    let mut stream = response.bytes_stream();
    let mut file = tokio::fs::File::create(&target)
        .await
        .map_err(|e| format!("创建更新包失败: {e}"))?;
    let mut hasher = Sha256::new();
    let mut downloaded = 0_u64;
    let _ = app.emit(
        "update-download-progress",
        UpdateDownloadProgress { downloaded, total },
    );

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("读取更新包失败: {e}"))?;
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("写入更新包失败: {e}"))?;
        downloaded += chunk.len() as u64;
        let _ = app.emit(
            "update-download-progress",
            UpdateDownloadProgress { downloaded, total },
        );
    }
    file.flush()
        .await
        .map_err(|e| format!("保存更新包失败: {e}"))?;
    drop(file);

    let actual_sha256 = format!("{:x}", hasher.finalize());
    if !actual_sha256.eq_ignore_ascii_case(expected_sha256) {
        let _ = tokio::fs::remove_file(&target).await;
        return Err("更新包 SHA-256 校验失败，已停止安装".to_string());
    }
    Ok(target)
}

#[cfg(target_os = "windows")]
fn powershell_quote(value: &Path) -> String {
    format!("'{}'", value.to_string_lossy().replace('\'', "''"))
}

#[cfg(target_os = "windows")]
fn schedule_update_install(app: &AppHandle, package: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let current_exe = std::env::current_exe().map_err(|e| format!("获取当前程序路径失败: {e}"))?;
    let installed_exe = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("PPBind").join("ppbind.exe"))
        .unwrap_or_else(|| current_exe.clone());
    let script_path = package.with_extension("update.ps1");
    let installer = powershell_quote(package);
    let installed = powershell_quote(&installed_exe);
    let fallback = powershell_quote(&current_exe);
    let package_literal = powershell_quote(package);
    let script = format!(
        "$ErrorActionPreference = 'Stop'\r\nStart-Sleep -Milliseconds 900\r\n$process = Start-Process -FilePath {installer} -ArgumentList '/S' -Wait -PassThru\r\nif ($process.ExitCode -ne 0) {{ throw \"PPBind installer failed: $($process.ExitCode)\" }}\r\n$launcher = {installed}\r\nif (-not (Test-Path -LiteralPath $launcher)) {{ $launcher = {fallback} }}\r\nStart-Process -FilePath $launcher\r\nRemove-Item -LiteralPath {package_literal} -Force -ErrorAction SilentlyContinue\r\nRemove-Item -LiteralPath $PSCommandPath -Force -ErrorAction SilentlyContinue\r\n"
    );
    std::fs::write(&script_path, script).map_err(|e| format!("创建更新启动脚本失败: {e}"))?;

    let spawn_result = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-WindowStyle",
            "Hidden",
            "-File",
        ])
        .arg(&script_path)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    if let Err(error) = spawn_result {
        let _ = std::fs::remove_file(&script_path);
        return Err(format!("启动更新安装程序失败: {error}"));
    }

    log::info!(
        "PPBind {} update downloaded; installation will continue in the background",
        env!("CARGO_PKG_VERSION")
    );
    let _ = app.emit("update-install-started", ());
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn schedule_update_install(_app: &AppHandle, package: &Path) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    _app.opener()
        .open_path(package.to_string_lossy().to_string(), None::<String>)
        .map_err(|e| format!("打开更新安装包失败: {e}"))
}

#[tauri::command]
pub async fn install_update_and_restart(app: AppHandle) -> Result<bool, String> {
    let Some(info) = resolve_ppbind_update().await? else {
        return Ok(false);
    };
    let package = download_ppbind_update(&app, &info).await?;
    schedule_update_install(&app, &package)?;
    Ok(true)
}

#[tauri::command]
pub async fn check_app_update_available(_app: AppHandle) -> Result<Option<String>, String> {
    Ok(resolve_ppbind_update()
        .await?
        .map(|info| info.available_version))
}
/// 获取 app_config_dir 覆盖配置 (从 Store)
#[tauri::command]
pub async fn get_app_config_dir_override(app: AppHandle) -> Result<Option<String>, String> {
    Ok(crate::app_store::refresh_app_config_dir_override(&app)
        .map(|p| p.to_string_lossy().to_string()))
}

/// 设置 app_config_dir 覆盖配置 (到 Store)
#[tauri::command]
pub async fn set_app_config_dir_override(
    app: AppHandle,
    path: Option<String>,
) -> Result<bool, String> {
    crate::app_store::set_app_config_dir_to_store(&app, path.as_deref())?;
    Ok(true)
}

/// 设置开机自启
#[tauri::command]
pub async fn set_auto_launch(enabled: bool) -> Result<bool, String> {
    if enabled {
        crate::auto_launch::enable_auto_launch().map_err(|e| format!("启用开机自启失败: {e}"))?;
    } else {
        crate::auto_launch::disable_auto_launch().map_err(|e| format!("禁用开机自启失败: {e}"))?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::merge_settings_for_save;
    use crate::settings::{
        AppSettings, CodexOfficialHistoryUnifyMigration, CodexProviderTemplateMigration,
        CodexThirdPartyHistoryProviderBucketMigration, LocalMigrations, S3SyncSettings,
        WebDavSyncSettings,
    };

    #[test]
    fn save_settings_should_preserve_existing_webdav_when_payload_omits_it() {
        let existing = AppSettings {
            webdav_sync: Some(WebDavSyncSettings {
                base_url: "https://dav.example.com".to_string(),
                username: "alice".to_string(),
                password: "secret".to_string(),
                ..WebDavSyncSettings::default()
            }),
            ..AppSettings::default()
        };

        let incoming = AppSettings::default();
        let merged = merge_settings_for_save(incoming, &existing);

        assert!(merged.webdav_sync.is_some());
        assert_eq!(
            merged.webdav_sync.as_ref().map(|v| v.base_url.as_str()),
            Some("https://dav.example.com")
        );
    }

    #[test]
    fn save_settings_should_keep_incoming_webdav_when_present() {
        let existing = AppSettings {
            webdav_sync: Some(WebDavSyncSettings {
                base_url: "https://dav.old.example.com".to_string(),
                username: "old".to_string(),
                password: "old-pass".to_string(),
                ..WebDavSyncSettings::default()
            }),
            ..AppSettings::default()
        };

        let incoming = AppSettings {
            webdav_sync: Some(WebDavSyncSettings {
                base_url: "https://dav.new.example.com".to_string(),
                username: "new".to_string(),
                password: "new-pass".to_string(),
                ..WebDavSyncSettings::default()
            }),
            ..AppSettings::default()
        };

        let merged = merge_settings_for_save(incoming, &existing);

        assert_eq!(
            merged.webdav_sync.as_ref().map(|v| v.base_url.as_str()),
            Some("https://dav.new.example.com")
        );
    }

    /// Regression test: frontend always receives empty password from
    /// get_settings_for_frontend(). If a component accidentally spreads
    /// the full settings object into save_settings, the empty password
    /// must NOT overwrite the existing one.
    #[test]
    fn save_settings_should_preserve_password_when_incoming_has_empty_password() {
        let existing = AppSettings {
            webdav_sync: Some(WebDavSyncSettings {
                base_url: "https://dav.example.com".to_string(),
                username: "alice".to_string(),
                password: "secret".to_string(),
                ..WebDavSyncSettings::default()
            }),
            ..AppSettings::default()
        };

        // Simulate frontend sending settings with cleared password
        let incoming = AppSettings {
            webdav_sync: Some(WebDavSyncSettings {
                base_url: "https://dav.example.com".to_string(),
                username: "alice".to_string(),
                password: "".to_string(),
                ..WebDavSyncSettings::default()
            }),
            ..AppSettings::default()
        };

        let merged = merge_settings_for_save(incoming, &existing);

        assert_eq!(
            merged.webdav_sync.as_ref().map(|v| v.password.as_str()),
            Some("secret"),
            "empty password from frontend must not overwrite existing password"
        );
    }

    /// When both incoming and existing have no password, merge should
    /// work without panicking and keep the empty state.
    #[test]
    fn save_settings_should_handle_both_empty_passwords() {
        let existing = AppSettings {
            webdav_sync: Some(WebDavSyncSettings {
                base_url: "https://dav.example.com".to_string(),
                username: "alice".to_string(),
                password: "".to_string(),
                ..WebDavSyncSettings::default()
            }),
            ..AppSettings::default()
        };

        let incoming = AppSettings {
            webdav_sync: Some(WebDavSyncSettings {
                base_url: "https://dav.example.com".to_string(),
                username: "alice".to_string(),
                password: "".to_string(),
                ..WebDavSyncSettings::default()
            }),
            ..AppSettings::default()
        };

        let merged = merge_settings_for_save(incoming, &existing);

        assert_eq!(
            merged.webdav_sync.as_ref().map(|v| v.password.as_str()),
            Some("")
        );
    }

    #[test]
    fn save_settings_should_preserve_existing_s3_when_payload_omits_it() {
        let existing = AppSettings {
            s3_sync: Some(S3SyncSettings {
                bucket: "bucket".to_string(),
                access_key_id: "ak".to_string(),
                secret_access_key: "secret".to_string(),
                ..S3SyncSettings::default()
            }),
            ..AppSettings::default()
        };

        let incoming = AppSettings::default();
        let merged = merge_settings_for_save(incoming, &existing);

        assert!(merged.s3_sync.is_some());
        assert_eq!(
            merged
                .s3_sync
                .as_ref()
                .map(|v| v.secret_access_key.as_str()),
            Some("secret")
        );
    }

    #[test]
    fn save_settings_should_preserve_s3_secret_when_incoming_has_empty_secret() {
        let existing = AppSettings {
            s3_sync: Some(S3SyncSettings {
                bucket: "bucket".to_string(),
                access_key_id: "ak".to_string(),
                secret_access_key: "secret".to_string(),
                ..S3SyncSettings::default()
            }),
            ..AppSettings::default()
        };

        let incoming = AppSettings {
            s3_sync: Some(S3SyncSettings {
                bucket: "bucket".to_string(),
                access_key_id: "ak".to_string(),
                secret_access_key: "".to_string(),
                ..S3SyncSettings::default()
            }),
            ..AppSettings::default()
        };

        let merged = merge_settings_for_save(incoming, &existing);

        assert_eq!(
            merged
                .s3_sync
                .as_ref()
                .map(|v| v.secret_access_key.as_str()),
            Some("secret")
        );
    }

    #[test]
    fn save_settings_should_preserve_local_migrations_when_payload_omits_it() {
        let existing = AppSettings {
            local_migrations: Some(LocalMigrations {
                codex_third_party_history_provider_bucket_v1: Some(
                    CodexThirdPartyHistoryProviderBucketMigration {
                        completed_at: "2026-05-20T00:00:00Z".to_string(),
                        target_provider_id: "custom".to_string(),
                        source_provider_ids: vec!["rightcode".to_string()],
                        migrated_jsonl_files: 2,
                        migrated_state_rows: 3,
                        scanned_history_files: true,
                    },
                ),
                codex_provider_template_v1: Some(CodexProviderTemplateMigration {
                    completed_at: "2026-05-20T00:01:00Z".to_string(),
                    migrated_provider_ids: vec!["legacy".to_string()],
                }),
                codex_official_history_unify_v1: Some(CodexOfficialHistoryUnifyMigration {
                    completed_at: "2026-06-12T00:00:00Z".to_string(),
                    target_provider_id: "custom".to_string(),
                    migrated_jsonl_files: 5,
                    migrated_state_rows: 7,
                    codex_config_dir: None,
                }),
            }),
            ..AppSettings::default()
        };

        let incoming = AppSettings::default();
        let merged = merge_settings_for_save(incoming, &existing);

        let migration = merged
            .local_migrations
            .as_ref()
            .and_then(|migrations| {
                migrations
                    .codex_third_party_history_provider_bucket_v1
                    .as_ref()
            })
            .expect("local migration marker should be preserved");
        assert_eq!(migration.target_provider_id, "custom");
        assert_eq!(migration.migrated_jsonl_files, 2);
        assert_eq!(migration.migrated_state_rows, 3);

        let template_migration = merged
            .local_migrations
            .as_ref()
            .and_then(|migrations| migrations.codex_provider_template_v1.as_ref())
            .expect("template migration marker should be preserved");
        assert_eq!(
            template_migration.migrated_provider_ids,
            vec!["legacy".to_string()]
        );

        let unify_migration = merged
            .local_migrations
            .as_ref()
            .and_then(|migrations| migrations.codex_official_history_unify_v1.as_ref())
            .expect("official unify migration marker should be preserved");
        assert_eq!(unify_migration.migrated_jsonl_files, 5);
        assert_eq!(unify_migration.migrated_state_rows, 7);
    }

    /// incoming 带有 local_migrations（哪怕是空的）也不能覆盖后端维护的标记。
    #[test]
    fn save_settings_should_keep_backend_migration_markers_over_incoming() {
        let existing = AppSettings {
            local_migrations: Some(LocalMigrations {
                codex_third_party_history_provider_bucket_v1: None,
                codex_provider_template_v1: None,
                codex_official_history_unify_v1: Some(CodexOfficialHistoryUnifyMigration {
                    completed_at: "2026-06-12T00:00:00Z".to_string(),
                    target_provider_id: "custom".to_string(),
                    migrated_jsonl_files: 1,
                    migrated_state_rows: 2,
                    codex_config_dir: None,
                }),
            }),
            ..AppSettings::default()
        };

        let incoming = AppSettings {
            local_migrations: Some(LocalMigrations::default()),
            ..AppSettings::default()
        };
        let merged = merge_settings_for_save(incoming, &existing);

        assert!(merged
            .local_migrations
            .as_ref()
            .and_then(|migrations| migrations.codex_official_history_unify_v1.as_ref())
            .is_some());
    }

    /// 后端清掉 marker 后（如关闭统一会话开关）、前端缓存刷新前的全量保存
    /// 会携带旧 marker；merge 必须忽略它，否则被"复活"的标记会让重新开启
    /// 时误判已迁移而漏迁。
    #[test]
    fn save_settings_should_ignore_stale_incoming_migration_markers() {
        let existing = AppSettings::default();

        let incoming = AppSettings {
            local_migrations: Some(LocalMigrations {
                codex_official_history_unify_v1: Some(CodexOfficialHistoryUnifyMigration {
                    completed_at: "2026-06-12T00:00:00Z".to_string(),
                    target_provider_id: "custom".to_string(),
                    migrated_jsonl_files: 1,
                    migrated_state_rows: 2,
                    codex_config_dir: None,
                }),
                ..LocalMigrations::default()
            }),
            ..AppSettings::default()
        };
        let merged = merge_settings_for_save(incoming, &existing);

        assert!(merged.local_migrations.is_none());
    }
}

/// 获取开机自启状态
#[tauri::command]
pub async fn get_auto_launch_status() -> Result<bool, String> {
    crate::auto_launch::is_auto_launch_enabled().map_err(|e| format!("获取开机自启状态失败: {e}"))
}

/// 获取整流器配置
#[tauri::command]
pub async fn get_rectifier_config(
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::proxy::types::RectifierConfig, String> {
    state.db.get_rectifier_config().map_err(|e| e.to_string())
}

/// 设置整流器配置
#[tauri::command]
pub async fn set_rectifier_config(
    state: tauri::State<'_, crate::AppState>,
    config: crate::proxy::types::RectifierConfig,
) -> Result<bool, String> {
    state
        .db
        .set_rectifier_config(&config)
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// 获取优化器配置
#[tauri::command]
pub async fn get_optimizer_config(
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::proxy::types::OptimizerConfig, String> {
    state.db.get_optimizer_config().map_err(|e| e.to_string())
}

/// 设置优化器配置
#[tauri::command]
pub async fn set_optimizer_config(
    state: tauri::State<'_, crate::AppState>,
    config: crate::proxy::types::OptimizerConfig,
) -> Result<bool, String> {
    state
        .db
        .set_optimizer_config(&config)
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// 获取 Copilot 优化器配置
#[tauri::command]
pub async fn get_copilot_optimizer_config(
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::proxy::types::CopilotOptimizerConfig, String> {
    state
        .db
        .get_copilot_optimizer_config()
        .map_err(|e| e.to_string())
}

/// 设置 Copilot 优化器配置
#[tauri::command]
pub async fn set_copilot_optimizer_config(
    state: tauri::State<'_, crate::AppState>,
    config: crate::proxy::types::CopilotOptimizerConfig,
) -> Result<bool, String> {
    state
        .db
        .set_copilot_optimizer_config(&config)
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// 获取日志配置
#[tauri::command]
pub async fn get_log_config(
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::proxy::types::LogConfig, String> {
    state.db.get_log_config().map_err(|e| e.to_string())
}

/// 设置日志配置
#[tauri::command]
pub async fn set_log_config(
    state: tauri::State<'_, crate::AppState>,
    config: crate::proxy::types::LogConfig,
) -> Result<bool, String> {
    state
        .db
        .set_log_config(&config)
        .map_err(|e| e.to_string())?;
    log::set_max_level(config.to_level_filter());
    log::info!(
        "日志配置已更新: enabled={}, level={}",
        config.enabled,
        config.level
    );
    Ok(true)
}

#[cfg(test)]
mod ppbind_update_tests {
    use super::{compare_versions, select_update_asset, GithubReleaseAsset};
    use std::cmp::Ordering;

    #[test]
    fn version_compare_detects_newer_stable_release() {
        assert_eq!(
            compare_versions("v3.20.4", "3.20.3"),
            Some(Ordering::Greater)
        );
        assert_eq!(compare_versions("v3.20.3", "3.20.3"), Some(Ordering::Equal));
        assert_eq!(
            compare_versions("v3.20.3-beta.1", "3.20.3"),
            Some(Ordering::Less)
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn update_asset_prefers_nsis_installer_over_standalone_exe() {
        let assets = vec![
            GithubReleaseAsset {
                name: "ppbind.exe".to_string(),
                browser_download_url: "https://example.com/ppbind.exe".to_string(),
                digest: None,
            },
            GithubReleaseAsset {
                name: "PPBind_3.20.4_x64-setup.exe".to_string(),
                browser_download_url: "https://example.com/setup.exe".to_string(),
                digest: Some("sha256:abc".to_string()),
            },
        ];

        let selected = select_update_asset(&assets).expect("asset");
        assert_eq!(selected.name, "PPBind_3.20.4_x64-setup.exe");
    }
}
