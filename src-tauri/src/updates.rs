use crate::model::Result;
use reqwest::{header, Client, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::AppHandle;

const RELEASES_API_URL: &str =
    "https://api.github.com/repos/guozhiqiang123/AndroidBugDesk/releases?per_page=20";
const RELEASES_PAGE_URL: &str = "https://github.com/guozhiqiang123/AndroidBugDesk/releases";
const RELEASE_DOWNLOAD_PREFIX: &str = "/guozhiqiang123/AndroidBugDesk/releases/download/";
const MAX_UPDATE_BYTES: u64 = 512 * 1024 * 1024;
static UPDATE_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
    draft: bool,
    prerelease: bool,
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<GitHubAsset>,
}

#[derive(Clone, Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    ok: bool,
    current_version: String,
    latest_version: Option<String>,
    has_update: bool,
    release_url: String,
    published_at: Option<String>,
    releases_checked: usize,
    checked_at_epoch_ms: u64,
    asset_name: Option<String>,
    asset_size_bytes: Option<u64>,
    can_auto_update: bool,
    message: String,
    error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInstallResult {
    ok: bool,
    message: String,
    file_path: Option<String>,
    error: Option<String>,
}

#[derive(Debug)]
struct DownloadedUpdateAsset {
    file_path: PathBuf,
}

struct UpdateGuard;

impl UpdateGuard {
    fn acquire() -> std::result::Result<Self, String> {
        UPDATE_IN_PROGRESS
            .compare_exchange(false, true, AtomicOrdering::SeqCst, AtomicOrdering::SeqCst)
            .map(|_| Self)
            .map_err(|_| "更新任务已经在进行中。".to_string())
    }
}

impl Drop for UpdateGuard {
    fn drop(&mut self) {
        UPDATE_IN_PROGRESS.store(false, AtomicOrdering::SeqCst);
    }
}

#[tauri::command]
pub async fn check_for_updates() -> UpdateCheckResult {
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let checked_at_epoch_ms = now_ms();
    match fetch_releases(RELEASES_API_URL).await {
        Ok(releases) => build_update_result(current_version, checked_at_epoch_ms, releases),
        Err(error) => update_error(current_version, checked_at_epoch_ms, error),
    }
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> UpdateInstallResult {
    let guard = match UpdateGuard::acquire() {
        Ok(guard) => guard,
        Err(error) => return install_error(error),
    };
    let releases = match fetch_releases(RELEASES_API_URL).await {
        Ok(releases) => releases,
        Err(error) => return install_error(error),
    };
    let latest = match latest_stable_release(&releases) {
        Some(release) => release,
        None => return install_error("GitHub Release 列表中没有可识别的正式版本".into()),
    };
    if compare_versions(&latest.tag_name, env!("CARGO_PKG_VERSION")) != Ordering::Greater {
        return install_error("当前版本已经是最新正式版本。".into());
    }
    let asset = match selected_auto_update_asset(latest) {
        Some(asset) => asset,
        None => return install_error("新版本没有适配当前系统与架构的可安装文件。".into()),
    };
    let downloaded = match download_and_verify_asset(asset).await {
        Ok(downloaded) => downloaded,
        Err(error) => return install_error(error),
    };
    if let Err(error) = launch_update_installer(&downloaded.file_path) {
        return install_error(error);
    }

    let file_path = downloaded.file_path.to_string_lossy().to_string();
    std::mem::forget(guard);
    let app_for_exit = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(700));
        app_for_exit.exit(0);
    });
    UpdateInstallResult {
        ok: true,
        message: "安装程序已准备好，应用即将退出并完成覆盖升级。".into(),
        file_path: Some(file_path),
        error: None,
    }
}

#[tauri::command]
pub fn open_release_page() -> Result<()> {
    open::that(RELEASES_PAGE_URL).map_err(|_| "无法使用系统浏览器打开 GitHub Release 页面".into())
}

async fn fetch_releases(url: &str) -> Result<Vec<GitHubRelease>> {
    let client = github_client(Duration::from_secs(30))?;
    let response = client
        .get(url)
        .header(header::ACCEPT, "application/vnd.github+json")
        .header(
            header::USER_AGENT,
            format!("AndroidBugDesk/{}", env!("CARGO_PKG_VERSION")),
        )
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|error| format!("无法连接 GitHub：{error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(if status.as_u16() == 403 || status.as_u16() == 429 {
            "GitHub 暂时限制了更新检查频率，请稍后再试".into()
        } else {
            format!("GitHub Release 列表请求失败（HTTP {}）", status.as_u16())
        });
    }
    response
        .json::<Vec<GitHubRelease>>()
        .await
        .map_err(|error| format!("GitHub Release 列表格式无法识别：{error}"))
}

