//! Public-client OAuth: system browser, S256 PKCE, loopback callback, no renderer credentials.
use crate::{commands::AppState, model::Result, workspace::AuthResult};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use tauri::State;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Mutex, Notify},
    time::{timeout, Duration, Instant},
};
use zeroize::Zeroize;

const SECRET_KEY: &str = "oauth-login";
const MAX_RESPONSE: usize = 65_536;
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

fn unix_now() -> i64 {
    chrono::Utc::now().timestamp()
}
fn random_secret() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}
fn http_client() -> Result<Client> {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| "授权连接初始化失败".into())
}
fn trusted_endpoint(value: &str, base: &str, authorization: bool) -> Result<Url> {
    let url = Url::parse(value).map_err(|_| "云效授权端点格式不正确")?;
    let origin = Url::parse(base).map_err(|_| "云效服务地址不正确")?;
    let same_origin = url.origin() == origin.origin();
    let browser_origin = authorization
        && url.host_str() == Some("account-devops.aliyun.com")
        && url.port_or_known_default() == Some(443);
    let transport = url.scheme() == "https"
        || (cfg!(test)
            && url.scheme() == "http"
            && url.host_str() == Some("127.0.0.1")
            && same_origin);
    if !transport
        || (!same_origin && !browser_origin)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.query().is_some()
    {
        return Err("云效返回了不受信任的授权端点，已停止登录".into());
    }
    Ok(url)
}
async fn response_json(response: reqwest::Response) -> Result<serde_json::Value> {
    let status = response.status();
    let mut bytes = response
        .bytes()
        .await
        .map_err(|_| "授权响应读取失败，请重试登录")?
        .to_vec();
    if bytes.len() > MAX_RESPONSE {
        return Err("授权响应过大，已停止登录".into());
    }
    let parsed =
        serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|_| "授权服务响应格式不正确");
    bytes.zeroize();
    // Never include response bodies, authorization codes, or tokens in errors.
    if !status.is_success() {
        let expired = parsed.as_ref().ok().is_some_and(|v| {
            matches!(v["error"].as_str(), Some("invalid_grant" | "invalid_token"))
        });
        return Err(if expired || status.as_u16() == 401 {
            "[授权失效] 云效授权已过期，请重新授权".into()
        } else {
            format!("云效授权服务返回 HTTP {}，请稍后重试", status.as_u16())
        });
    }
    Ok(parsed?)
}
#[derive(Deserialize)]
struct Endpoints {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    registration_endpoint: String,
    code_challenge_methods_supported: Vec<String>,
    token_endpoint_auth_methods_supported: Vec<String>,
    grant_types_supported: Vec<String>,
    response_types_supported: Vec<String>,
}
async fn discover(client: &Client, base: &str) -> Result<Endpoints> {
    let response = client
        .get(format!("{base}/.well-known/oauth-authorization-server"))
        .send()
        .await
        .map_err(|_| "无法连接云效授权服务，请检查网络后重试")?;
    let endpoints: Endpoints = serde_json::from_value(response_json(response).await?)
        .map_err(|_| "云效授权元数据格式不正确")?;
    if endpoints.issuer.trim_end_matches('/') != base.trim_end_matches('/')
        || !endpoints
            .code_challenge_methods_supported
            .iter()
            .any(|v| v == "S256")
        || !endpoints
            .token_endpoint_auth_methods_supported
            .iter()
            .any(|v| v == "none")
        || !endpoints
            .grant_types_supported
            .iter()
            .any(|v| v == "authorization_code")
        || !endpoints
            .response_types_supported
            .iter()
            .any(|v| v == "code")
    {
        return Err("云效授权服务不支持所需登录方式".into());
    }
    trusted_endpoint(&endpoints.authorization_endpoint, base, true)?;
    trusted_endpoint(&endpoints.token_endpoint, base, false)?;
    trusted_endpoint(&endpoints.registration_endpoint, base, false)?;
    Ok(endpoints)
}

