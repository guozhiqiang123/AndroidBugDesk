use crate::{
    adb, evidence, model::*, store::Store, submission::Submitter, validation, yunxiao::Api,
};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{atomic::AtomicU64, Mutex},
};
use tauri::{Manager, State};
use zeroize::Zeroize;
pub struct AppState {
    pub store: Store,
    pub profile: Mutex<Profile>,
    pub token: Mutex<Option<String>>,
    pub user: Mutex<Option<crate::workspace::UserIdentity>>,
    pub auth_epoch: AtomicU64,
    pub workspace_epoch: AtomicU64,
    pub(crate) oauth: Mutex<Option<std::sync::Arc<crate::oauth::OAuthSession>>>,
    pub(crate) oauth_flow: crate::oauth::LoginFlow,
    pub(crate) api_base: String,
    pub metadata: Mutex<Option<Metadata>>,
    pub adb: Mutex<Option<PathBuf>>,
    pub collector: Mutex<Option<adb::Collector>>,
    pub locks: Mutex<HashSet<String>>,
}
pub(crate) const SERVICE: &str = "com.qa.androidbugdesk";
impl AppState {
    pub fn new(root: PathBuf) -> Result<Self> {
        let store = Store::new(&root)?;
        store.recover()?;
        crate::capture::recover(&store)?;
        for mut session in store.list("session")? {
            if session["stoppedAt"].is_null() {
                session["stoppedAt"] = json!(now());
                session["gaps"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!("客户端上次退出导致采集中断；需建立新会话"));
                store.put("session", &string(&session, "id"), &session)?;
            }
        }
        // Restore only after validating the token's actual user. Never auto-select the example project.
        let profile = crate::workspace::empty_profile();
        let token = crate::login_memory::load_token(&store)?;
        let oauth = crate::oauth::restore(&store, "https://openapi-rdc.aliyuncs.com")?;
        Ok(Self {
            store,
            profile: Mutex::new(profile),
            token: Mutex::new(token),
            user: Mutex::new(None),
            auth_epoch: AtomicU64::new(0),
            workspace_epoch: AtomicU64::new(0),
            oauth: Mutex::new(oauth),
            oauth_flow: Default::default(),
            api_base: "https://openapi-rdc.aliyuncs.com".into(),
            metadata: Mutex::new(None),
            adb: Mutex::new(adb::discover()),
            collector: Mutex::new(None),
            locks: Mutex::new(HashSet::new()),
        })
    }
    pub fn api(&self) -> Result<Api> {
        if self.user.lock().unwrap().is_none() {
            return Err("请先验证登录令牌".into());
        }
        let p = self.profile.lock().unwrap().clone();
        let t = self.token.lock().unwrap();
        Api::with_base(p, t.as_deref().ok_or("请先登录云效")?, &self.api_base)
            .map(|api| api.with_oauth(self.oauth.lock().unwrap().clone()))
    }
    pub(crate) fn editable(&self, id: &str) -> Result<()> {
        if self.locks.lock().unwrap().contains(id) {
            return Err("该草稿正在提交，不能修改证据".into());
        }
        self.store.editable(id)
    }
    pub(crate) fn adb_path(&self) -> Result<PathBuf> {
        self.adb.lock().unwrap().clone().ok_or_else(|| {
            "未找到 ADB，请安装官方 Android SDK Platform Tools 或选择 adb 路径".into()
        })
    }
    pub(crate) fn assert_no_token(&self, d: &Value) -> Result<()> {
        if let Some(session) = self.oauth.lock().unwrap().as_ref() {
            if session.contains_secret(&d.to_string())? {
                return Err("草稿包含当前云效授权凭据，请移除后再保存或上传".into());
            }
        }
        let token = self.token.lock().unwrap().clone();
        if let Some(mut t) = token {
            if t.len() >= 8 {
                if d.to_string().contains(&t) {
                    t.zeroize();
                    return Err("草稿包含当前 PAT，请移除凭据后再保存或上传".into());
                }
                for a in &attachments(d)? {
                    use std::io::Read;
                    let mut f = std::fs::File::open(&a.path).map_err(|_| "附件不存在")?;
                    let mut buf = [0u8; 65536];
                    let mut overlap = Vec::new();
                    loop {
                        let n = f.read(&mut buf).map_err(|_| "附件读取失败")?;
                        if n == 0 {
                            break;
                        }
                        overlap.extend_from_slice(&buf[..n]);
                        if overlap.windows(t.len()).any(|v| v == t.as_bytes()) {
                            t.zeroize();
                            return Err("附件包含当前 PAT，请生成脱敏副本后确认上传".into());
                        }
                        let keep = t.len() - 1;
                        if overlap.len() > keep {
                            overlap.drain(..overlap.len() - keep);
                        }
                    }
                }
            }
            t.zeroize();
        }
        Ok(())
    }
}
#[tauri::command]
pub fn bootstrap(state: State<AppState>) -> Result<Value> {
    let user = state.user.lock().unwrap().clone();
    let authorized = user.is_some();
    let remember_login = crate::login_memory::enabled(&state.store)?;
    let drafts = state.store.list("draft")?;
    let draft_scopes = crate::project_settings::draft_scopes_for_state(&state, &drafts)?;
    let submissions = crate::workitem_links::history_with_urls(state.store.list("submission")?);
    Ok(
        json!({"profile":state.profile.lock().unwrap().clone(),"authorized":authorized,"rememberLogin":remember_login,"loginMethod":state.store.get::<String>("settings", "login_method")?.unwrap_or_else(|| if remember_login { "pat".into() } else { "oauth".into() }),"user":user,"metadata":state.metadata.lock().unwrap().clone(),"adbPath":state.adb.lock().unwrap().as_ref().map(|v|v.to_string_lossy()),"drafts":drafts,"draftScopes":draft_scopes,"submissions":submissions,"sessions":state.store.list("session")?,"captures":state.store.list("capture")?}),
    )
}
#[tauri::command]
pub async fn set_auth(token: String, remember: bool, state: State<'_, AppState>) -> Result<()> {
    let result = crate::workspace::login(&state, token, remember).await?;
    if result.status == "valid" {
        Ok(())
    } else {
        Err(result.message)
    }
}
#[tauri::command]
pub async fn check_auth(state: State<'_, AppState>) -> Result<crate::workspace::AuthResult> {
    crate::workspace::check_auth(&state).await
}
#[tauri::command]
pub async fn login(
    token: String,
    remember: bool,
    state: State<'_, AppState>,
) -> Result<crate::workspace::AuthResult> {
    crate::workspace::login(&state, token, remember).await
}
#[tauri::command]
pub async fn clear_auth(state: State<'_, AppState>) -> Result<()> {
    if !state.locks.lock().unwrap().is_empty() {
        return Err("请等待本次提交结束后再退出登录".into());
    }
    crate::workspace::invalidate(&state);
    state.oauth_flow.cancel();
    let oauth = state.oauth.lock().unwrap().take();
    if let Some(session) = oauth {
        session.close_and_wait().await;
    }
    stop_session(state.clone()).await?;
    crate::login_memory::set_enabled(&state.store, false)?;
    crate::oauth::clear_saved()?;
    let mut t = state.token.lock().unwrap();
    if let Some(s) = t.as_mut() {
        s.zeroize();
    }
    *t = None;
    *state.metadata.lock().unwrap() = None;
    match keyring::Entry::new(SERVICE, "personal-pat")
        .map_err(|_| "无法访问系统安全存储")?
        .delete_credential()
    {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err(
            "本次授权已清除，但系统保存项清除失败，请检查钥匙串/凭据管理器/Secret Service".into(),
        ),
    }
}
#[tauri::command]
pub async fn load_metadata(type_id: String, state: State<'_, AppState>) -> Result<Metadata> {
    let scope = crate::workspace::current_scope(&state, "")?;
    let epoch = state
        .workspace_epoch
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        + 1;
    *state.metadata.lock().unwrap() = None;
    let remembered = if type_id.is_empty() {
        state
            .store
            .get::<String>("last_type", &scope)?
            .unwrap_or_default()
    } else {
        type_id.clone()
    };
    let api = state.api()?;
    let result = api.metadata(&remembered).await;
    let m = if type_id.is_empty()
        && result
            .as_ref()
            .err()
            .is_some_and(|e| e.contains("所选 Bug 类型不属于当前项目"))
    {
        crate::workspace::observed_error(&state, api.metadata("").await)?
    } else {
        crate::workspace::observed_error(&state, result)?
    };
    if epoch
        != state
            .workspace_epoch
            .load(std::sync::atomic::Ordering::SeqCst)
        || crate::workspace::current_scope(&state, "")? != scope
    {
        return Err("账号或项目已变化，请重新读取字段".into());
    }
    *state.metadata.lock().unwrap() = Some(m.clone());
    Ok(m)
}
#[tauri::command]
pub async fn list_organizations(state: State<'_, AppState>) -> Result<Vec<OptionItem>> {
    crate::workspace::observed_error(&state, state.api()?.organizations().await)
}
#[tauri::command]
pub async fn list_projects(
    organization_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<OptionItem>> {
    let mut api = state.api()?;
    if !api
        .organizations()
        .await?
        .iter()
        .any(|o| o.id == organization_id)
    {
        return Err("组织不在当前令牌可访问列表中".into());
    }
    api.profile.organization_id = organization_id;
    api.profile.validate()?;
    crate::workspace::observed_error(&state, api.projects().await)
}
#[tauri::command]
pub async fn select_project(
    organization_id: String,
    project_id: String,
    state: State<'_, AppState>,
) -> Result<Profile> {
    crate::workspace::observed_error(
        &state,
        crate::workspace::select_project(&state, organization_id, project_id).await,
    )
}
#[tauri::command]
pub fn load_selection_memory(type_id: String, state: State<AppState>) -> Result<Value> {
    let m = state
        .metadata
        .lock()
        .unwrap()
        .clone()
        .ok_or("请先读取项目字段")?;
    if m.type_id != type_id {
        return Err("所选类型与当前字段配置不一致".into());
    }
    crate::workspace::check_metadata_scope(&state, &m)?;
    let key = crate::workspace::current_scope(&state, &type_id)?;
    let remembered: Value = state.store.get("selection", &key)?.unwrap_or(json!({}));
    Ok(crate::workspace::sanitize_selections(&remembered, &m))
}
#[tauri::command]
pub fn remember_selections(selection: Value, state: State<AppState>) -> Result<()> {
    if serde_json::to_vec(&selection)
        .map_err(|_| "选项格式错误")?
        .len()
        > 65536
    {
        return Err("选项记忆数据过大".into());
    }
    let m = state
        .metadata
        .lock()
        .unwrap()
        .clone()
        .ok_or("请先读取项目字段")?;
    if string(&selection, "workitemTypeId") != m.type_id {
        return Err("选项与当前Bug类型不一致，请刷新字段".into());
    }
    crate::workspace::check_metadata_scope(&state, &m)?;
    let key = crate::workspace::current_scope(&state, &m.type_id)?;
    let cleaned = crate::workspace::sanitize_selections(&selection, &m);
    if state
        .token
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|t| cleaned.to_string().contains(t))
    {
        return Err("选择记忆不得包含登录令牌".into());
    }
    state.store.put("selection", &key, &cleaned)?;
    state.store.put(
        "last_type",
        &crate::workspace::current_scope(&state, "")?,
        &m.type_id,
    )
}
#[tauri::command]
pub async fn list_remote_history(
    page: usize,
    filters: Option<crate::history::HistoryFilters>,
    project_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<crate::workspace::HistoryPage> {
    crate::history::list_for_state(
        &state,
        page,
        filters.unwrap_or_default(),
        project_id.as_deref(),
    )
    .await
}
#[tauri::command]
pub async fn import_profile(state: State<'_, AppState>) -> Result<Profile> {
    let file = rfd::AsyncFileDialog::new()
        .add_filter("项目配置", &["json"])
        .pick_file()
        .await
        .ok_or("已取消")?;
    let raw = tokio::fs::read(file.path())
        .await
        .map_err(|_| "配置读取失败")?;
    if raw.len() > 65536 {
        return Err("项目配置过大".into());
    }
    let p: Profile = serde_json::from_slice(&raw).map_err(|_| "项目配置格式错误")?;
    p.validate()?;
    let user = state
        .user
        .lock()
        .unwrap()
        .clone()
        .ok_or("请先验证登录令牌")?;
    state
        .store
        .put("settings", &format!("profile:{}", user.id), &p)?;
    state
        .workspace_epoch
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    *state.profile.lock().unwrap() = p.clone();
    *state.metadata.lock().unwrap() = None;
    Ok(p)
}
#[tauri::command]
pub fn new_draft(state: State<AppState>) -> Result<Value> {
    let scope = crate::workspace::current_scope(&state, "")?;
    let mut d: Value =
        serde_json::from_str(include_str!("../../schemas/empty-draft.json")).unwrap();
    d["id"] = json!(uuid());
    d["createdAt"] = json!(now());
    d["updatedAt"] = json!(now());
    crate::project_settings::apply_current_package(&state, &mut d)?;
    state.store.put("draft", &string(&d, "id"), &d)?;
    state.store.put("draft_scope", &string(&d, "id"), &scope)?;
    Ok(d)
}
#[tauri::command]
pub fn save_draft(mut draft: Value, state: State<AppState>) -> Result<Value> {
    let _write = state.store.writes.lock().unwrap();
    crate::project_settings::apply_current_package(&state, &mut draft)?;
    validation::check_shape(&draft)?;
    let id = string(&draft, "id");
    state.editable(&id)?;
    let old = state.store.draft(&id)?;
    draft["attachments"] = old["attachments"].clone();
    draft["createdAt"] = old["createdAt"].clone();
    draft["sessionId"] = old["sessionId"].clone();
    draft["collectionErrors"] = old["collectionErrors"].clone();
    draft["sources"] = old["sources"].clone();
    if string(&draft, "bugPage") != string(&old, "bugPage") {
        let prior = old["sources"]["bugPage"].clone();
        draft["sources"]["bugPage"] =
            json!({"status":"manual","at":now(),"detail":format!("人工输入；原始来源：{}",prior)});
    }
    draft["event"]["originalAt"] = old["event"]["originalAt"].clone();
    draft["event"]["source"] = old["event"]["source"].clone();
    for group in ["app", "device"] {
        for (key, v) in draft[group].as_object().unwrap().clone() {
            // The project mapping owns packageName. A renderer echo or a changed
            // setting must never make it look like a manually edited device value.
            if group == "app" && key == "packageName" {
                continue;
            }
            if old[group][&key] != v {
                let path = format!("{group}.{key}");
                let prior = old["sources"][&path].clone();
                draft["sources"][&path] = json!({"status":"manual","at":now(),"detail":format!("人工输入；原始来源：{}",prior)});
                if group == "app"
                    && ["packageName", "versionName", "versionCode"].contains(&key.as_str())
                    && prior["status"] == "collected"
                {
                    draft["collectionErrors"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!(format!(
                            "设备自动身份 {key} 被人工修改，原始值 {}",
                            old[group][&key]
                        )));
                }
            }
        }
    }
    // Protected sources were restored from storage above, so stamp the
    // authoritative mapping again immediately before persistence.
    crate::project_settings::apply_current_package(&state, &mut draft)?;
    draft["updatedAt"] = json!(now());
    state.assert_no_token(&draft)?;
    state.store.put("draft", &id, &draft)?;
    Ok(draft)
}
#[tauri::command]
pub fn validate_draft(id: String, state: State<AppState>) -> Result<Vec<Issue>> {
    let d = state.store.draft(&id)?;
    Ok(validation::validate(
        &d,
        &state.profile.lock().unwrap(),
        state.metadata.lock().unwrap().as_ref(),
        state.user.lock().unwrap().is_some(),
        true,
    ))
}
#[tauri::command]
pub async fn choose_adb(state: State<'_, AppState>) -> Result<String> {
    let file = rfd::AsyncFileDialog::new()
        .set_title("选择 Android SDK Platform Tools 中的 adb")
        .pick_file()
        .await
        .ok_or("已取消")?;
    let bytes = adb::output(file.path(), None, &["version"], 10).await?;
    if !String::from_utf8_lossy(&bytes).contains("Android Debug Bridge") {
        return Err("所选程序不是 ADB".into());
    }
    *state.adb.lock().unwrap() = Some(file.path().into());
    Ok(file.path().to_string_lossy().into())
}
#[tauri::command]
pub async fn list_devices(state: State<'_, AppState>) -> Result<Value> {
    adb::devices(&state.adb_path()?).await
}
#[tauri::command]
pub async fn inspect_device(
    id: String,
    serial: String,
    state: State<'_, AppState>,
) -> Result<Value> {
    state.editable(&id)?;
    let scope = crate::workspace::current_scope(&state, "")?;
    if state.store.get::<String>("draft_scope", &id)?.as_deref() != Some(&scope) {
        return Err("草稿不属于当前账号或项目".into());
    }
    let original_draft = state.store.draft(&id)?;
    let configured_package = crate::project_settings::current_package_for_state(&state)?;
    let epoch = state
        .workspace_epoch
        .load(std::sync::atomic::Ordering::SeqCst);
    adb::valid_serial(&serial)?;
    let path = state.adb_path()?;
    let listed = adb::devices(&path).await?;
    let selected = listed
        .as_array()
        .and_then(|rows| rows.iter().find(|row| string(row, "serial") == serial));
    match selected.map(|row| string(row, "state")).as_deref() {
        Some("device") => {
            let snapshot = adb::inspect_snapshot(&path, &serial, &configured_package).await?;
            let _write = state.store.writes.lock().unwrap();
            state.editable(&id)?;
            if epoch
                != state
                    .workspace_epoch
                    .load(std::sync::atomic::Ordering::SeqCst)
                || crate::workspace::current_scope(&state, "")? != scope
                || crate::project_settings::current_package_for_state(&state)? != configured_package
            {
                return Err("账号或项目已变化，请重新选择设备".into());
            }
            let mut draft = state.store.draft(&id)?;
            if draft != original_draft
                || string(&draft, "updatedAt") != string(&original_draft, "updatedAt")
            {
                return Err("读取设备期间草稿已被修改，请保存后重新读取设备".into());
            }
            let session_id = string(&draft, "sessionId");
            if !session_id.is_empty() {
                let session: Value = state
                    .store
                    .get("session", &session_id)?
                    .ok_or("草稿绑定的设备会话不存在，请新建草稿")?;
                if string(&session, "serial") != serial {
                    return Err("草稿已经绑定另一台设备，请新建草稿后读取设备信息".into());
                }
                let previous_package = string(&session, "packageName");
                let observed_package = string(&snapshot, "app.packageName");
                if previous_package != observed_package {
                    return Err("草稿已经绑定另一应用，请新建草稿后读取设备信息".into());
                }
                let previous_version = string(&draft, "app.versionName");
                let observed_version = string(&snapshot, "app.versionName");
                if previous_version != observed_version {
                    return Err(
                        "草稿已经绑定另一应用版本或版本信息不可用，请新建草稿后读取设备信息".into(),
                    );
                }
            }
            draft["device"] = snapshot["device"].clone();
            for (key, value) in snapshot["app"].as_object().unwrap() {
                draft["app"][key] = value.clone();
            }
            let mut sources = draft["sources"].as_object().cloned().unwrap_or_default();
            let keep_manual_page = string(&snapshot, "bugPage").is_empty()
                && sources
                    .get("bugPage")
                    .is_some_and(|source| source["status"] == "manual");
            if !keep_manual_page {
                draft["bugPage"] = snapshot["bugPage"].clone();
            }
            if let Some(observed) = snapshot["sources"].as_object() {
                sources.extend(
                    observed
                        .iter()
                        .filter(|(key, _)| !keep_manual_page || key.as_str() != "bugPage")
                        .map(|(key, value)| (key.clone(), value.clone())),
                );
            }
            draft["sources"] = json!(sources);
            crate::project_settings::apply_current_package(&state, &mut draft)?;
            let existing = draft["collectionErrors"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let mut errors = existing;
            for error in snapshot["errors"].as_array().into_iter().flatten() {
                if !errors.iter().any(|item| item == error) {
                    errors.push(error.clone());
                }
            }
            draft["collectionErrors"] = json!(errors);
            draft["previewConfirmed"] = json!(false);
            draft["updatedAt"] = json!(now());
            state
                .store
                .put_many(&[("draft", &id, &draft), ("snapshot", &id, &snapshot)])?;
            Ok(snapshot)
        }
        Some("offline") => Err("所选 USB 设备处于离线状态，请重新连接后重试".into()),
        Some("unauthorized") => Err("所选 USB 设备尚未授权，请在手机上允许 USB 调试".into()),
        Some(state) if !state.is_empty() => Err(format!("所选 USB 设备状态不可用：{state}")),
        _ => Err("所选 USB 设备已断开，请刷新设备列表".into()),
    }
}
#[tauri::command]
pub async fn start_session(serial: String, state: State<'_, AppState>) -> Result<Value> {
    crate::workspace::current_scope(&state, "")?;
    adb::valid_serial(&serial)?;
    let package_name = crate::project_settings::current_package_for_state(&state)?;
    let existing = { state.collector.lock().unwrap().take() };
    if let Some(c) = existing {
        adb::stop_logs(c).await;
    }
    let path = state.adb_path()?;
    let devices = adb::devices(&path).await?;
    if !devices
        .as_array()
        .unwrap()
        .iter()
        .any(|d| string(d, "serial") == serial && string(d, "state") == "device")
    {
        return Err("所选设备未授权、离线或断开，请检查 USB 调试".into());
    }
    let snapshot = adb::snapshot(&path, &serial, &package_name).await?;
    let session = json!({"id":uuid(),"serial":serial,"packageName":package_name,"startedAt":now(),"stoppedAt":null,"snapshot":snapshot,"logFiles":[],"gaps":[]});
    let collector = adb::start_logs(path, serial, state.store.clone(), session.clone()).await?;
    *state.collector.lock().unwrap() = Some(collector);
    Ok(session)
}
#[tauri::command]
pub async fn stop_session(state: State<'_, AppState>) -> Result<()> {
    let c = { state.collector.lock().unwrap().take() };
    if let Some(c) = c {
        adb::stop_logs(c).await;
    }
    Ok(())
}
#[tauri::command]
pub async fn mark_problem(id: String, state: State<'_, AppState>) -> Result<Value> {
    state.editable(&id)?;
    if crate::capture::pending(&state.store, &id)? {
        return Err("请等待当前证据采集完成后再记录问题".into());
    }
    let (sid, cancelled) = state
        .collector
        .lock()
        .unwrap()
        .as_ref()
        .map(|c| (c.id.clone(), c.stop.subscribe()))
        .ok_or("请先开始采集")?;
    let session: Value = state.store.get("session", &sid)?.ok_or("会话不存在")?;
    if !session["stoppedAt"].is_null() {
        return Err("会话已经中断，请重新连接建立会话".into());
    }
    let marked = now();
    let path = state.adb_path()?;
    let serial = string(&session, "serial");
    let snapshot = adb::snapshot(&path, &serial, &string(&session, "packageName")).await?;
    if session["snapshot"]["app"] != snapshot["app"] {
        stop_session(state.clone()).await?;
        return Err("安装包身份在采集中发生变化，请建立新会话".into());
    }
    let before = chrono::Utc::now().timestamp_micros();
    let clock = adb::output(&path, Some(&serial), &["shell", "date", "+%s"], 5).await;
    let after = chrono::Utc::now().timestamp_micros();
    let offset = clock
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|v| v.trim().parse::<i64>().ok())
        .and_then(|v| v.checked_mul(1_000_000))
        .and_then(|v| v.checked_sub(before + (after - before) / 2));
    let clock_note = if offset.is_some() {
        format!(
            "设备 epoch 秒与电脑请求中点估算；设备秒级精度，往返 {} ms，非精确同步",
            (after - before) / 1000
        )
    } else {
        "无法读取设备 epoch 时间；时间对应未知，不能宣称完整覆盖".into()
    };
    let temp = state.store.root.join(format!("capture-{}.png", uuid()));
    let screenshot = match adb::screenshot(&path, &serial, &temp).await {
        Ok(_) => evidence::inspect(
            &temp,
            "image",
            "ADB 问题时截图；保持原始分辨率并使用 PNG 无损压缩",
            &state.store.root,
        ),
        Err(e) => Err(e),
    };
    let _ = std::fs::remove_file(temp);
    let _write = state.store.writes.lock().unwrap();
    state.editable(&id)?;
    if crate::capture::pending(&state.store, &id)? {
        return Err("该问题已有待完成采集".into());
    }
    let mut d = state.store.draft(&id)?;
    let scope = crate::workspace::current_scope(&state, "")?;
    if state.store.get::<String>("draft_scope", &id)?.as_deref() != Some(&scope) {
        return Err("草稿不属于当前账号或项目".into());
    }
    d["sessionId"] = json!(sid);
    d["device"] = snapshot["device"].clone();
    for (k, v) in snapshot["app"].as_object().ok_or("采集应用信息无效")? {
        d["app"][k] = v.clone();
    }
    let page_source = d["sources"].get("bugPage").cloned();
    d["sources"] = snapshot["sources"].clone();
    if let Some(source) = page_source {
        d["sources"]["bugPage"] = source;
    }
    d["collectionErrors"] = snapshot["errors"].clone();
    d["event"] = json!({"occurredAt":marked,"originalAt":marked,"source":"auto","sourceNote":"电脑时钟标记；日志保存设备时钟估算依据"});
    d["previewConfirmed"] = json!(false);
    d["updatedAt"] = json!(now());
    match screenshot {
        Ok(a) => d["attachments"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::to_value(a).unwrap()),
        Err(e) => d["collectionErrors"].as_array_mut().unwrap().push(json!(e)),
    }
    state.store.put("snapshot", &id, &snapshot)?;
    crate::capture::schedule(
        &state.store,
        &d,
        &sid,
        &marked,
        offset,
        &clock_note,
        cancelled,
    )?;
    Ok(d)
}
#[tauri::command]
pub fn pin_logs(id: String, state: State<AppState>) -> Result<Value> {
    let _write = state.store.writes.lock().unwrap();
    state.editable(&id)?;
    if crate::capture::pending(&state.store, &id)? {
        return Err("正在等待问题后日志，请等待自动固定或先结束采集".into());
    }
    crate::capture::retry_interrupted(&state.store, &id)?;
    state.store.draft(&id)
}
#[tauri::command]
pub async fn capture_bugreport(id: String, state: State<'_, AppState>) -> Result<Value> {
    state.editable(&id)?;
    let d = state.store.draft(&id)?;
    let sid = string(&d, "sessionId");
    let session: Value = state
        .store
        .get("session", &sid)?
        .ok_or("请先记录问题并绑定设备会话")?;
    let scope = crate::workspace::current_scope(&state, "")?;
    if state.store.get::<String>("draft_scope", &id)?.as_deref() != Some(&scope) {
        return Err("草稿不属于当前账号或项目".into());
    }
    let path = state.adb_path()?;
    let task_id = uuid();
    {
        let _write = state.store.writes.lock().unwrap();
        state.editable(&id)?;
        if crate::capture::pending_kind(&state.store, &id, "bugreport")? {
            return Err("系统报告正在采集，请勿重复启动".into());
        }
        state.store.put("capture", &task_id, &json!({"id":task_id,"draftId":id,"sessionId":sid,"kind":"bugreport","status":"pending","startedAt":now()}))?;
    }
    let result = adb::bugreport(&path, &string(&session, "serial"), &state.store.root, 120).await;
    let _write = state.store.writes.lock().unwrap();
    let mut task: Value = state
        .store
        .get("capture", &task_id)?
        .ok_or("采集记录不存在")?;
    task["finishedAt"] = json!(now());
    let mut d = state.store.draft(&id)?;
    match result {
        Ok(c) => {
            task["status"] = json!(if c.attachment.is_some() {
                "complete"
            } else {
                "failed"
            });
            task["receipt"] = serde_json::to_value(&c).map_err(|e| e.to_string())?;
            if let Some(a) = c.attachment {
                d["attachments"]
                    .as_array_mut()
                    .unwrap()
                    .push(serde_json::to_value(a).unwrap());
            } else {
                d["collectionErrors"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!(c.failure.unwrap_or("系统报告未完成".into())));
            }
        }
        Err(e) => {
            task["status"] = json!("failed");
            task["error"] = json!(e);
            d["collectionErrors"].as_array_mut().unwrap().push(json!(e));
        }
    }
    state.editable(&id)?;
    d["previewConfirmed"] = json!(false);
    d["updatedAt"] = json!(now());
    state
        .store
        .put_many(&[("draft", &id, &d), ("capture", &task_id, &task)])?;
    Ok(d)
}
#[tauri::command]
pub fn redact_log(
    id: String,
    attachment_id: String,
    literal: String,
    state: State<AppState>,
) -> Result<Value> {
    let _write = state.store.writes.lock().unwrap();
    state.editable(&id)?;
    let mut d = state.store.draft(&id)?;
    let original = attachments(&d)?
        .into_iter()
        .find(|a| a.id == attachment_id)
        .ok_or("日志不存在")?;
    let mut redacted = evidence::redact_text(
        &state.store.root,
        &original,
        &[evidence::TextRedactionRule {
            label: "用户指定文本".into(),
            literal,
        }],
    )?;
    if original.source.starts_with("ADB 问题日志窗口派生副本")
        || original.source.contains("来源为问题日志窗口")
    {
        redacted.attachment.source.push_str("；来源为问题日志窗口");
    }
    let receipt = crate::capture::paired_report(
        &state.store,
        &redacted.receipt_path,
        &redacted.attachment.id,
        "文本替换记录（请核对副本）",
    )?;
    let mut inherited = Vec::new();
    for a in attachments(&d)? {
        if a.role == "report" && a.derived_from.as_deref() == Some(&original.id) {
            let mut copy = evidence::inspect(
                std::path::Path::new(&a.path),
                "report",
                &a.source,
                &state.store.root,
            )?;
            copy.derived_from = Some(redacted.attachment.id.clone());
            inherited.push(serde_json::to_value(copy).unwrap());
        }
    }
    // The draft uploads every listed file. Replace the source entry with the
    // redacted copy while preserving its immutable file on disk.
    d["attachments"].as_array_mut().unwrap().retain(|a| {
        string(a, "id") != original.id
            && !(string(a, "derivedFrom") == original.id && string(a, "role") == "report")
    });
    d["attachments"].as_array_mut().unwrap().extend([
        serde_json::to_value(redacted.attachment).unwrap(),
        serde_json::to_value(receipt).unwrap(),
    ]);
    d["attachments"].as_array_mut().unwrap().extend(inherited);
    d["previewConfirmed"] = json!(false);
    d["updatedAt"] = json!(now());
    state.store.put("draft", &id, &d)?;
    Ok(d)
}
#[tauri::command]
pub async fn import_attachment(id: String, state: State<'_, AppState>) -> Result<Value> {
    state.editable(&id)?;
    check_attachment_scope(&state, &id)?;
    let files = rfd::AsyncFileDialog::new()
        .set_title("选择证据文件")
        .pick_files()
        .await
        .ok_or("已取消")?;
    let paths = files
        .iter()
        .map(|file| file.path().to_path_buf())
        .collect::<Vec<_>>();
    import_attachment_paths(&state, &id, &paths)
}

pub(crate) fn import_attachment_paths(
    state: &AppState,
    id: &str,
    paths: &[PathBuf],
) -> Result<Value> {
    let _write = state.store.writes.lock().unwrap();
    state.editable(id)?;
    check_attachment_scope(state, id)?;
    let mut d = state.store.draft(id)?;
    for path in paths {
        let a = evidence::inspect_import(path, &state.store.root).map_err(|error| {
            format!(
                "导入 {} 失败：{error}",
                path.file_name().unwrap_or_default().to_string_lossy()
            )
        })?;
        d["attachments"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::to_value(a).unwrap());
    }
    d["previewConfirmed"] = json!(false);
    state.store.put("draft", id, &d)?;
    Ok(d)
}
#[tauri::command]
pub async fn preview_attachment(
    id: String,
    attachment_id: String,
    state: State<'_, AppState>,
) -> Result<()> {
    let a = preview_attachment_for_state(&state, &id, &attachment_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        if uses_macos_quick_look(&a) {
            evidence::verify_attachment(&a)?;
            return std::process::Command::new("/usr/bin/qlmanage")
                .args(["-p", &a.path])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map(|_| ())
                .map_err(|_| "无法使用 macOS 快速查看预览 ZIP 报告".to_string());
        }
        open_attachment_with(&a, |path, application| match application {
            Some(application) => open::with(path, application),
            None => open::that(path),
        })
    })
    .await
    .map_err(|_| "预览启动失败".to_string())?
}

