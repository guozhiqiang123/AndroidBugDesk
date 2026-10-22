use crate::{
    agent_service,
    commands::AppState,
    model::{now, DeveloperOptions, Result},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{path::PathBuf, sync::Mutex, time::Duration};
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

const DESCRIPTOR_NAME: &str = "agent-gateway.json";
const MAX_REQUEST_BYTES: u64 = 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayDescriptor {
    pub version: u8,
    pub transport: String,
    pub endpoint: String,
    pub token: String,
    pub pid: u32,
    pub started_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GatewayRequest {
    version: u8,
    request_id: String,
    token: String,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GatewayResponse {
    version: u8,
    request_id: String,
    result: Option<Value>,
    error: Option<String>,
}

struct RunningGateway {
    stop: tokio::sync::watch::Sender<bool>,
    descriptor: GatewayDescriptor,
}

pub struct AgentGateway {
    root: PathBuf,
    running: Mutex<Option<RunningGateway>>,
}

impl AgentGateway {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            running: Mutex::new(None),
        }
    }

    pub fn status(&self) -> Option<GatewayDescriptor> {
        self.running
            .lock()
            .unwrap()
            .as_ref()
            .map(|running| running.descriptor.clone())
    }

    // Keep startup inside a polled async future: Tokio's platform listeners require an active
    // reactor even though their constructors are synchronous.
    pub async fn start<R: tauri::Runtime>(&self, app: AppHandle<R>) -> Result<GatewayDescriptor> {
        let mut running = self.running.lock().unwrap();
        if let Some(current) = running.as_ref() {
            return Ok(current.descriptor.clone());
        }
        remove_stale_files(&self.root);
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let (stop, receiver) = tokio::sync::watch::channel(false);

        #[cfg(unix)]
        let (descriptor, task) = {
            use std::os::unix::fs::PermissionsExt;
            let endpoint = self.root.join("agent-gateway.sock");
            let listener = std::os::unix::net::UnixListener::bind(&endpoint)
                .map_err(|error| format!("无法启动 AI 本地接口：{error}"))?;
            listener
                .set_nonblocking(true)
                .map_err(|error| format!("无法配置 AI 本地接口：{error}"))?;
            std::fs::set_permissions(&endpoint, std::fs::Permissions::from_mode(0o600))
                .map_err(|error| format!("无法保护 AI 本地接口：{error}"))?;
            let listener = tokio::net::UnixListener::from_std(listener)
                .map_err(|error| format!("无法接管 AI 本地接口：{error}"))?;
            let descriptor = GatewayDescriptor {
                version: agent_service::PROTOCOL_VERSION,
                transport: "unix".into(),
                endpoint: endpoint.to_string_lossy().into_owned(),
                token,
                pid: std::process::id(),
                started_at: now(),
            };
            let task = tauri::async_runtime::spawn(run_unix(listener, app, receiver));
            (descriptor, task)
        };

        #[cfg(windows)]
        let (descriptor, task) = {
            use tokio::net::windows::named_pipe::ServerOptions;
            let endpoint = format!(
                r"\\.\pipe\android-bug-desk-{}",
                uuid::Uuid::new_v4().simple()
            );
            let first = ServerOptions::new()
                .first_pipe_instance(true)
                .create(&endpoint)
                .map_err(|error| format!("无法启动 AI 本地接口：{error}"))?;
            let descriptor = GatewayDescriptor {
                version: agent_service::PROTOCOL_VERSION,
                transport: "windows-pipe".into(),
                endpoint: endpoint.clone(),
                token,
                pid: std::process::id(),
                started_at: now(),
            };
            let task = tauri::async_runtime::spawn(run_windows(first, endpoint, app, receiver));
            (descriptor, task)
        };

        write_descriptor(&self.root, &descriptor)?;
        drop(task);
        crate::diagnostics::event("agent_gateway", "started");
        *running = Some(RunningGateway { stop, descriptor });
        Ok(running.as_ref().unwrap().descriptor.clone())
    }

    pub fn stop(&self) {
        let running = self.running.lock().unwrap().take();
        if let Some(running) = running {
            let _ = running.stop.send(true);
            crate::diagnostics::event("agent_gateway", "stopped");
        }
        remove_stale_files(&self.root);
    }
}

pub async fn request(method: &str, params: Value) -> Result<Value> {
    let root = default_app_local_data_dir()?;
    request_at(&root, method, params).await
}

pub async fn request_at(root: &std::path::Path, method: &str, params: Value) -> Result<Value> {
    if method.is_empty() || method.len() > 80 || method.contains(['\r', '\n']) {
        return Err("AI 工具方法名不正确".into());
    }
    let raw = std::fs::read(descriptor_path(root))
        .map_err(|_| "Android Bug Desk 未运行或尚未开启“允许本机 AI 接入”".to_string())?;
    let descriptor: GatewayDescriptor =
        serde_json::from_slice(&raw).map_err(|_| "AI 本地接口描述文件已损坏，请重启客户端")?;
    if descriptor.version != agent_service::PROTOCOL_VERSION || descriptor.pid == 0 {
        return Err("AI 本地接口版本不兼容，请升级或重启 Android Bug Desk".into());
    }
    let request = GatewayRequest {
        version: agent_service::PROTOCOL_VERSION,
        request_id: uuid::Uuid::new_v4().to_string(),
        token: descriptor.token,
        method: method.into(),
        params,
    };

    #[cfg(unix)]
    let response = {
        if descriptor.transport != "unix" {
            return Err("AI 本地接口 transport 与当前系统不匹配".into());
        }
        let stream = tokio::net::UnixStream::connect(&descriptor.endpoint)
            .await
            .map_err(|_| "无法连接 Android Bug Desk，请确认客户端正在运行".to_string())?;
        exchange(stream, &request).await?
    };

    #[cfg(windows)]
    let response = {
        use tokio::net::windows::named_pipe::ClientOptions;
        if descriptor.transport != "windows-pipe" {
            return Err("AI 本地接口 transport 与当前系统不匹配".into());
        }
        let stream = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match ClientOptions::new().open(&descriptor.endpoint) {
                    Ok(stream) => return Ok(stream),
                    Err(error) if error.raw_os_error() == Some(231) => {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                    Err(_) => {
                        return Err("无法连接 Android Bug Desk，请确认客户端正在运行".to_string());
                    }
                }
            }
        })
        .await
        .map_err(|_| "连接 Android Bug Desk 本地接口超时".to_string())??;
        exchange(stream, &request).await?
    };

    if response.request_id != request.request_id || response.version != request.version {
        return Err("AI 本地接口响应与请求不匹配".into());
    }
    match (response.result, response.error) {
        (Some(result), None) => Ok(result),
        (None, Some(error)) => Err(error),
        _ => Err("AI 本地接口返回了无效响应".into()),
    }
}

