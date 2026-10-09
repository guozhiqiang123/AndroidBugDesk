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
