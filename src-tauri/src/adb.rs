use crate::{model::*, store::Store};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
};
use tokio::{io::AsyncReadExt, process::Command, sync::watch};
pub fn valid_serial(s: &str) -> Result<()> {
    if regex::Regex::new(r"^[A-Za-z0-9._-]{1,128}$")
        .unwrap()
        .is_match(s)
    {
        Ok(())
    } else {
        Err("设备序列号格式不正确；首版仅支持 USB 设备".into())
    }
}
pub fn valid_package(s: &str) -> Result<()> {
    if regex::Regex::new(r"^[A-Za-z][\w]*(\.[A-Za-z][\w]*)+$")
        .unwrap()
        .is_match(s)
    {
        Ok(())
    } else {
        Err("包名格式不正确".into())
    }
}
fn command(path: &Path) -> Command {
    let mut c = Command::new(path);
    c.kill_on_drop(true);
    #[cfg(windows)]
    c.creation_flags(0x08000000);
    c
}
pub fn discover() -> Option<PathBuf> {
    let mut paths = Vec::new();
    for name in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(s) = std::env::var_os(name) {
            paths.push(PathBuf::from(s).join("platform-tools"));
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        paths.push(PathBuf::from(&home).join("Library/Android/sdk/platform-tools"));
        paths.push(PathBuf::from(home).join("Android/Sdk/platform-tools"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        paths.push(PathBuf::from(local).join("Android/Sdk/platform-tools"));
    }
    if let Some(path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&path));
    }
    paths
        .into_iter()
        .map(|p| p.join(if cfg!(windows) { "adb.exe" } else { "adb" }))
        .find(|p| p.is_file())
}
pub async fn output(
    path: &Path,
    serial: Option<&str>,
    args: &[&str],
    timeout: u64,
) -> Result<Vec<u8>> {
    let mut c = command(path);
    if let Some(s) = serial {
        valid_serial(s)?;
        c.args(["-s", s]);
    }
    c.args(args);
    let o = tokio::time::timeout(std::time::Duration::from_secs(timeout), c.output())
        .await
        .map_err(|_| "ADB 采集超时")?
        .map_err(|_| "无法启动 ADB，请检查 Platform Tools 路径")?;
    if !o.status.success() {
        return Err(format!(
            "ADB 采集失败：{}",
            String::from_utf8_lossy(&o.stderr)
                .chars()
                .take(300)
                .collect::<String>()
        ));
    }
    if o.stdout.len() > 32_000_000 {
        return Err("ADB 输出过大，请使用系统报告导出".into());
    }
    Ok(o.stdout)
}
async fn text(path: &Path, serial: &str, args: &[&str]) -> Result<String> {
    Ok(
        String::from_utf8_lossy(&output(path, Some(serial), args, 15).await?)
            .trim()
            .into(),
    )
}
pub async fn devices(path: &Path) -> Result<Value> {
    let data = output(path, None, &["devices", "-l"], 15).await?;
    Ok(json!(String::from_utf8_lossy(&data)
        .lines()
        .skip(1)
        .filter_map(|s| {
            let mut parts = s.split_whitespace();
            let serial = parts.next()?;
            let state = parts.next()?;
            Some(json!({"serial":serial,"state":state,"label":s}))
        })
        .collect::<Vec<_>>()))
}
pub fn parse_memory(s: &str) -> (String, String) {
    let get = |key: &str| {
        s.lines()
            .find_map(|line| {
                line.strip_prefix(key)
                    .and_then(|v| v.split_whitespace().next())
                    .map(str::to_string)
            })
            .unwrap_or_default()
    };
    (get("MemTotal:"), get("MemAvailable:"))
}
pub fn parse_screen(s: &str) -> String {
    let mut value = String::new();
    for line in s.lines() {
        if let Some((_, v)) = line.split_once(':') {
            if line.starts_with("Physical") || line.starts_with("Override") {
                value = v.trim().to_string();
            }
        }
    }
    value
}
pub fn parse_app(s: &str) -> Value {
    let field = |re: &str| {
        regex::Regex::new(re)
            .unwrap()
            .captures(s)
            .and_then(|c| c.get(1).map(|s| s.as_str().to_string()))
            .unwrap_or_default()
    };
    json!({"versionName":field(r"(?m)^\s*versionName=(\S+)"),"versionCode":field(r"\bversionCode=(\d+)"),"firstInstallTime":field(r"firstInstallTime=(.+)"),"lastUpdateTime":field(r"lastUpdateTime=(.+)")})
}
pub async fn snapshot(path: &Path, serial: &str, package: &str) -> Result<Value> {
    valid_serial(serial)?;
    valid_package(package)?;
    let at = now();
    let mut device = json!({"model":"","androidVersion":"","totalMemKb":"","availableMemKb":"","resolution":"","density":""});
    let mut app = json!({"packageName":package,"versionName":"","versionCode":""});
    let mut sources = json!({});
    let mut errors = Vec::new();
    let mut details = json!({"deviceAlias":format!("device-{}",&hex::encode(Sha256::digest(serial.as_bytes()))[..12])});
    for (key, args) in [
        ("model", vec!["shell", "getprop", "ro.product.model"]),
        (
            "androidVersion",
            vec!["shell", "getprop", "ro.build.version.release"],
        ),
        ("memory", vec!["shell", "cat", "/proc/meminfo"]),
        ("resolution", vec!["shell", "wm", "size"]),
        ("density", vec!["shell", "wm", "density"]),
    ] {
        match text(path, serial, &args).await {
            Ok(raw) => {
                details[key] = json!(raw);
                if key == "memory" {
                    let (a, b) = parse_memory(&raw);
                    device["totalMemKb"] = json!(a);
                    device["availableMemKb"] = json!(b);
                } else {
                    device[key] = json!(if ["resolution", "density"].contains(&key) {
                        parse_screen(&raw)
                    } else {
                        raw
                    });
                }
            }
            Err(e) => errors.push(format!("{key}: {e}")),
        }
    }
    let user = text(path, serial, &["shell", "am", "get-current-user"])
        .await
        .ok()
        .filter(|v| v.chars().all(|c| c.is_ascii_digit()) && !v.is_empty());
    if let Some(user) = user {
        details["androidUser"] = json!(user);
        let installed = text(
            path,
            serial,
            &["shell", "pm", "list", "packages", "--user", &user, package],
        )
        .await?;
        if !installed.lines().any(|v| v == format!("package:{package}")) {
            return Err("目标包未安装在当前 Android 用户中".into());
        }
        match text(path, serial, &["shell", "dumpsys", "package", package]).await {
            Ok(v) => {
                let parsed = parse_app(&v);
                app["versionName"] = parsed["versionName"].clone();
                app["versionCode"] = parsed["versionCode"].clone();
                details["app"] = parsed;
            }
            Err(e) => errors.push(e),
        }
    } else {
        errors.push("无法确定 Android 当前用户，应用身份需人工补齐".into());
    }
    for (key, property) in [
        ("manufacturer", "ro.product.manufacturer"),
        ("api", "ro.build.version.sdk"),
        ("fingerprint", "ro.build.fingerprint"),
        ("abi", "ro.product.cpu.abilist"),
    ] {
        match text(path, serial, &["shell", "getprop", property]).await {
            Ok(v) => details[key] = json!(v),
            Err(e) => errors.push(format!("{key}: {e}")),
        }
    }
    for (key, args) in [
        ("storage", vec!["shell", "df", "-k", "/data"]),
        ("battery", vec!["shell", "dumpsys", "battery"]),
        (
            "fontScale",
            vec!["shell", "settings", "get", "system", "font_scale"],
        ),
    ] {
        match text(path, serial, &args).await {
            Ok(v) => details[key] = json!(v),
            Err(e) => errors.push(format!("{key}: {e}")),
        }
    }
    let device_time = text(path, serial, &["shell", "date", "+%Y-%m-%dT%H:%M:%S%z"])
        .await
        .ok();
    details["clockEstimate"] = json!({"deviceTime":device_time,"hostAt":now(),"precision":"sequential ADB estimate; not synchronized"});
    for (group, obj) in [("device", &device), ("app", &app)] {
        for (key, v) in obj.as_object().unwrap() {
            let collected = v.as_str().is_some_and(|s| !s.is_empty());
            sources[format!("{group}.{key}")] = json!({"status":if collected{"collected"}else{"failed"},"at":at,"detail":if collected{"ADB 自动采集"}else{"采集缺失，请人工补充来源"}});
            if !collected {
                errors.push(format!("{group}.{key} 未采集到有效值"));
            }
        }
    }
    Ok(
        json!({"device":device,"app":app,"sources":sources,"errors":errors,"details":details,"deviceTime":device_time,"capturedAt":at}),
    )
}
pub async fn screenshot(path: &Path, serial: &str, dest: &Path) -> Result<()> {
    let data = output(path, Some(serial), &["exec-out", "screencap", "-p"], 20).await?;
    if !data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("设备截图失败或屏幕限制，需导入已有图片".into());
    }
    tokio::fs::write(dest, data)
        .await
        .map_err(|e| e.to_string())
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BugreportCapture {
    pub serial: String,
    pub started_at: String,
    pub finished_at: String,
    pub timed_out: bool,
    pub exit_code: Option<i32>,
    pub failure: Option<String>,
    pub attachment: Option<Attachment>,
    pub directory: PathBuf,
}