// Stored only in the OS credential store. Never return this DTO through IPC or SQLite.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Tokens {
    client_id: String,
    access_token: String,
    refresh_token: String,
    expires_at: i64,
    user_id: String,
}
impl Drop for Tokens {
    fn drop(&mut self) {
        self.access_token.zeroize();
        self.refresh_token.zeroize();
    }
}
trait Secrets: Send + Sync {
    fn read(&self) -> Result<String>;
    fn write(&self, value: &str) -> Result<()>;
    fn delete(&self) -> Result<()>;
}
struct SystemSecrets;
impl Secrets for SystemSecrets {
    fn read(&self) -> Result<String> {
        keyring::Entry::new(crate::commands::SERVICE, SECRET_KEY)
            .and_then(|entry| entry.get_password())
            .map_err(|_| "无法读取系统保存的云效授权".into())
    }
    fn write(&self, value: &str) -> Result<()> {
        keyring::Entry::new(crate::commands::SERVICE, SECRET_KEY)
            .and_then(|entry| entry.set_password(value))
            .map_err(|_| "系统安全存储不可用，请取消“记住登录”后重试".into())
    }
    fn delete(&self) -> Result<()> {
        match keyring::Entry::new(crate::commands::SERVICE, SECRET_KEY)
            .and_then(|entry| entry.delete_credential())
        {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err("无法移除系统保存的云效授权，请检查安全存储".into()),
        }
    }
}
fn valid_secret(value: &str, prefix: &str) -> bool {
    value.starts_with(prefix)
        && value.len() > prefix.len()
        && value.len() <= 4096
        && !value.chars().any(char::is_whitespace)
}
fn validate_tokens(tokens: &Tokens) -> Result<()> {
    if tokens.client_id.is_empty()
        || tokens.client_id.len() > 512
        || !valid_secret(&tokens.access_token, "oat-")
        || !valid_secret(&tokens.refresh_token, "ort-")
        || tokens.expires_at <= 0
    {
        return Err("云效授权令牌格式不正确，请重新授权".into());
    }
    Ok(())
}
fn decode_tokens(
    value: &serde_json::Value,
    client_id: String,
    fallback_refresh: &str,
    user_id: String,
) -> Result<Tokens> {
    let expires = value["expires_in"]
        .as_i64()
        .filter(|v| (1..=2_678_400).contains(v))
        .ok_or("云效授权缺少有效过期时间")?;
    if !value["token_type"]
        .as_str()
        .is_some_and(|v| v.eq_ignore_ascii_case("bearer"))
    {
        return Err("云效授权令牌类型不正确".into());
    }
    let tokens = Tokens {
        client_id,
        access_token: value["access_token"].as_str().unwrap_or_default().into(),
        refresh_token: value["refresh_token"]
            .as_str()
            .unwrap_or(fallback_refresh)
            .into(),
        expires_at: unix_now() + expires,
        user_id,
    };
    validate_tokens(&tokens)?;
    Ok(tokens)
}
pub struct OAuthSession {
    tokens: Mutex<Tokens>,
    base: String,
    client: Client,
    remembered: AtomicBool,
    persist_needed: AtomicBool,
    closed: AtomicBool,
    secrets: Arc<dyn Secrets>,
}
impl OAuthSession {
    fn new(tokens: Tokens, base: &str, secrets: Arc<dyn Secrets>) -> Result<Self> {
        validate_tokens(&tokens)?;
        Ok(Self {
            tokens: Mutex::new(tokens),
            base: base.into(),
            client: http_client()?,
            remembered: AtomicBool::new(false),
            persist_needed: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            secrets,
        })
    }
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }
    pub async fn close_and_wait(&self) {
        self.close();
        let mut tokens = self.tokens.lock().await;
        tokens.access_token.zeroize();
        tokens.refresh_token.zeroize();
    }
    pub fn contains_secret(&self, text: &str) -> Result<bool> {
        let tokens = self
            .tokens
            .try_lock()
            .map_err(|_| "正在刷新登录授权，请稍后保存")?;
        Ok(text.contains(&tokens.access_token) || text.contains(&tokens.refresh_token))
    }
    pub async fn access_token(&self) -> Result<String> {
        let mut tokens = self.tokens.lock().await;
        if self.closed.load(Ordering::SeqCst) {
            return Err("本次授权已取消，请重新登录".into());
        }
        if tokens.expires_at <= unix_now() + 60 {
            let endpoints = discover(&self.client, &self.base).await?;
            let response = self
                .client
                .post(endpoints.token_endpoint)
                .form(&[
                    ("grant_type", "refresh_token"),
                    ("client_id", &tokens.client_id),
                    ("refresh_token", &tokens.refresh_token),
                ])
                .send()
                .await
                .map_err(|_| "暂时无法刷新云效授权，请检查网络后重试")?;
            let next = decode_tokens(
                &response_json(response).await?,
                tokens.client_id.clone(),
                &tokens.refresh_token,
                tokens.user_id.clone(),
            )?;
            if self.closed.load(Ordering::SeqCst) {
                return Err("本次授权已取消，请重新登录".into());
            }
            *tokens = next;
            self.persist_needed.store(true, Ordering::SeqCst);
        }
        if self.remembered.load(Ordering::SeqCst) && self.persist_needed.load(Ordering::SeqCst) {
            self.persist(&tokens)?;
            self.persist_needed.store(false, Ordering::SeqCst);
        }
        Ok(tokens.access_token.clone())
    }
    fn persist(&self, tokens: &Tokens) -> Result<()> {
        let mut serialized = serde_json::to_string(tokens).map_err(|_| "云效授权保存失败")?;
        let result = self.secrets.write(&serialized);
        serialized.zeroize();
        result
    }
    async fn bind_user(&self, user_id: &str, remember: bool) -> Result<()> {
        let mut tokens = self.tokens.lock().await;
        if !tokens.user_id.is_empty() && tokens.user_id != user_id {
            return Err("保存的授权与当前账号不一致，请重新授权".into());
        }
        tokens.user_id = user_id.into();
        if remember {
            self.persist(&tokens)?;
        } else {
            self.secrets.delete()?;
        }
        self.remembered.store(remember, Ordering::SeqCst);
        Ok(())
    }
    pub async fn verify_user(&self, user_id: &str) -> Result<()> {
        let tokens = self.tokens.lock().await;
        if tokens.user_id.is_empty() || tokens.user_id != user_id {
            return Err("[授权失效] 保存的云效授权与账号不一致，请重新授权".into());
        }
        Ok(())
    }
}
pub fn restore(store: &crate::store::Store, base: &str) -> Result<Option<Arc<OAuthSession>>> {
    restore_with(store, base, Arc::new(SystemSecrets))
}
fn restore_with(
    store: &crate::store::Store,
    base: &str,
    secrets: Arc<dyn Secrets>,
) -> Result<Option<Arc<OAuthSession>>> {
    if !crate::login_memory::enabled(store)?
        || store.get::<String>("settings", "login_method")?.as_deref() != Some("oauth")
    {
        return Ok(None);
    }
    let Ok(mut serialized) = secrets.read() else {
        return Ok(None);
    };
    let parsed = serde_json::from_str::<Tokens>(&serialized);
    serialized.zeroize();
    let Ok(tokens) = parsed else {
        return Ok(None);
    };
    let Ok(session) = OAuthSession::new(tokens, base, secrets) else {
        return Ok(None);
    };
    session.remembered.store(true, Ordering::SeqCst);
    Ok(Some(Arc::new(session)))
}
pub fn clear_saved() -> Result<()> {
    SystemSecrets.delete()
}