pub(crate) fn uses_macos_quick_look(a: &Attachment) -> bool {
    cfg!(target_os = "macos") && a.role == "report" && a.mime == "application/zip"
}

fn check_attachment_scope(state: &AppState, id: &str) -> Result<()> {
    let scope = crate::workspace::current_scope(state, "")?;
    if state.store.get::<String>("draft_scope", id)?.as_deref() != Some(&scope) {
        return Err("附件不属于当前账号或项目".into());
    }
    Ok(())
}

pub(crate) fn open_attachment_with(
    a: &Attachment,
    open: impl FnOnce(&std::path::Path, Option<&str>) -> std::io::Result<()>,
) -> Result<()> {
    evidence::verify_attachment(a)?;
    // A user's file association may point images to an app that cannot preview them.
    // Use Apple's image viewer explicitly; ZIP reports are routed to Quick Look by
    // preview_attachment so Archive Utility does not expand them beside the evidence.
    let application = if cfg!(target_os = "macos")
        && a.role == "image"
        && ["image/png", "image/jpeg"].contains(&a.mime.as_str())
    {
        Some("/System/Applications/Preview.app")
    } else {
        None
    };
    open(std::path::Path::new(&a.path), application).map_err(|_| {
        if application.is_some() {
            "无法使用 macOS“预览”打开此图片，请确认系统“预览”应用可用后重试".to_string()
        } else {
            "无法使用系统应用打开此附件，请设置该格式的默认打开应用后重试".to_string()
        }
    })
}