/// Rust-only entry point. The caller supplies the trusted store root, never an IPC output path.
/// A failed command may leave partial files; they are retained, but never reported as an attachment.
pub async fn bugreport(
    path: &Path,
    serial: &str,
    root: &Path,
    timeout_seconds: u64,
) -> Result<BugreportCapture> {
    if !(1..=600).contains(&timeout_seconds) {
        return Err("系统报告超时必须在 1 到 600 秒之间".into());
    }
    capture_bugreport(
        path,
        serial,
        root,
        std::time::Duration::from_secs(timeout_seconds),
    )
    .await
}

async fn capture_bugreport(
    path: &Path,
    serial: &str,
    root: &Path,
    timeout: std::time::Duration,
) -> Result<BugreportCapture> {
    valid_serial(serial)?;
    // A leading dash is not a device identity, even if it passes the legacy serial validator.
    if serial.starts_with('-') {
        return Err("设备序列号不能以选项前缀开头".into());
    }
    let directory = crate::evidence::derived_directory(root, "bugreports")?;
    let stdout = std::fs::File::create(directory.join("stdout.log")).map_err(|e| e.to_string())?;
    let stderr = std::fs::File::create(directory.join("stderr.log")).map_err(|e| e.to_string())?;
    let mut result = BugreportCapture {
        serial: serial.into(),
        started_at: now(),
        finished_at: String::new(),
        timed_out: false,
        exit_code: None,
        failure: None,
        attachment: None,
        directory,
    };
    // argv only; no shell, root, package/PID filter, or fallback to another device.
    let child = command(path)
        .args(["-s", serial, "bugreport"])
        .arg(&result.directory)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn();
    match child {
        Err(e) => result.failure = Some(format!("ADB 系统报告启动失败：{e}")),
        Ok(mut child) => match tokio::time::timeout(timeout, child.wait()).await {
            Ok(Ok(status)) => {
                result.exit_code = status.code();
                if !status.success() {
                    result.failure = Some(format!("ADB 系统报告退出失败：{status}"));
                }
            }
            Ok(Err(e)) => result.failure = Some(format!("ADB 系统报告进程状态读取失败：{e}")),
            Err(_) => {
                result.timed_out = true;
                result.failure = Some("ADB 系统报告超时；部分输出不代表采集成功".into());
                // Reap only the exact child we started. kill_on_drop also protects cancellation.
                let _ = child.kill().await;
                if let Ok(status) = child.wait().await {
                    result.exit_code = status.code();
                }
            }
        },
    }
    if result.failure.is_none() {
        let report = (|| -> Result<Attachment> {
            let mut candidates = Vec::new();
            for entry in std::fs::read_dir(&result.directory).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                if entry
                    .path()
                    .extension()
                    .is_some_and(|v| v.eq_ignore_ascii_case("zip"))
                {
                    if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
                        return Err("系统报告不是本地普通文件".into());
                    }
                    candidates.push(entry.path());
                }
            }
            if candidates.len() != 1 {
                return Err("ADB 未生成唯一 ZIP 系统报告；需人工补充".into());
            }
            crate::evidence::inspect(&candidates[0], "report", "ADB 按需系统报告；未脱敏", root)
        })();
        match report {
            Ok(a) => result.attachment = Some(a),
            Err(e) => result.failure = Some(e),
        }
    }
    result.finished_at = now();
    let receipt = serde_json::to_vec_pretty(&result).map_err(|e| e.to_string())?;
    std::fs::write(result.directory.join("receipt.json"), receipt).map_err(|e| e.to_string())?;
    Ok(result)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProblemWindow {
    pub marked_at: String,
    pub due_at: String,
}