#[derive(Default)]
pub struct LoginFlow {
    generation: AtomicU64,
    running: AtomicBool,
    cancel: Notify,
}
impl LoginFlow {
    pub fn cancel(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.cancel.notify_waiters();
    }
    fn check(&self, generation: u64) -> Result<()> {
        if self.generation.load(Ordering::SeqCst) != generation {
            Err("已取消授权登录".into())
        } else {
            Ok(())
        }
    }
}
struct FlowGuard<'a> {
    flow: &'a LoginFlow,
    generation: u64,
}
impl Drop for FlowGuard<'_> {
    fn drop(&mut self) {
        self.flow.running.store(false, Ordering::SeqCst);
    }
}

async fn read_callback(
    stream: &mut TcpStream,
    port: u16,
    state: &str,
) -> Result<Option<Result<String>>> {
    let mut request = Vec::new();
    while request.len() < 8192 {
        let mut chunk = [0u8; 512];
        let count = stream
            .read(&mut chunk)
            .await
            .map_err(|_| "授权回调读取失败")?;
        if count == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..count]);
        if request.windows(4).any(|v| v == b"\r\n\r\n") {
            break;
        }
    }
    let request = String::from_utf8(request).map_err(|_| "授权回调格式不正确")?;
    let first: Vec<&str> = request
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect();
    let authority = format!("127.0.0.1:{port}");
    let host = request.lines().find_map(|line| {
        line.split_once(':')
            .filter(|(key, _)| key.eq_ignore_ascii_case("host"))
            .map(|(_, value)| value.trim())
    });
    let url = first
        .get(1)
        .and_then(|target| Url::parse(&format!("http://{authority}{target}")).ok());
    let mut accepted = None;
    if first.first() == Some(&"GET") && host == Some(authority.as_str()) {
        if let Some(url) = url.filter(|v| v.path() == "/oauth/callback") {
            let pairs: Vec<_> = url.query_pairs().collect();
            let values = |key: &str| {
                pairs
                    .iter()
                    .filter(|(k, _)| k == key)
                    .map(|(_, v)| v.as_ref())
                    .collect::<Vec<_>>()
            };
            let states = values("state");
            if states.len() == 1 && states[0] == state {
                let codes = values("code");
                let errors = values("error");
                accepted = Some(if errors.len() == 1 && codes.is_empty() {
                    Err("未完成云效授权，可重新授权或使用 PAT 登录".into())
                } else if errors.is_empty()
                    && codes.len() == 1
                    && !codes[0].is_empty()
                    && codes[0].len() <= 4096
                    && !codes[0].chars().any(char::is_whitespace)
                {
                    Ok(codes[0].to_string())
                } else {
                    Err("云效授权回调缺少有效授权码，请重新授权".into())
                });
            }
        }
    }
    let (status, text) = if accepted.is_some() {
        (
            "200 OK",
            "授权结果已返回客户端。可以关闭此页面，返回 Android Bug Desk。",
        )
    } else {
        (
            "400 Bad Request",
            "无效的授权回调。请返回 Android Bug Desk 重新授权。",
        )
    };
    let body = format!("<!doctype html><html lang=\"zh\"><meta charset=\"utf-8\"><meta name=\"referrer\" content=\"no-referrer\"><title>Android Bug Desk</title><p>{text}</p></html>");
    let reply = format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'\r\nConnection: close\r\n\r\n{body}", body.len());
    let _ = stream.write_all(reply.as_bytes()).await;
    let _ = stream.shutdown().await;
    Ok(accepted)
}
async fn authorize<'a>(
    base: &str,
    flow: &'a LoginFlow,
    secrets: Arc<dyn Secrets>,
    open_browser: impl FnOnce(&str) -> Result<()>,
) -> Result<(Arc<OAuthSession>, FlowGuard<'a>)> {
    if flow
        .running
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("授权登录正在进行，请先完成或取消".into());
    }
    let generation = flow.generation.fetch_add(1, Ordering::SeqCst) + 1;
    let guard = FlowGuard { flow, generation };
    let client = http_client()?;
    let endpoints = discover(&client, base).await?;
    flow.check(generation)?;
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|_| "无法开启本机授权回调，请使用 PAT 登录")?;
    let port = listener
        .local_addr()
        .map_err(|_| "无法读取授权回调地址")?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/oauth/callback");
    let registration = client.post(endpoints.registration_endpoint).json(&json!({"client_name":"Android Bug Desk","redirect_uris":[redirect_uri],"grant_types":["authorization_code","refresh_token"],"response_types":["code"],"token_endpoint_auth_method":"none"})).send().await.map_err(|_| "云效授权客户端注册失败，请检查网络后重试")?;
    let registered = response_json(registration).await?;
    let client_id = registered["client_id"]
        .as_str()
        .filter(|v| !v.is_empty() && v.len() <= 512)
        .ok_or("云效未返回有效授权客户端 ID")?
        .to_string();
    let verifier = zeroize::Zeroizing::new(random_secret());
    let state = random_secret();
    let mut url =
        Url::parse(&endpoints.authorization_endpoint).map_err(|_| "授权页面地址不正确")?;
    url.query_pairs_mut().extend_pairs([
        ("response_type", "code"),
        ("client_id", client_id.as_str()),
        ("redirect_uri", redirect_uri.as_str()),
        ("state", state.as_str()),
        ("code_challenge", challenge(&verifier).as_str()),
        ("code_challenge_method", "S256"),
    ]);
    flow.check(generation)?;
    open_browser(url.as_str())?;
    let deadline = Instant::now() + LOGIN_TIMEOUT;
    let code = loop {
        flow.check(generation)?;
        let accepted = tokio::select! {
            _ = flow.cancel.notified() => { flow.check(generation)?; continue; },
            _ = tokio::time::sleep_until(deadline) => return Err("授权等待已超时，请重新授权或使用 PAT 登录".into()),
            accepted = listener.accept() => accepted.map_err(|_| "授权回调连接失败")?,
        };
        let (mut stream, _) = accepted;
        if let Ok(Ok(Some(result))) = timeout(
            Duration::from_secs(3),
            read_callback(&mut stream, port, &state),
        )
        .await
        {
            break zeroize::Zeroizing::new(result?);
        }
    };
    flow.check(generation)?;
    let response = client
        .post(endpoints.token_endpoint)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", &client_id),
            ("redirect_uri", &redirect_uri),
            ("code", code.as_str()),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .await
        .map_err(|_| "云效授权令牌获取失败，请重新授权")?;
    let tokens = decode_tokens(&response_json(response).await?, client_id, "", "".into())?;
    flow.check(generation)?;
    Ok((Arc::new(OAuthSession::new(tokens, base, secrets)?), guard))
}