async fn exchange<S>(mut stream: S, request: &GatewayRequest) -> Result<GatewayResponse>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut bytes = serde_json::to_vec(request).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    tokio::time::timeout(REQUEST_TIMEOUT, stream.write_all(&bytes))
        .await
        .map_err(|_| "写入 AI 本地接口超时".to_string())?
        .map_err(|_| "写入 AI 本地接口失败".to_string())?;
    let mut response = Vec::new();
    let mut reader = BufReader::new(stream).take(MAX_REQUEST_BYTES + 1);
    tokio::time::timeout(REQUEST_TIMEOUT, reader.read_until(b'\n', &mut response))
        .await
        .map_err(|_| "读取 AI 本地接口超时".to_string())?
        .map_err(|_| "读取 AI 本地接口失败".to_string())?;
    if response.len() as u64 > MAX_REQUEST_BYTES {
        return Err("AI 本地接口响应过大".into());
    }
    serde_json::from_slice(&response).map_err(|_| "AI 本地接口响应格式错误".into())
}

#[cfg(unix)]
async fn run_unix<R: tauri::Runtime>(
    listener: tokio::net::UnixListener,
    app: AppHandle<R>,
    mut stop: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() { break; }
            }
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move { serve(stream, app).await; });
                }
                Err(error) => {
                    crate::diagnostics::event("agent_gateway", &format!("accept failed: {error}"));
                    break;
                }
            }
        }
    }
}