pub(crate) fn preview_attachment_for_state(
    state: &AppState,
    id: &str,
    attachment_id: &str,
) -> Result<Attachment> {
    check_attachment_scope(state, id)?;
    let d = state.store.draft(id)?;
    let mut a = attachments(&d)?
        .into_iter()
        .find(|a| a.id == attachment_id)
        .ok_or("附件不存在")?;
    let root = state
        .store
        .root
        .join("attachments")
        .canonicalize()
        .map_err(|_| "附件目录不存在")?;
    let path = PathBuf::from(&a.path)
        .canonicalize()
        .map_err(|_| "附件文件不存在")?;
    if !path.starts_with(&root) {
        return Err("只能预览当前草稿的本地附件".into());
    }
    evidence::verify_attachment(&a)?;
    let extension = path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    use std::io::Read;
    let mut header = [0u8; 4];
    let len = std::fs::File::open(&path)
        .map_err(|_| "附件文件无法读取")?
        .read(&mut header)
        .map_err(|_| "附件文件无法读取")?;
    let h = &header[..len];
    if [
        "exe",
        "com",
        "bat",
        "cmd",
        "ps1",
        "command",
        "sh",
        "desktop",
        "app",
        "lnk",
        "url",
        "msi",
        "vbs",
        "js",
        "py",
        "pyw",
        "rb",
        "pl",
        "jar",
        "hta",
        "scpt",
        "applescript",
        "bash",
        "zsh",
        "fish",
    ]
    .contains(&extension.as_str())
        || h.starts_with(b"MZ")
        || h.starts_with(b"\x7fELF")
        || h.starts_with(b"#!")
        || [
            b"\xfe\xed\xfa\xce",
            b"\xce\xfa\xed\xfe",
            b"\xfe\xed\xfa\xcf",
            b"\xcf\xfa\xed\xfe",
            b"\xca\xfe\xba\xbe",
            b"\xbe\xba\xfe\xca",
        ]
        .iter()
        .any(|magic| h.starts_with(*magic))
    {
        return Err("此附件是程序或脚本，不能作为预览启动".into());
    }
    a.path = path.to_string_lossy().into_owned();
    Ok(a)
}
#[tauri::command]
pub fn save_attachment_coverage(
    id: String,
    attachment_id: String,
    coverage_start: String,
    coverage_end: String,
    state: State<AppState>,
) -> Result<Value> {
    confirm_attachment(id, attachment_id, true, coverage_start, coverage_end, state)
}
pub fn confirm_attachment(
    id: String,
    attachment_id: String,
    confirmed: bool,
    coverage_start: String,
    coverage_end: String,
    state: State<AppState>,
) -> Result<Value> {
    let _write = state.store.writes.lock().unwrap();
    state.editable(&id)?;
    let mut d = state.store.draft(&id)?;
    let row = d["attachments"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|a| string(a, "id") == attachment_id)
        .ok_or("附件不存在")?;
    let a: Attachment = serde_json::from_value(row.clone()).map_err(|_| "附件数据错误")?;
    evidence::verify_attachment(&a)?;
    row["confirmed"] = json!(confirmed);
    row["coverageStart"] = json!(coverage_start);
    row["coverageEnd"] = json!(coverage_end);
    let owner = if a.role == "report" {
        a.derived_from.as_deref().unwrap_or(&attachment_id)
    } else {
        &attachment_id
    };
    for paired in d["attachments"].as_array_mut().unwrap() {
        if string(paired, "id") == owner
            || (string(paired, "role") == "report" && string(paired, "derivedFrom") == owner)
        {
            paired["confirmed"] = json!(confirmed);
        }
    }
    d["previewConfirmed"] = json!(false);
    validation::check_shape(&d)?;
    state.assert_no_token(&d)?;
    state.store.put("draft", &id, &d)?;
    Ok(d)
}
#[tauri::command]
pub fn remove_attachment(
    id: String,
    attachment_id: String,
    state: State<AppState>,
) -> Result<Value> {
    let _write = state.store.writes.lock().unwrap();
    state.editable(&id)?;
    let mut d = state.store.draft(&id)?;
    let files = attachments(&d)?;
    let target = files
        .iter()
        .find(|a| a.id == attachment_id)
        .ok_or("附件不存在")?;
    if target.role == "report"
        && target
            .derived_from
            .as_ref()
            .is_some_and(|owner| files.iter().any(|a| &a.id == owner))
    {
        return Err("来源回执须与关联副本一起保留；请移除关联副本以成组移除".into());
    }
    d["attachments"].as_array_mut().unwrap().retain(|a| {
        string(a, "id") != attachment_id
            && !(string(a, "role") == "report" && string(a, "derivedFrom") == attachment_id)
    });
    d["previewConfirmed"] = json!(false);
    state.store.put("draft", &id, &d)?;
    Ok(d)
}
#[tauri::command]
pub async fn export_evidence(
    id: String,
    include_media: bool,
    state: State<'_, AppState>,
) -> Result<String> {
    let d = state.store.draft(&id)?;
    state.assert_no_token(&d)?;
    let e = evidence::build(
        &state.store,
        &d,
        &state.profile.lock().unwrap(),
        include_media,
    )?;
    let file = rfd::AsyncFileDialog::new()
        .set_file_name(format!("evidence-{}.zip", e.id))
        .save_file()
        .await
        .ok_or("已取消")?;
    std::fs::copy(&e.zip_path, file.path()).map_err(|e| e.to_string())?;
    Ok(file.path().to_string_lossy().into())
}
#[tauri::command]
pub async fn submit_bug(id: String, state: State<'_, AppState>) -> Result<Submission> {
    submit_with_state(id, &state).await
}
pub(crate) async fn submit_with_state(id: String, state: &AppState) -> Result<Submission> {
    {
        let mut locks = state.locks.lock().unwrap();
        if !locks.insert(id.clone()) {
            return Err("该草稿正在提交，请勿重复点击".into());
        }
    }
    let result = async {
        let api = crate::workspace::submission_api(state, &id)?;
        if crate::capture::pending(&state.store, &id)? {
            return Err("证据仍在采集中，请等待采集完成或中断记录落盘后提交".into());
        }
        let d = state.store.draft(&id)?;
        let configured_package = crate::project_settings::current_package_for_state(state)?;
        if string(&d, "app.packageName") != configured_package {
            return Err("项目包名配置已变化，请返回新建提交页保存草稿后再提交".into());
        }
        state.assert_no_token(&d)?;
        Submitter {
            store: state.store.clone(),
        }
        .submit(&id, &api)
        .await
    }
    .await;
    state.locks.lock().unwrap().remove(&id);
    result
}
#[tauri::command]
pub async fn reconcile_submission(
    id: String,
    remote_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Submission> {
    reconcile_with_state(id, remote_id, &state).await
}
pub(crate) async fn reconcile_with_state(
    id: String,
    remote_id: Option<String>,
    state: &AppState,
) -> Result<Submission> {
    {
        let mut locks = state.locks.lock().unwrap();
        if !locks.insert(id.clone()) {
            return Err("正在核对该提交".into());
        }
    }
    let result = async {
        let api = crate::workspace::submission_api(state, &id)?;
        Submitter {
            store: state.store.clone(),
        }
        .reconcile(&id, &api, remote_id)
        .await
    }
    .await;
    state.locks.lock().unwrap().remove(&id);
    result
}
#[tauri::command]
pub fn copy_receipt(id: String, repair: bool, state: State<AppState>) -> Result<()> {
    let s: Submission = state.store.get("submission", &id)?.ok_or("提交不存在")?;
    let d = state.store.draft(&id)?;
    let text = crate::workitem_links::receipt_text(&s, &d, repair)?;
    arboard::Clipboard::new()
        .map_err(|_| "剪贴板不可用")?
        .set_text(text)
        .map_err(|_| "复制失败".into())
}
#[tauri::command]
pub fn open_receipt(id: String, state: State<AppState>) -> Result<()> {
    let s: Submission = state.store.get("submission", &id)?.ok_or("提交不存在")?;
    let url = crate::workitem_links::bug_url(s.url.as_deref(), s.identifier.as_deref())
        .ok_or("云效暂未提供可用工作项编号或链接")?;
    let u = reqwest::Url::parse(&url).map_err(|_| "链接不合法")?;
    if u.scheme() != "https" || u.host_str() != Some("devops.aliyun.com") {
        return Err("只允许打开已确认的云效链接".into());
    }
    open::that(url).map_err(|_| "打开浏览器失败".into())
}
pub fn setup(app: &mut tauri::App) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let root = app.path().app_local_data_dir()?;
    let state = AppState::new(root).map_err(std::io::Error::other)?;
    app.manage(state);
    Ok(())
}