#[tauri::command]
pub fn cancel_oauth_login(state: State<AppState>) {
    if state.oauth_flow.running.load(Ordering::SeqCst) {
        state.oauth_flow.cancel();
    }
}
#[tauri::command]
pub async fn oauth_login(remember: bool, state: State<'_, AppState>) -> Result<AuthResult> {
    if state.collector.lock().unwrap().is_some() || !state.locks.lock().unwrap().is_empty() {
        return Err("请先停止采集并等待提交结束后再登录".into());
    }
    crate::workspace::invalidate(&state);
    let epoch = state.auth_epoch.load(Ordering::SeqCst);
    let (session, guard) = authorize(
        &state.api_base,
        &state.oauth_flow,
        Arc::new(SystemSecrets),
        |url| open::that(url).map_err(|_| "无法打开系统浏览器，请使用 PAT 登录".into()),
    )
    .await?;
    let token = zeroize::Zeroizing::new(session.access_token().await?);
    let user =
        crate::yunxiao::Api::with_base(crate::workspace::empty_profile(), &token, &state.api_base)?
            .with_oauth(Some(session.clone()))
            .verify_user()
            .await?;
    if epoch != state.auth_epoch.load(Ordering::SeqCst) {
        return Err("登录已变化，请重新授权".into());
    }
    let ensure_current = || {
        state.oauth_flow.check(guard.generation)?;
        if epoch == state.auth_epoch.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err("登录已变化，请重新授权".to_string())
        }
    };
    ensure_current()?;
    let previous = state.oauth.lock().unwrap().take();
    if let Some(previous) = previous {
        previous.close_and_wait().await;
    }
    ensure_current()?;
    session.bind_user(&user.id, remember).await?;
    ensure_current()?;
    // Clear a previous PAT when switching login methods; credentials remain OS-only.
    crate::login_memory::clear_pat()?;
    state.store.put("settings", "login_method", &"oauth")?;
    crate::login_memory::set_enabled(&state.store, remember)?;
    if let Some(old) = state.oauth.lock().unwrap().replace(session) {
        old.close();
    }
    let mut old = state.token.lock().unwrap();
    if let Some(value) = old.as_mut() {
        value.zeroize();
    }
    *old = Some(token.to_string());
    drop(old);
    crate::workspace::accept_identity(&state, user, epoch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use wiremock::{
        matchers::{header, method, path},
        Mock, MockServer, ResponseTemplate,
    };

    #[derive(Default)]
    struct MemorySecrets {
        value: StdMutex<Option<String>>,
        writes: AtomicU64,
        fail: AtomicBool,
    }
    impl Secrets for MemorySecrets {
        fn read(&self) -> Result<String> {
            self.value.lock().unwrap().clone().ok_or("missing".into())
        }
        fn write(&self, value: &str) -> Result<()> {
            if self.fail.load(Ordering::SeqCst) {
                return Err("mock safe storage unavailable".into());
            }
            self.writes.fetch_add(1, Ordering::SeqCst);
            *self.value.lock().unwrap() = Some(value.into());
            Ok(())
        }
        fn delete(&self) -> Result<()> {
            *self.value.lock().unwrap() = None;
            Ok(())
        }
    }
    fn tokens(expired: bool) -> Tokens {
        Tokens {
            client_id: "mock-client".into(),
            access_token: "oat-old-access".into(),
            refresh_token: "ort-old-refresh".into(),
            expires_at: unix_now() + if expired { -60 } else { 3600 },
            user_id: "mock-user".into(),
        }
    }
    fn token_reply() -> serde_json::Value {
        json!({"access_token":"oat-new-access","refresh_token":"ort-new-refresh","expires_in":86400,"token_type":"Bearer"})
    }
    async fn metadata(server: &MockServer, token_endpoint: Option<&str>) {
        Mock::given(method("GET")).and(path("/.well-known/oauth-authorization-server")).respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer":server.uri(), "authorization_endpoint":format!("{}/authorize",server.uri()), "registration_endpoint":format!("{}/register",server.uri()), "token_endpoint":token_endpoint.map(str::to_owned).unwrap_or_else(||format!("{}/token",server.uri())),
            "code_challenge_methods_supported":["S256"], "grant_types_supported":["authorization_code","refresh_token"], "response_types_supported":["code"], "token_endpoint_auth_methods_supported":["none"]
        }))).mount(server).await;
    }
    async fn registration(server: &MockServer) {
        Mock::given(method("POST"))
            .and(path("/register"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"client_id":"mock-client"})),
            )
            .expect(1)
            .mount(server)
            .await;
    }
    fn form(request: &wiremock::Request) -> std::collections::HashMap<String, String> {
        Url::parse(&format!(
            "http://localhost/?{}",
            std::str::from_utf8(&request.body).unwrap()
        ))
        .unwrap()
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
    }
    fn callback(browser_url: &str, key: &str, value: &str) -> String {
        let url = Url::parse(browser_url).unwrap();
        let params: std::collections::HashMap<_, _> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        let mut redirect = Url::parse(&params["redirect_uri"]).unwrap();
        redirect
            .query_pairs_mut()
            .extend_pairs([("state", params["state"].as_str()), (key, value)]);
        redirect.into()
    }
    #[test]
    fn pkce_matches_rfc7636_and_untrusted_endpoints_are_rejected() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        for endpoint in [
            "https://evil.example/token",
            "http://openapi-rdc.aliyuncs.com/token",
            "https://name:secret@openapi-rdc.aliyuncs.com/token",
            "https://openapi-rdc.aliyuncs.com/token?secret=x",
        ] {
            assert!(trusted_endpoint(endpoint, "https://openapi-rdc.aliyuncs.com", false).is_err());
        }
        assert!(trusted_endpoint(
            "https://account-devops.aliyun.com/v1/oauth2/authorize",
            "https://openapi-rdc.aliyuncs.com",
            true
        )
        .is_ok());
        assert!(trusted_endpoint(
            "https://account-devops.aliyun.com/token",
            "https://openapi-rdc.aliyuncs.com",
            false
        )
        .is_err());
    }
    #[tokio::test]
    async fn full_flow_registers_loopback_checks_state_and_exchanges_s256_code() {
        let server = MockServer::start().await;
        metadata(&server, None).await;
        registration(&server).await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(token_reply()))
            .expect(1)
            .mount(&server)
            .await;
        let browser = Arc::new(StdMutex::new(String::new()));
        let captured = browser.clone();
        let secrets = Arc::new(MemorySecrets::default());
        let flow = LoginFlow::default();
        let (session, guard) = authorize(&server.uri(), &flow, secrets.clone(), move |url| {
            *captured.lock().unwrap() = url.into();
            let correct = callback(url, "code", "mock-code");
            let mut wrong = Url::parse(&correct).unwrap();
            wrong.set_query(Some("state=wrong&code=intercepted"));
            tokio::spawn(async move {
                let c = http_client().unwrap();
                assert_eq!(c.get(wrong).send().await.unwrap().status(), 400);
                let response = c.get(correct).send().await.unwrap();
                assert_eq!(response.status(), 200);
                assert!(!response.text().await.unwrap().contains("mock-code"));
            });
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(session.access_token().await.unwrap(), "oat-new-access");
        session.bind_user("mock-user", true).await.unwrap();
        session.verify_user("mock-user").await.unwrap();
        assert!(session.verify_user("other-user").await.is_err());
        assert!(flow.running.load(Ordering::SeqCst));
        drop(guard);
        assert!(!flow.running.load(Ordering::SeqCst));
        let requests = server.received_requests().await.unwrap();
        let registered: serde_json::Value = serde_json::from_slice(
            &requests
                .iter()
                .find(|r| r.url.path() == "/register")
                .unwrap()
                .body,
        )
        .unwrap();
        let exchange = form(requests.iter().find(|r| r.url.path() == "/token").unwrap());
        let opened = Url::parse(&browser.lock().unwrap()).unwrap();
        let params: std::collections::HashMap<_, _> = opened
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert_eq!(exchange["grant_type"], "authorization_code");
        assert_eq!(exchange["code"], "mock-code");
        assert_eq!(exchange["redirect_uri"], registered["redirect_uris"][0]);
        assert_eq!(
            challenge(&exchange["code_verifier"]),
            params["code_challenge"]
        );
        assert_eq!(params["code_challenge_method"], "S256");
        assert_eq!(registered["token_endpoint_auth_method"], "none");
        assert!(registered.get("client_secret").is_none());
        assert!(secrets.read().unwrap().contains("ort-new-refresh"));
    }
    #[tokio::test]
    async fn denial_and_cancel_do_not_exchange_or_save_tokens() {
        for denied in [true, false] {
            let server = MockServer::start().await;
            metadata(&server, None).await;
            registration(&server).await;
            let secrets = Arc::new(MemorySecrets::default());
            let flow = LoginFlow::default();
            let result = authorize(&server.uri(), &flow, secrets.clone(), |url| {
                if denied {
                    let url = callback(url, "error", "access_denied");
                    tokio::spawn(async move {
                        http_client().unwrap().get(url).send().await.unwrap();
                    });
                } else {
                    flow.cancel();
                }
                Ok(())
            })
            .await;
            assert!(result.is_err());
            assert!(!flow.running.load(Ordering::SeqCst));
            assert_eq!(secrets.writes.load(Ordering::SeqCst), 0);
            assert!(!server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .any(|r| r.url.path() == "/token"));
        }
    }
    #[tokio::test]
    async fn malicious_discovery_stops_before_registration() {
        let server = MockServer::start().await;
        metadata(&server, Some("https://evil.example/token")).await;
        let flow = LoginFlow::default();
        let result = authorize(
            &server.uri(),
            &flow,
            Arc::new(MemorySecrets::default()),
            |_| panic!("must not open browser"),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
    #[tokio::test]
    async fn concurrent_refresh_rotates_once_and_updates_safe_storage() {
        let server = MockServer::start().await;
        metadata(&server, None).await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(token_reply())
                    .set_delay(Duration::from_millis(50)),
            )
            .expect(1)
            .mount(&server)
            .await;
        let secrets = Arc::new(MemorySecrets::default());
        let session =
            Arc::new(OAuthSession::new(tokens(true), &server.uri(), secrets.clone()).unwrap());
        session.remembered.store(true, Ordering::SeqCst);
        let (first, second) = tokio::join!(session.access_token(), session.access_token());
        assert_eq!(first.unwrap(), "oat-new-access");
        assert_eq!(second.unwrap(), "oat-new-access");
        assert_eq!(secrets.writes.load(Ordering::SeqCst), 1);
        let requests = server.received_requests().await.unwrap();
        let refresh = form(requests.iter().find(|r| r.url.path() == "/token").unwrap());
        assert_eq!(refresh["grant_type"], "refresh_token");
        assert_eq!(refresh["refresh_token"], "ort-old-refresh");
        assert!(!refresh.contains_key("client_secret"));
        assert!(secrets.read().unwrap().contains("ort-new-refresh"));
    }
    #[tokio::test]
    async fn refresh_errors_never_include_tokens_and_invalid_grant_requires_login() {
        let server = MockServer::start().await;
        metadata(&server, None).await;
        Mock::given(method("POST")).and(path("/token")).respond_with(ResponseTemplate::new(400).set_body_json(json!({"error":"invalid_grant","error_description":"ort-old-refresh oat-old-access server-secret"}))).mount(&server).await;
        let secrets = Arc::new(MemorySecrets::default());
        let session = OAuthSession::new(tokens(true), &server.uri(), secrets.clone()).unwrap();
        let error = session.access_token().await.unwrap_err();
        assert!(error.contains("授权失效"));
        assert!(!error.contains("ort-"));
        assert!(!error.contains("server-secret"));
        assert_eq!(secrets.writes.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn failed_refresh_persistence_retries_storage_without_reusing_rotated_token() {
        let server = MockServer::start().await;
        metadata(&server, None).await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(token_reply()))
            .expect(1)
            .mount(&server)
            .await;
        let secrets = Arc::new(MemorySecrets::default());
        secrets.fail.store(true, Ordering::SeqCst);
        let session = OAuthSession::new(tokens(true), &server.uri(), secrets.clone()).unwrap();
        session.remembered.store(true, Ordering::SeqCst);
        assert!(session.access_token().await.is_err());
        secrets.fail.store(false, Ordering::SeqCst);
        assert_eq!(session.access_token().await.unwrap(), "oat-new-access");
        assert!(secrets.read().unwrap().contains("ort-new-refresh"));
    }
    #[tokio::test]
    async fn closing_a_session_prevents_refresh_and_detects_credentials_in_drafts() {
        let server = MockServer::start().await;
        let session = OAuthSession::new(
            tokens(false),
            &server.uri(),
            Arc::new(MemorySecrets::default()),
        )
        .unwrap();
        assert!(session
            .contains_secret("prefix ort-old-refresh suffix")
            .unwrap());
        assert!(!session
            .contains_secret("ordinary issue description")
            .unwrap());
        session.close_and_wait().await;
        assert!(session.access_token().await.is_err());
        assert!(server.received_requests().await.unwrap().is_empty());
    }
    #[tokio::test]
    async fn restored_oauth_verifies_identity_and_keeps_tokens_out_of_bootstrap() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/oapi/v1/platform/user"))
            .and(header("Authorization", "Bearer oat-old-access"))
            .and(header("x-yunxiao-token", "oat-old-access"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"id":"mock-user","name":"测试用户"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let root = tempfile::tempdir().unwrap();
        let mut state = AppState::new(root.path().into()).unwrap();
        state.api_base = server.uri();
        let session = Arc::new(
            OAuthSession::new(
                tokens(false),
                &server.uri(),
                Arc::new(MemorySecrets::default()),
            )
            .unwrap(),
        );
        *state.oauth.lock().unwrap() = Some(session);
        let mut profile = crate::workspace::empty_profile();
        profile.organization_id = "mock-org".into();
        profile.project_id = "mock-project".into();
        profile.name = "测试项目".into();
        state
            .store
            .put("settings", "profile:mock-user", &profile)
            .unwrap();
        assert_eq!(
            crate::workspace::check_auth(&state).await.unwrap().status,
            "valid"
        );
        assert_eq!(state.profile.lock().unwrap().project_id, "mock-project");
        assert!(state.api().is_ok());
        assert!(state
            .assert_no_token(&json!({"title":"ort-old-refresh"}))
            .is_err());
        use tauri::Manager;
        let app = tauri::test::mock_builder()
            .manage(state)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let bootstrap = crate::commands::bootstrap(app.state()).unwrap().to_string();
        assert!(!bootstrap.contains("oat-"));
        assert!(!bootstrap.contains("ort-"));
        let settings = app.state::<AppState>().store.list("settings").unwrap();
        assert!(!serde_json::to_string(&settings).unwrap().contains("oat-"));
    }
    #[tokio::test]
    async fn restore_only_reads_opted_in_oauth_and_never_mixes_pat_credentials() {
        let root = tempfile::tempdir().unwrap();
        let store = crate::store::Store::new(root.path()).unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        secrets
            .write(&serde_json::to_string(&tokens(false)).unwrap())
            .unwrap();
        assert!(
            restore_with(&store, "https://openapi-rdc.aliyuncs.com", secrets.clone())
                .unwrap()
                .is_none()
        );
        crate::login_memory::set_enabled(&store, true).unwrap();
        store.put("settings", "login_method", &"pat").unwrap();
        assert!(
            restore_with(&store, "https://openapi-rdc.aliyuncs.com", secrets.clone())
                .unwrap()
                .is_none()
        );
        store.put("settings", "login_method", &"oauth").unwrap();
        assert!(crate::login_memory::load_token(&store).unwrap().is_none());
        let restored = restore_with(&store, "https://openapi-rdc.aliyuncs.com", secrets.clone())
            .unwrap()
            .unwrap();
        assert_eq!(restored.access_token().await.unwrap(), "oat-old-access");
        secrets.delete().unwrap();
        assert!(
            restore_with(&store, "https://openapi-rdc.aliyuncs.com", secrets.clone())
                .unwrap()
                .is_none()
        );
        secrets.write("corrupt credential").unwrap();
        assert!(
            restore_with(&store, "https://openapi-rdc.aliyuncs.com", secrets)
                .unwrap()
                .is_none()
        );
        assert!(!serde_json::to_string(&store.list("settings").unwrap())
            .unwrap()
            .contains("oat-"));
    }
    #[tokio::test]
    async fn api_refreshes_before_sending_and_uses_new_bearer_and_yunxiao_headers() {
        let server = MockServer::start().await;
        metadata(&server, None).await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(token_reply()))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/oapi/v1/platform/user"))
            .and(header("Authorization", "Bearer oat-new-access"))
            .and(header("x-yunxiao-token", "oat-new-access"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"id":"mock-user","name":"测试用户"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let session = Arc::new(
            OAuthSession::new(
                tokens(true),
                &server.uri(),
                Arc::new(MemorySecrets::default()),
            )
            .unwrap(),
        );
        let api = crate::yunxiao::Api::with_base(
            crate::workspace::empty_profile(),
            "oat-old-access",
            &server.uri(),
        )
        .unwrap()
        .with_oauth(Some(session));
        assert_eq!(api.verify_user().await.unwrap().id, "mock-user");
    }
    #[tokio::test]
    async fn closing_during_refresh_does_not_resurrect_saved_credentials() {
        let server = MockServer::start().await;
        metadata(&server, None).await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(token_reply())
                    .set_delay(Duration::from_millis(100)),
            )
            .mount(&server)
            .await;
        let secrets = Arc::new(MemorySecrets::default());
        let session =
            Arc::new(OAuthSession::new(tokens(true), &server.uri(), secrets.clone()).unwrap());
        session.remembered.store(true, Ordering::SeqCst);
        let running = session.clone();
        let refresh = tokio::spawn(async move { running.access_token().await });
        timeout(Duration::from_secs(2), async {
            loop {
                if server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .any(|r| r.url.path() == "/token")
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        session.close_and_wait().await;
        secrets.delete().unwrap();
        assert!(refresh.await.unwrap().is_err());
        assert_eq!(secrets.writes.load(Ordering::SeqCst), 0);
        assert!(secrets.read().is_err());
    }
    #[tokio::test]
    async fn restored_different_user_is_rejected_before_workspace_access() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/oapi/v1/platform/user"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"id":"other-user","name":"其他用户"})),
            )
            .mount(&server)
            .await;
        let root = tempfile::tempdir().unwrap();
        let mut state = AppState::new(root.path().into()).unwrap();
        state.api_base = server.uri();
        *state.oauth.lock().unwrap() = Some(Arc::new(
            OAuthSession::new(
                tokens(false),
                &server.uri(),
                Arc::new(MemorySecrets::default()),
            )
            .unwrap(),
        ));
        let result = crate::workspace::check_auth(&state).await.unwrap();
        assert_eq!(result.status, "invalid");
        assert!(state.user.lock().unwrap().is_none());
        assert!(state.api().is_err());
    }
    #[test]
    fn malformed_token_responses_are_rejected() {
        for value in [
            json!({"access_token":"oat-x","refresh_token":"ort-x","expires_in":0,"token_type":"bearer"}),
            json!({"access_token":"oat-x","refresh_token":"ort-x","expires_in":3600,"token_type":"unknown"}),
            json!({"access_token":"oat-bad\ntoken","refresh_token":"ort-x","expires_in":3600,"token_type":"bearer"}),
        ] {
            assert!(decode_tokens(&value, "client".into(), "", "".into()).is_err());
        }
    }
}