#[cfg(windows)]
async fn run_windows<R: tauri::Runtime>(
    mut server: tokio::net::windows::named_pipe::NamedPipeServer,
    endpoint: String,
    app: AppHandle<R>,
    mut stop: tokio::sync::watch::Receiver<bool>,
) {
    use tokio::net::windows::named_pipe::ServerOptions;
    loop {
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() { break; }
                continue;
            }
            connected = server.connect() => {
                if let Err(error) = connected {
                    crate::diagnostics::event("agent_gateway", &format!("pipe connect failed: {error}"));
                    break;
                }
            }
        }
        let next = match ServerOptions::new().create(&endpoint) {
            Ok(next) => next,
            Err(error) => {
                crate::diagnostics::event("agent_gateway", &format!("pipe create failed: {error}"));
                break;
            }
        };
        let connected = std::mem::replace(&mut server, next);
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            serve(connected, app).await;
        });
    }
}

async fn serve<S, R>(mut stream: S, app: AppHandle<R>)
where
    S: AsyncRead + AsyncWrite + Unpin,
    R: tauri::Runtime,
{
    let response = match read_request(&mut stream).await {
        Ok(request) => handle_request(&app, request).await,
        Err(error) => GatewayResponse {
            version: agent_service::PROTOCOL_VERSION,
            request_id: String::new(),
            result: None,
            error: Some(error),
        },
    };
    if let Ok(mut bytes) = serde_json::to_vec(&response) {
        bytes.push(b'\n');
        let _ = stream.write_all(&bytes).await;
        let _ = stream.shutdown().await;
    }
}

async fn read_request<S>(stream: &mut S) -> Result<GatewayRequest>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut bytes = Vec::new();
    let mut reader = BufReader::new(stream).take(MAX_REQUEST_BYTES + 1);
    tokio::time::timeout(REQUEST_TIMEOUT, reader.read_until(b'\n', &mut bytes))
        .await
        .map_err(|_| "AI 本地接口请求超时".to_string())?
        .map_err(|_| "AI 本地接口请求读取失败".to_string())?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_REQUEST_BYTES {
        return Err("AI 本地接口请求为空或过大".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "AI 本地接口请求格式错误".into())
}

async fn handle_request<R: tauri::Runtime>(
    app: &AppHandle<R>,
    request: GatewayRequest,
) -> GatewayResponse {
    let request_id = request.request_id.clone();
    let result = async {
        if request.version != agent_service::PROTOCOL_VERSION {
            return Err("AI 本地接口协议版本不兼容".into());
        }
        if request.request_id.is_empty()
            || request.request_id.len() > 128
            || request.method.is_empty()
            || request.method.len() > 80
        {
            return Err("AI 本地接口请求标识不正确".into());
        }
        let gateway = app.state::<AgentGateway>();
        let expected = gateway
            .status()
            .map(|descriptor| descriptor.token)
            .ok_or("AI 本地接口已关闭")?;
        if !constant_time_eq(request.token.as_bytes(), expected.as_bytes()) {
            return Err("AI 本地接口授权无效".into());
        }
        let state = app.state::<AppState>();
        let options = state
            .store
            .get::<DeveloperOptions>("settings", "developer_options")?
            .unwrap_or_default();
        if !options.agent_access_enabled {
            return Err("请先在设置中开启“允许本机 AI 接入”".into());
        }
        agent_service::dispatch(app, &state, &request.method, request.params).await
    }
    .await;
    match result {
        Ok(result) => GatewayResponse {
            version: agent_service::PROTOCOL_VERSION,
            request_id,
            result: Some(result),
            error: None,
        },
        Err(error) => GatewayResponse {
            version: agent_service::PROTOCOL_VERSION,
            request_id,
            result: None,
            error: Some(error),
        },
    }
}