fn github_client(timeout: Duration) -> Result<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|error| format!("无法初始化更新请求：{error}"))
}

fn latest_stable_release(releases: &[GitHubRelease]) -> Option<&GitHubRelease> {
    releases
        .iter()
        .filter(|release| {
            !release.draft && !release.prerelease && parse_version(&release.tag_name).is_some()
        })
        .max_by(|left, right| compare_versions(&left.tag_name, &right.tag_name))
}

fn build_update_result(
    current_version: String,
    checked_at_epoch_ms: u64,
    releases: Vec<GitHubRelease>,
) -> UpdateCheckResult {
    let releases_checked = releases
        .iter()
        .filter(|release| !release.draft && !release.prerelease)
        .count();
    let Some(latest) = latest_stable_release(&releases) else {
        return update_error(
            current_version,
            checked_at_epoch_ms,
            "GitHub Release 列表中没有可识别的正式版本".into(),
        );
    };
    let has_update = compare_versions(&latest.tag_name, &current_version) == Ordering::Greater;
    let latest_version = latest.tag_name.trim_start_matches(['v', 'V']).to_string();
    let asset = selected_auto_update_asset(latest);
    let message = if has_update && asset.is_some() {
        format!("发现新版本 v{latest_version}，可直接下载安装。")
    } else if has_update {
        format!("发现新版本 v{latest_version}，但没有适配当前系统与架构的安装包。")
    } else if compare_versions(&current_version, &latest.tag_name) == Ordering::Greater {
        format!("当前版本 v{current_version} 高于最新正式版本 v{latest_version}。")
    } else {
        format!("当前版本 v{current_version} 已是最新正式版本。")
    };
    UpdateCheckResult {
        ok: true,
        current_version,
        latest_version: Some(latest_version),
        has_update,
        release_url: latest.html_url.clone(),
        published_at: latest.published_at.clone(),
        releases_checked,
        checked_at_epoch_ms,
        asset_name: asset.map(|item| item.name.clone()),
        asset_size_bytes: asset.map(|item| item.size),
        can_auto_update: has_update && asset.is_some(),
        message,
        error: None,
    }
}

fn update_error(
    current_version: String,
    checked_at_epoch_ms: u64,
    error: String,
) -> UpdateCheckResult {
    UpdateCheckResult {
        ok: false,
        current_version,
        latest_version: None,
        has_update: false,
        release_url: RELEASES_PAGE_URL.into(),
        published_at: None,
        releases_checked: 0,
        checked_at_epoch_ms,
        asset_name: None,
        asset_size_bytes: None,
        can_auto_update: false,
        message: "更新检查失败。".into(),
        error: Some(error),
    }
}

fn install_error(error: String) -> UpdateInstallResult {
    UpdateInstallResult {
        ok: false,
        message: "更新失败。".into(),
        file_path: None,
        error: Some(error),
    }
}

fn selected_auto_update_asset(release: &GitHubRelease) -> Option<&GitHubAsset> {
    select_release_asset(&release.assets, env::consts::OS, env::consts::ARCH)
        .filter(|asset| valid_asset_for_install(asset))
}

fn select_release_asset<'a>(
    assets: &'a [GitHubAsset],
    os: &str,
    arch: &str,
) -> Option<&'a GitHubAsset> {
    let suffix = match (os, arch) {
        ("macos", "aarch64") => "-macos-arm64.dmg",
        ("macos", "x86_64") => "-macos-x64.dmg",
        ("windows", "x86_64") => "-windows-x64-setup.exe",
        ("linux", "x86_64") if env::var_os("APPIMAGE").is_some() => "-linux-x64.AppImage",
        ("linux", "x86_64") => "-linux-x64.deb",
        _ => return None,
    };
    assets.iter().find(|asset| asset.name.ends_with(suffix))
}

fn valid_asset_for_install(asset: &GitHubAsset) -> bool {
    asset.size > 0
        && asset.size <= MAX_UPDATE_BYTES
        && expected_sha256(asset).is_some()
        && allowed_download_url(&asset.browser_download_url)
        && sanitize_file_name(&asset.name).as_deref() == Some(asset.name.as_str())
}