impl ProblemWindow {
    pub fn new(marked_at: &str) -> Result<Self> {
        let mark =
            chrono::DateTime::parse_from_rfc3339(marked_at).map_err(|_| "问题标记时间无效")?;
        let due = mark
            .checked_add_signed(chrono::Duration::seconds(30))
            .ok_or("问题时间超出范围")?;
        Ok(Self {
            marked_at: marked_at.into(),
            due_at: due.to_rfc3339(),
        })
    }

    /// Foreground wait only. Persist this window before awaiting and reconcile interrupted jobs on restart.
    /// Ready means the host deadline elapsed; evidence coverage must still be checked separately.
    pub async fn wait_ready(&self, mut cancelled: watch::Receiver<bool>) -> Result<WindowWait> {
        let validated = Self::new(&self.marked_at)?;
        if chrono::DateTime::parse_from_rfc3339(&validated.due_at).ok()
            != chrono::DateTime::parse_from_rfc3339(&self.due_at).ok()
        {
            return Err("日志固定截止时间必须是标记后 30 秒".into());
        }
        if *cancelled.borrow() || cancelled.has_changed().is_err() {
            return Ok(WindowWait::Interrupted);
        }
        let due = chrono::DateTime::parse_from_rfc3339(&self.due_at).map_err(|_| "截止时间无效")?;
        let delay = (due.with_timezone(&chrono::Utc) - chrono::Utc::now())
            .to_std()
            .unwrap_or_default();
        tokio::select! {
            _ = tokio::time::sleep(delay) => Ok(WindowWait::Ready),
            _ = async { loop {
                if cancelled.changed().await.is_err() || *cancelled.borrow() { break; }
            }} => Ok(WindowWait::Interrupted),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum WindowWait {
    Ready,
    Interrupted,
}
pub struct Collector {
    pub id: String,
    pub stop: watch::Sender<bool>,
    pub task: tauri::async_runtime::JoinHandle<()>,
}
pub struct BoundaryDeduper {
    existing: std::collections::HashSet<Vec<u8>>,
    pending: Vec<u8>,
    active: bool,
}
impl BoundaryDeduper {
    pub fn new(initial: &[u8]) -> Self {
        Self {
            existing: initial
                .split(|b| *b == b'\n')
                .filter(|line| !line.is_empty())
                .map(|line| line.to_vec())
                .collect(),
            pending: Vec::new(),
            active: true,
        }
    }
    pub fn push(&mut self, bytes: &[u8]) -> Vec<u8> {
        if !self.active {
            return bytes.to_vec();
        }
        self.pending.extend_from_slice(bytes);
        let mut output = Vec::new();
        while let Some(end) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            if !self.existing.contains(&line[..line.len() - 1]) {
                self.active = false;
                self.existing.clear();
                output.extend(line);
                output.append(&mut self.pending);
                return output;
            }
        }
        if self.pending.len() > 65536 {
            self.active = false;
            self.existing.clear();
            output.append(&mut self.pending);
        }
        output
    }
    pub fn finish(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.pending)
    }
}
pub async fn start_logs(
    path: PathBuf,
    serial: String,
    store: Store,
    mut session: Value,
) -> Result<Collector> {
    let id = string(&session, "id");
    let dir = store.root.join("sessions").join(&id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // Preserve existing buffers before starting; never clear logs and never filter raw evidence to one PID.
    let initial = output(
        &path,
        Some(&serial),
        &[
            "logcat", "-d", "-v", "epoch", "-b", "main", "-b", "system", "-b", "crash",
        ],
        20,
    )
    .await?;
    let initial_path = dir.join("0000-existing.log");
    std::fs::write(&initial_path, &initial).map_err(|e| e.to_string())?;
    session["logFiles"] = json!([initial_path]);
    session["gaps"] = json!(["初始缓冲区覆盖时间尚未逐行解析；持续采集起点以会话开始时间为准"]);
    store.put("session", &id, &session)?;
    let mut child = command(&path)
        .args([
            "-s", &serial, "logcat", "-v", "epoch", "-b", "main", "-b", "system", "-b", "crash",
            "-T", "1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "无法启动持续日志")?;
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let (stderr_stop, mut stderr_rx) = watch::channel(false);
    let stderr_task = tauri::async_runtime::spawn(async move {
        let mut buffer = [0u8; 2048];
        let mut last = String::new();
        loop {
            tokio::select! {_ = stderr_rx.changed()=>break, r=stderr.read(&mut buffer)=>{match r{Ok(0)|Err(_)=>break,Ok(n)=>last=String::from_utf8_lossy(&buffer[..n]).chars().take(500).collect()}}}
        }
        last
    });
    let (tx, mut rx) = watch::channel(false);
    let session_id = id.clone();
    let mut deduper = BoundaryDeduper::new(&initial);
    let task = tauri::async_runtime::spawn(async move {
        use std::io::Write;
        let mut index = 1;
        let mut bytes = 0u64;
        let mut total = 0u64;
        let mut buffer = [0u8; 16384];
        let mut file: Option<std::fs::File> = None;
        let mut reason = String::new();
        loop {
            if file.is_none() {
                let p = dir.join(format!("{index:04}-live.log"));
                match std::fs::File::create(&p) {
                    Ok(f) => {
                        file = Some(f);
                        session["logFiles"].as_array_mut().unwrap().push(json!(p));
                        let _ = store.put("session", &session_id, &session);
                    }
                    Err(_) => {
                        reason = "日志文件创建失败，可能磁盘不足".into();
                        break;
                    }
                }
            }
            tokio::select! {
                _=rx.changed()=>break,
                r=stdout.read(&mut buffer)=>{match r {
                    Ok(0)=>{reason="ADB 日志流断开；没有切换到其他设备".into();break;},
                    Ok(n)=>{
                        let clean = deduper.push(&buffer[..n]);
                        if file.as_mut().unwrap().write_all(&clean).is_err(){reason="日志写入失败，请检查剩余磁盘".into();break;}
                        bytes+=clean.len() as u64;total+=clean.len() as u64;
                        if bytes>=5_000_000 {if let Some(f)=file.take(){let _=f.sync_all();}index+=1;bytes=0;}
                        if total>=250_000_000{reason="会话日志达到 250 MB 本地上限，已停止并保留全部问题证据".into();break;}
                    },
                    Err(_)=>{reason="日志读取中断".into();break;}
                }}
            }
        }
        let _ = child.kill().await;
        let _ = child.wait().await;
        let _ = stderr_stop.send(true);
        let stderr = stderr_task.await.unwrap_or_default();
        if let Some(mut f) = file.take() {
            let _ = f.write_all(&deduper.finish());
            let _ = f.sync_all();
        }
        if !stderr.is_empty() {
            session["gaps"].as_array_mut().unwrap().push(json!(stderr));
        }
        if !reason.is_empty() {
            session["gaps"]
                .as_array_mut()
                .unwrap()
                .push(json!(format!("{}: {reason}", now())));
        }
        session["stoppedAt"] = json!(now());
        let _ = store.put("session", &session_id, &session);
    });
    Ok(Collector { id, stop: tx, task })
}
pub async fn stop_logs(c: Collector) {
    let _ = c.stop.send(true);
    let _ = c.task.await;
}

#[cfg(test)]
mod gap_tests {
    use super::*;

    #[tokio::test]
    async fn delayed_pin_requires_deadline_and_live_owner() {
        let past = (chrono::Utc::now() - chrono::Duration::seconds(31)).to_rfc3339();
        let window = ProblemWindow::new(&past).unwrap();
        let (tx, rx) = watch::channel(false);
        assert_eq!(window.wait_ready(rx).await.unwrap(), WindowWait::Ready);
        let window = ProblemWindow::new(&now()).unwrap();
        let rx = tx.subscribe();
        tx.send(true).unwrap();
        assert_eq!(
            window.wait_ready(rx).await.unwrap(),
            WindowWait::Interrupted
        );
        let (tx, rx) = watch::channel(false);
        drop(tx);
        assert_eq!(
            window.wait_ready(rx).await.unwrap(),
            WindowWait::Interrupted
        );
        let (_, rx) = watch::channel(false);
        let mut invalid = window;
        invalid.due_at = past;
        assert!(invalid.wait_ready(rx).await.is_err());
    }

    #[tokio::test]
    async fn delayed_pin_can_be_cancelled_while_waiting() {
        let window = ProblemWindow::new(&now()).unwrap();
        let (tx, rx) = watch::channel(false);
        let signal = async {
            tokio::task::yield_now().await;
            tx.send(true).unwrap();
        };
        let (result, ()) = tokio::join!(window.wait_ready(rx), signal);
        assert_eq!(result.unwrap(), WindowWait::Interrupted);
    }

    #[cfg(unix)]
    fn fake_adb(root: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = root.join("fake adb");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[cfg(unix)]
    fn quoted(path: &Path) -> String {
        format!("'{}'", path.to_str().unwrap().replace('\'', "'\\''"))
    }

    #[cfg(unix)]
    fn fixture_zip(root: &Path) -> PathBuf {
        use std::io::Write;
        let path = root.join("fixture.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.start_file("bugreport.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"fixture report: no device was contacted")
            .unwrap();
        zip.finish().unwrap();
        path
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bugreport_pins_serial_and_safe_directory_with_valid_zip_hash() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("evidence root ; literal");
        std::fs::create_dir(&root).unwrap();
        let fixture = fixture_zip(&root);
        let trace = root.join("arguments.txt");
        let adb = fake_adb(
            &root,
            &format!(
                "printf '%s\\n' \"$@\" > {}\n/bin/cp {} \"$4/report.zip\"",
                quoted(&trace),
                quoted(&fixture)
            ),
        );
        for serial in ["USB-A", "USB-B"] {
            let result = bugreport(&adb, serial, &root, 5).await.unwrap();
            assert_eq!(result.serial, serial);
            assert_eq!(result.exit_code, Some(0));
            assert!(!result.timed_out);
            assert!(result.failure.is_none());
            let a = result.attachment.unwrap();
            assert_eq!(a.sha256, crate::evidence::hash_file(&fixture).unwrap().1);
            assert!(!a.confirmed);
            assert!(Path::new(&a.path).starts_with(root.canonicalize().unwrap()));
            let args = std::fs::read_to_string(&trace).unwrap();
            assert_eq!(
                args.lines().take(3).collect::<Vec<_>>(),
                ["-s", serial, "bugreport"]
            );
            assert_eq!(
                args.lines().nth(3).unwrap(),
                result.directory.to_str().unwrap()
            );
            assert_eq!(args.lines().count(), 4);
            let receipt: Value = serde_json::from_slice(
                &std::fs::read(result.directory.join("receipt.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(receipt["attachment"]["sha256"], a.sha256);
            assert!(
                receipt["finishedAt"].as_str().unwrap() >= receipt["startedAt"].as_str().unwrap()
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bugreport_failures_never_accept_partial_or_invalid_output() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = fixture_zip(temp.path());
        let bodies = [
            format!("/bin/cp {} \"$4/report.zip\"\nexit 7", quoted(&fixture)),
            "exit 0".into(),
            "printf invalid > \"$4/report.zip\"".into(),
            format!("/bin/ln -s {} \"$4/report.zip\"", quoted(&fixture)),
            format!(
                "/bin/cp {} \"$4/a.zip\"\n/bin/cp {} \"$4/b.zip\"",
                quoted(&fixture),
                quoted(&fixture)
            ),
        ];
        for body in bodies {
            let adb = fake_adb(temp.path(), &body);
            let result = bugreport(&adb, "USB-A", temp.path(), 5).await.unwrap();
            assert!(result.failure.is_some());
            assert!(result.attachment.is_none());
            assert!(!result.timed_out);
            assert!(result.directory.join("receipt.json").is_file());
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bugreport_timeout_reaps_the_exact_child_and_keeps_diagnostics() {
        let temp = tempfile::tempdir().unwrap();
        let pid_path = temp.path().join("pid");
        let adb = fake_adb(temp.path(), &format!("printf '%s' \"$$\" > {}\nprintf 'partial' > \"$4/partial.txt\"\nexec /bin/sleep 10", quoted(&pid_path)));
        let before = std::time::Instant::now();
        let result = capture_bugreport(
            &adb,
            "USB-A",
            temp.path(),
            std::time::Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert!(before.elapsed() < std::time::Duration::from_secs(5));
        assert!(result.timed_out);
        assert!(result.failure.is_some());
        assert!(result.attachment.is_none());
        assert!(result.directory.join("partial.txt").is_file());
        let pid = std::fs::read_to_string(pid_path).unwrap();
        assert!(!std::process::Command::new("/bin/kill")
            .args(["-0", &pid])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success());
    }

    #[tokio::test]
    async fn bugreport_rejects_injection_and_invalid_timeout_before_spawning() {
        let temp = tempfile::tempdir().unwrap();
        for serial in [
            "",
            "-d",
            "USB;touch pwned",
            "A\nB",
            "A B",
            "../../dev",
            "a:5555",
        ] {
            assert!(bugreport(Path::new("missing-adb"), serial, temp.path(), 1)
                .await
                .is_err());
        }
        for timeout in [0, 601] {
            assert!(
                bugreport(Path::new("missing-adb"), "USB-A", temp.path(), timeout)
                    .await
                    .is_err()
            );
        }
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
        let result = bugreport(&temp.path().join("missing-adb"), "USB-A", temp.path(), 1)
            .await
            .unwrap();
        assert!(result.failure.unwrap().contains("启动失败"));
        assert!(result.exit_code.is_none());
    }
}