fn descriptor_path(root: &std::path::Path) -> PathBuf {
    root.join(DESCRIPTOR_NAME)
}

fn write_descriptor(root: &std::path::Path, descriptor: &GatewayDescriptor) -> Result<()> {
    std::fs::create_dir_all(root).map_err(|error| error.to_string())?;
    let path = descriptor_path(root);
    let temporary = root.join(format!(".{DESCRIPTOR_NAME}.tmp"));
    std::fs::write(
        &temporary,
        serde_json::to_vec(descriptor).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    std::fs::rename(&temporary, &path).map_err(|error| error.to_string())
}

fn remove_stale_files(root: &std::path::Path) {
    let _ = std::fs::remove_file(descriptor_path(root));
    #[cfg(unix)]
    let _ = std::fs::remove_file(root.join("agent-gateway.sock"));
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

pub fn default_app_local_data_dir() -> Result<PathBuf> {
    if let Ok(value) = std::env::var("ANDROID_BUG_DESK_DATA_DIR") {
        let path = PathBuf::from(value);
        if path.is_absolute() {
            return Ok(path);
        }
        return Err("ANDROID_BUG_DESK_DATA_DIR 必须是绝对路径".into());
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").ok_or("无法确定当前用户目录")?;
        return Ok(PathBuf::from(home)
            .join("Library/Application Support")
            .join("com.qa.androidbugdesk"));
    }
    #[cfg(target_os = "windows")]
    {
        let local = std::env::var_os("LOCALAPPDATA").ok_or("无法确定本地应用数据目录")?;
        return Ok(PathBuf::from(local).join("com.qa.androidbugdesk"));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(data) = std::env::var_os("XDG_DATA_HOME") {
            return Ok(PathBuf::from(data).join("com.qa.androidbugdesk"));
        }
        let home = std::env::var_os("HOME").ok_or("无法确定当前用户目录")?;
        return Ok(PathBuf::from(home)
            .join(".local/share")
            .join("com.qa.androidbugdesk"));
    }
    #[allow(unreachable_code)]
    Err("当前系统暂不支持 AI 本地接口".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::Manager;

    #[test]
    fn token_comparison_requires_same_bytes() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn descriptor_rejects_unknown_fields() {
        assert!(
            serde_json::from_value::<GatewayDescriptor>(serde_json::json!({
                "version":1,"transport":"unix","endpoint":"/tmp/a","token":"x","pid":1,
                "startedAt":"now","secret":"unexpected"
            }))
            .is_err()
        );
    }

    #[tokio::test]
    async fn gateway_uses_live_app_state_and_removes_endpoint_on_stop() {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::new(root.path().to_path_buf()).unwrap();
        state
            .store
            .put(
                "settings",
                "developer_options",
                &DeveloperOptions {
                    debug_enabled: false,
                    agent_access_enabled: true,
                },
            )
            .unwrap();
        let app = tauri::test::mock_builder()
            .manage(state)
            .manage(AgentGateway::new(root.path().to_path_buf()))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let gateway = app.state::<AgentGateway>();
        gateway.start(app.handle().clone()).await.unwrap();
        let status = request_at(root.path(), "workspace_status", serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(status["authorized"], false);
        assert_eq!(status["submissionPolicy"], "prepare-only");
        assert!(
            request_at(root.path(), "list_devices", serde_json::json!({}))
                .await
                .unwrap_err()
                .contains("登录")
        );
        assert!(request_at(root.path(), "unknown", serde_json::json!({}))
            .await
            .unwrap_err()
            .contains("不支持"));
        gateway.stop();
        assert!(!descriptor_path(root.path()).exists());
    }
}