fn expected_sha256(asset: &GitHubAsset) -> Option<String> {
    let digest = asset.digest.as_deref()?.strip_prefix("sha256:")?;
    (digest.len() == 64
        && digest
            .chars()
            .all(|character| character.is_ascii_hexdigit()))
    .then(|| digest.to_ascii_lowercase())
}

fn allowed_download_url(url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    parsed.scheme() == "https"
        && parsed.host_str() == Some("github.com")
        && parsed.path().starts_with(RELEASE_DOWNLOAD_PREFIX)
}

async fn download_and_verify_asset(asset: &GitHubAsset) -> Result<DownloadedUpdateAsset> {
    if !valid_asset_for_install(asset) {
        return Err("Release 安装包缺少有效的大小、SHA256 或官方下载地址。".into());
    }
    let client = github_client(Duration::from_secs(900))?;
    let response = client
        .get(&asset.browser_download_url)
        .header(header::ACCEPT, "application/octet-stream")
        .header(
            header::USER_AGENT,
            format!("AndroidBugDesk/{}", env!("CARGO_PKG_VERSION")),
        )
        .send()
        .await
        .map_err(|error| format!("下载安装包失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!("下载安装包失败（HTTP {}）。", response.status()));
    }
    if let Some(length) = response.content_length() {
        if length != asset.size || length > MAX_UPDATE_BYTES {
            return Err("安装包下载大小与 GitHub Release 信息不一致。".into());
        }
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("读取安装包失败：{error}"))?;
    verify_downloaded_bytes(asset, &bytes)?;

    let update_dir = env::temp_dir().join("android-bug-desk-updates");
    cleanup_update_dir(&update_dir);
    fs::create_dir_all(&update_dir).map_err(|error| format!("创建更新目录失败：{error}"))?;
    let destination = update_dir.join(&asset.name);
    let partial = destination.with_extension("download");
    fs::write(&partial, &bytes).map_err(|error| format!("写入安装包失败：{error}"))?;
    fs::rename(&partial, &destination).map_err(|error| format!("保存安装包失败：{error}"))?;
    Ok(DownloadedUpdateAsset {
        file_path: destination,
    })
}

fn verify_downloaded_bytes(asset: &GitHubAsset, bytes: &[u8]) -> Result<()> {
    if bytes.len() as u64 != asset.size || bytes.len() as u64 > MAX_UPDATE_BYTES {
        return Err("安装包实际大小与 GitHub Release 信息不一致。".into());
    }
    let expected_hash = expected_sha256(asset).ok_or("Release 安装包缺少 SHA256。")?;
    let actual_hash = hex::encode(Sha256::digest(bytes));
    if actual_hash != expected_hash {
        return Err("安装包 SHA256 校验失败，已取消更新。".into());
    }
    Ok(())
}

fn cleanup_update_dir(directory: &Path) {
    if let Ok(entries) = fs::read_dir(directory) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                let _ = fs::remove_file(path);
            }
        }
    }
}

fn launch_update_installer(file_path: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        launch_macos_installer(file_path)
    }
    #[cfg(target_os = "windows")]
    {
        launch_windows_installer(file_path)
    }
    #[cfg(target_os = "linux")]
    {
        launch_linux_installer(file_path)
    }
}

#[cfg(target_os = "macos")]
fn launch_macos_installer(file_path: &Path) -> Result<()> {
    if file_path.extension().and_then(|value| value.to_str()) != Some("dmg") {
        return Err("macOS 更新包格式不正确。".into());
    }
    let app_path = current_app_bundle_path()
        .unwrap_or_else(|| PathBuf::from("/Applications/Android Bug Desk.app"));
    let script_path = file_path
        .parent()
        .unwrap_or_else(|| Path::new("/tmp"))
        .join(format!(
            "install-android-bug-desk-{}.sh",
            std::process::id()
        ));
    fs::write(&script_path, macos_install_script(file_path, &app_path))
        .map_err(|error| format!("创建安装脚本失败：{error}"))?;
    Command::new("sh")
        .arg(&script_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("启动安装脚本失败：{error}"))
}

#[cfg(target_os = "macos")]
fn current_app_bundle_path() -> Option<PathBuf> {
    let mut path = env::current_exe().ok()?;
    while path.pop() {
        if path.extension().and_then(|value| value.to_str()) == Some("app") {
            return Some(path);
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn macos_install_script(dmg_path: &Path, app_path: &Path) -> String {
    format!(
        r#"#!/bin/sh
set -eu
DMG_PATH={dmg_path}
APP_PATH={app_path}
SCRIPT_PATH="$0"
MOUNT_DIR="$(mktemp -d "${{TMPDIR:-/tmp/}}android-bug-desk-update.XXXXXX")"
APP_PARENT="$(dirname "$APP_PATH")"
STAGED_APP="$APP_PARENT/.Android Bug Desk.update.app"
BACKUP_APP="$APP_PARENT/.Android Bug Desk.previous.app"
cleanup() {{
  hdiutil detach "$MOUNT_DIR" -quiet >/dev/null 2>&1 || true
  rmdir "$MOUNT_DIR" >/dev/null 2>&1 || true
}}
trap cleanup EXIT
hdiutil attach -nobrowse -quiet -readonly -mountpoint "$MOUNT_DIR" "$DMG_PATH"
SOURCE_APP="$(find "$MOUNT_DIR" -maxdepth 1 -name 'Android Bug Desk.app' -type d | head -n 1)"
[ -n "$SOURCE_APP" ]
codesign --verify --deep --strict "$SOURCE_APP"
rm -rf "$STAGED_APP" "$BACKUP_APP"
ditto "$SOURCE_APP" "$STAGED_APP"
codesign --verify --deep --strict "$STAGED_APP"
while pgrep -x android-bug-desk >/dev/null 2>&1; do sleep 0.3; done
if [ -d "$APP_PATH" ]; then mv "$APP_PATH" "$BACKUP_APP"; fi
if ! mv "$STAGED_APP" "$APP_PATH"; then
  if [ -d "$BACKUP_APP" ]; then mv "$BACKUP_APP" "$APP_PATH"; fi
  exit 1
fi
if ! codesign --verify --deep --strict "$APP_PATH"; then
  rm -rf "$APP_PATH"
  if [ -d "$BACKUP_APP" ]; then mv "$BACKUP_APP" "$APP_PATH"; fi
  exit 1
fi
rm -rf "$BACKUP_APP"
open "$APP_PATH" >/dev/null 2>&1 || true
rm -f "$DMG_PATH" "$SCRIPT_PATH" >/dev/null 2>&1 || true
rmdir "$(dirname "$DMG_PATH")" >/dev/null 2>&1 || true
"#,
        dmg_path = shell_single_quote(&dmg_path.to_string_lossy()),
        app_path = shell_single_quote(&app_path.to_string_lossy()),
    )
}

#[cfg(target_os = "windows")]
fn launch_windows_installer(file_path: &Path) -> Result<()> {
    if file_path.extension().and_then(|value| value.to_str()) != Some("exe") {
        return Err("Windows 更新包格式不正确。".into());
    }
    let app_executable =
        env::current_exe().map_err(|error| format!("无法确定当前应用安装位置：{error}"))?;
    let script_path = env::temp_dir().join(format!(
        "install-android-bug-desk-{}.cmd",
        std::process::id()
    ));
    fs::write(
        &script_path,
        windows_install_script(file_path, &app_executable),
    )
    .map_err(|error| format!("创建安装脚本失败：{error}"))?;
    use std::os::windows::process::CommandExt;
    Command::new("cmd")
        .creation_flags(0x08000000)
        .arg("/C")
        .arg("call")
        .arg(&script_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("启动 Windows 安装脚本失败：{error}"))
}

#[cfg(any(target_os = "windows", test))]
fn windows_install_script(file_path: &Path, app_executable: &Path) -> String {
    format!(
        r#"@echo off
setlocal
set "INSTALLER={installer}"
set "APP_EXE={app_executable}"
:wait_app
tasklist /FI "IMAGENAME eq android-bug-desk.exe" 2>NUL | find /I "android-bug-desk.exe" >NUL
if not errorlevel 1 (
  timeout /T 1 /NOBREAK >NUL
  goto wait_app
)
start "" /WAIT "%INSTALLER%" /S
if exist "%APP_EXE%" start "" "%APP_EXE%"
del /F /Q "%INSTALLER%" >NUL 2>NUL
del /F /Q "%~f0" >NUL 2>NUL
"#,
        installer = file_path.to_string_lossy().replace('%', "%%"),
        app_executable = app_executable.to_string_lossy().replace('%', "%%"),
    )
}

#[cfg(target_os = "linux")]
fn launch_linux_installer(file_path: &Path) -> Result<()> {
    let is_app_image = file_path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("AppImage"));
    let script_path = env::temp_dir().join(format!(
        "install-android-bug-desk-{}.sh",
        std::process::id()
    ));
    let script = if is_app_image {
        let target =
            env::var("APPIMAGE").map_err(|_| "无法确定当前 AppImage 的安装位置。".to_string())?;
        linux_app_image_script(file_path, Path::new(&target))
    } else {
        linux_deb_script(file_path)
    };
    fs::write(&script_path, script).map_err(|error| format!("创建安装脚本失败：{error}"))?;
    Command::new("sh")
        .arg(&script_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("启动 Linux 安装脚本失败：{error}"))
}

#[cfg(target_os = "linux")]
fn linux_app_image_script(file_path: &Path, target: &Path) -> String {
    format!(
        r#"#!/bin/sh
set -eu
PACKAGE={package}
TARGET={target}
SCRIPT_PATH="$0"
while pgrep -x android-bug-desk >/dev/null 2>&1; do sleep 0.3; done
chmod 755 "$PACKAGE"
mv -f "$PACKAGE" "$TARGET"
chmod 755 "$TARGET"
"$TARGET" >/dev/null 2>&1 &
rm -f "$SCRIPT_PATH" >/dev/null 2>&1 || true
"#,
        package = shell_single_quote(&file_path.to_string_lossy()),
        target = shell_single_quote(&target.to_string_lossy()),
    )
}

#[cfg(target_os = "linux")]
fn linux_deb_script(file_path: &Path) -> String {
    format!(
        r#"#!/bin/sh
set -eu
PACKAGE={package}
SCRIPT_PATH="$0"
while pgrep -x android-bug-desk >/dev/null 2>&1; do sleep 0.3; done
pkexec dpkg -i "$PACKAGE"
android-bug-desk >/dev/null 2>&1 &
rm -f "$PACKAGE" "$SCRIPT_PATH" >/dev/null 2>&1 || true
"#,
        package = shell_single_quote(&file_path.to_string_lossy()),
    )
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn sanitize_file_name(name: &str) -> Option<String> {
    let sanitized = name
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(*character, '.' | '-' | '_')
        })
        .collect::<String>();
    (!sanitized.is_empty()).then_some(sanitized)
}

fn parse_version(value: &str) -> Option<Vec<u64>> {
    let normalized = value.trim().trim_start_matches(['v', 'V']);
    let core = normalized.split(['-', '+']).next()?;
    let parts = core
        .split('.')
        .map(str::parse::<u64>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .ok()?;
    (parts.len() >= 2 && parts.len() <= 4).then_some(parts)
}

fn compare_versions(left: &str, right: &str) -> Ordering {
    let Some(left) = parse_version(left) else {
        return Ordering::Less;
    };
    let Some(right) = parse_version(right) else {
        return Ordering::Greater;
    };
    let length = left.len().max(right.len());
    (0..length)
        .map(|index| {
            left.get(index)
                .copied()
                .unwrap_or(0)
                .cmp(&right.get(index).copied().unwrap_or(0))
        })
        .find(|ordering| *ordering != Ordering::Equal)
        .unwrap_or(Ordering::Equal)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        matchers::{header, method, path},
        Mock, MockServer, ResponseTemplate,
    };

    fn asset(name: &str) -> GitHubAsset {
        GitHubAsset {
            name: name.into(),
            browser_download_url: format!(
                "https://github.com/guozhiqiang123/AndroidBugDesk/releases/download/v0.2.0/{name}"
            ),
            size: 1024,
            digest: Some(format!("sha256:{}", "a".repeat(64))),
        }
    }

    fn release(tag: &str, draft: bool, prerelease: bool) -> GitHubRelease {
        GitHubRelease {
            tag_name: tag.into(),
            html_url: format!(
                "https://github.com/guozhiqiang123/AndroidBugDesk/releases/tag/{tag}"
            ),
            draft,
            prerelease,
            published_at: Some("2026-10-12T00:00:00Z".into()),
            assets: vec![],
        }
    }

    #[test]
    fn numeric_versions_are_compared_without_lexical_errors() {
        assert_eq!(compare_versions("v0.10.0", "0.9.9"), Ordering::Greater);
        assert_eq!(compare_versions("1.2", "v1.2.0"), Ordering::Equal);
        assert_eq!(compare_versions("v1.2.3", "v2.0.0"), Ordering::Less);
    }

    #[test]
    fn latest_stable_release_ignores_drafts_prereleases_and_invalid_tags() {
        let result = build_update_result(
            "0.1.0".into(),
            123,
            vec![
                release("v9.0.0", true, false),
                release("v2.0.0-beta.1", false, true),
                release("nightly", false, false),
                release("v0.2.0", false, false),
                release("v0.1.5", false, false),
            ],
        );
        assert!(result.ok);
        assert!(result.has_update);
        assert_eq!(result.latest_version.as_deref(), Some("0.2.0"));
        assert_eq!(result.releases_checked, 3);
    }

    #[test]
    fn platform_selection_never_falls_back_to_the_wrong_architecture() {
        let assets = vec![
            asset("Android-Bug-Desk-0.2.0-macos-arm64.dmg"),
            asset("Android-Bug-Desk-0.2.0-macos-x64.dmg"),
            asset("Android-Bug-Desk-0.2.0-windows-x64-setup.exe"),
        ];
        assert!(select_release_asset(&assets, "macos", "aarch64")
            .unwrap()
            .name
            .ends_with("macos-arm64.dmg"));
        assert!(select_release_asset(&assets, "macos", "x86_64")
            .unwrap()
            .name
            .ends_with("macos-x64.dmg"));
        assert!(select_release_asset(&assets, "windows", "aarch64").is_none());
    }

    #[test]
    fn download_url_is_restricted_to_this_repository() {
        assert!(allowed_download_url(
            "https://github.com/guozhiqiang123/AndroidBugDesk/releases/download/v0.2.0/app.dmg"
        ));
        assert!(!allowed_download_url(
            "https://github.com/other/AndroidBugDesk/releases/download/v0.2.0/app.dmg"
        ));
        assert!(!allowed_download_url("https://example.com/app.dmg"));
    }

    #[test]
    fn downloaded_bytes_must_match_release_size_and_sha256() {
        let body = b"verified release package";
        let mut release_asset = asset("Android-Bug-Desk-0.2.0-macos-arm64.dmg");
        release_asset.size = body.len() as u64;
        release_asset.digest = Some(format!("sha256:{}", hex::encode(Sha256::digest(body))));
        assert!(verify_downloaded_bytes(&release_asset, body).is_ok());

        release_asset.digest = Some(format!("sha256:{}", "0".repeat(64)));
        assert_eq!(
            verify_downloaded_bytes(&release_asset, body).unwrap_err(),
            "安装包 SHA256 校验失败，已取消更新。"
        );
        release_asset.size += 1;
        assert_eq!(
            verify_downloaded_bytes(&release_asset, body).unwrap_err(),
            "安装包实际大小与 GitHub Release 信息不一致。"
        );
    }

    #[test]
    fn windows_script_relaunches_the_actual_install_path_only_when_it_exists() {
        let script = windows_install_script(
            Path::new(r"C:\Temp\Android-Bug-Desk-0.2.4-windows-x64-setup.exe"),
            Path::new(r"D:\QA Tools\Android Bug Desk\android-bug-desk.exe"),
        );
        assert!(
            script.contains(r#"set "APP_EXE=D:\QA Tools\Android Bug Desk\android-bug-desk.exe""#)
        );
        assert!(script.contains(r#"if exist "%APP_EXE%" start "" "%APP_EXE%""#));
        assert!(!script.contains(r"%LOCALAPPDATA%\Android Bug Desk"));
    }

    #[tokio::test]
    async fn release_list_request_uses_the_github_json_contract() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/releases"))
            .and(header("accept", "application/vnd.github+json"))
            .and(header("x-github-api-version", "2022-11-28"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([{
                "tag_name": "v0.2.0",
                "html_url": "https://github.com/guozhiqiang123/AndroidBugDesk/releases/tag/v0.2.0",
                "draft": false,
                "prerelease": false,
                "published_at": "2026-10-12T00:00:00Z",
                "assets": []
            }])))
            .mount(&server)
            .await;
        let releases = fetch_releases(&format!("{}/releases", server.uri()))
            .await
            .unwrap();
        assert_eq!(releases[0].tag_name, "v0.2.0");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_script_waits_verifies_and_relaunches() {
        let script = macos_install_script(
            Path::new("/tmp/update.dmg"),
            Path::new("/Applications/Android Bug Desk.app"),
        );
        assert!(script.contains("codesign --verify --deep --strict"));
        assert!(script.contains("pgrep -x android-bug-desk"));
        assert!(script.contains("ditto \"$SOURCE_APP\" \"$STAGED_APP\""));
        assert!(script.contains("mv \"$BACKUP_APP\" \"$APP_PATH\""));
        assert!(script.contains("open \"$APP_PATH\""));
    }
}
