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
        Ok(Self {
            store,
            profile: Mutex::new(profile),
            // Startup always requires manual entry, even if an older version saved a PAT.
            token: Mutex::new(None),
            user: Mutex::new(None),
            auth_epoch: AtomicU64::new(0),
            workspace_epoch: AtomicU64::new(0),
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
        Api::with_base(p, t.as_deref().ok_or("请填写个人 PAT")?, &self.api_base)
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
        let token = self.token.lock().unwrap().clone();
        if let Some(mut t) = token {
            if t.len() >= 8 {
                if d.to_string().contains(&t) {
                    t.zeroize();
                    return Err("草稿包含当前 PAT，请移除凭据后再保存或上传".into());
                }
                for a in attachments(d)?.iter().filter(|a| a.confirmed) {
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
    Ok(
        json!({"profile":state.profile.lock().unwrap().clone(),"authorized":authorized,"user":user,"metadata":state.metadata.lock().unwrap().clone(),"adbPath":state.adb.lock().unwrap().as_ref().map(|v|v.to_string_lossy()),"drafts":state.store.list("draft")?,"submissions":state.store.list("submission")?,"sessions":state.store.list("session")?,"captures":state.store.list("capture")?}),
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
    stop_session(state.clone()).await?;
    state.store.put("settings", "remembered", &false)?;
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
    state: State<'_, AppState>,
) -> Result<crate::workspace::HistoryPage> {
    crate::workspace::current_scope(&state, "")?;
    let user = state
        .user
        .lock()
        .unwrap()
        .clone()
        .ok_or("请先验证登录令牌")?;
    crate::workspace::observed_error(&state, state.api()?.history(&user.id, page).await)
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
    state.store.put("draft", &string(&d, "id"), &d)?;
    state.store.put("draft_scope", &string(&d, "id"), &scope)?;
    Ok(d)
}
#[tauri::command]
pub fn save_draft(mut draft: Value, state: State<AppState>) -> Result<Value> {
    let _write = state.store.writes.lock().unwrap();
    validation::check_shape(&draft)?;
    let id = string(&draft, "id");
    state.editable(&id)?;
    let old = state.store.draft(&id)?;
    draft["attachments"] = old["attachments"].clone();
    draft["createdAt"] = old["createdAt"].clone();
    draft["sessionId"] = old["sessionId"].clone();
    draft["collectionErrors"] = old["collectionErrors"].clone();
    draft["sources"] = old["sources"].clone();
    draft["event"]["originalAt"] = old["event"]["originalAt"].clone();
    draft["event"]["source"] = old["event"]["source"].clone();
    draft["buildManifest"] = old["buildManifest"].clone();
    if string(&draft, "buildStatus") == "verified" && string(&old, "buildStatus") != "verified" {
        return Err("首版不能通过人工编辑宣称构建身份已精确验证".into());
    }
    for group in ["app", "device"] {
        for (key, v) in draft[group].as_object().unwrap().clone() {
            if old[group][&key] != v {
                let path = format!("{group}.{key}");
                let prior = old["sources"][&path].clone();
                draft["sources"][&path] = json!({"status":"manual","at":now(),"detail":format!("人工输入，来源：{}；原始来源：{}",string(&draft,"manualNote"),prior)});
                if group == "app"
                    && ["packageName", "versionName", "versionCode"].contains(&key.as_str())
                    && prior["status"] == "collected"
                {
                    draft["buildStatus"] = json!("conflict");
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
    if let Some(snapshot) = state.store.get::<Value>("snapshot", &id)? {
        if ["packageName", "versionName", "versionCode"]
            .iter()
            .any(|k| {
                snapshot["app"][k]
                    .as_str()
                    .is_some_and(|v| !v.is_empty() && snapshot["app"][k] != draft["app"][k])
            })
        {
            draft["buildStatus"] = json!("conflict");
        }
    }
    if !old["sessionId"].is_null() {
        let session: Value = state
            .store
            .get("session", &string(&old, "sessionId"))?
            .ok_or("会话不存在")?;
        if let (Ok(event), Ok(start)) = (
            chrono::DateTime::parse_from_rfc3339(&string(&draft, "event.occurredAt")),
            chrono::DateTime::parse_from_rfc3339(&string(&session, "startedAt")),
        ) {
            if event < start {
                draft["event"]["lateCapture"] = json!(true);
                draft["event"]["source"] = json!("manual");
            }
        }
    }
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
pub async fn start_session(
    serial: String,
    package_name: String,
    build_url: String,
    business: String,
    state: State<'_, AppState>,
) -> Result<Value> {
    crate::workspace::current_scope(&state, "")?;
    adb::valid_serial(&serial)?;
    adb::valid_package(&package_name)?;
    if validation::fixed_build_url(&build_url).is_none() {
        return Err("请填写 Jenkins 固定构建 URL".into());
    }
    if !state
        .profile
        .lock()
        .unwrap()
        .business_profiles
        .contains(&business)
    {
        return Err("请选择配置中的业务方向".into());
    }
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
    let session = json!({"id":uuid(),"serial":serial,"packageName":package_name,"buildUrl":build_url,"business":business,"startedAt":now(),"stoppedAt":null,"snapshot":snapshot,"logFiles":[],"gaps":[]});
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
        Ok(()) => evidence::inspect(&temp, "image", "ADB 问题时截图", &state.store.root),
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
    d["business"] = session["business"].clone();
    d["buildUrl"] = session["buildUrl"].clone();
    d["device"] = snapshot["device"].clone();
    for (k, v) in snapshot["app"].as_object().ok_or("采集应用信息无效")? {
        d["app"][k] = v.clone();
    }
    d["sources"] = snapshot["sources"].clone();
    d["collectionErrors"] = snapshot["errors"].clone();
    d["event"] = json!({"occurredAt":marked,"originalAt":marked,"source":"auto","sourceNote":"电脑时钟标记；日志保存设备时钟估算依据","lateCapture":false});
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
    let redacted = evidence::redact_text(
        &state.store.root,
        &original,
        &[evidence::TextRedactionRule {
            label: "用户指定文本".into(),
            literal,
        }],
    )?;
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
    for a in d["attachments"].as_array_mut().unwrap() {
        if string(a, "id") == original.id
            || (string(a, "derivedFrom") == original.id && string(a, "role") == "report")
        {
            a["confirmed"] = json!(false);
        }
    }
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
pub async fn import_attachment(
    id: String,
    role: String,
    state: State<'_, AppState>,
) -> Result<Value> {
    state.editable(&id)?;
    let files = rfd::AsyncFileDialog::new()
        .set_title("选择证据文件")
        .pick_files()
        .await
        .ok_or("已取消")?;
    let _write = state.store.writes.lock().unwrap();
    state.editable(&id)?;
    let mut d = state.store.draft(&id)?;
    for f in files {
        let a = evidence::inspect(
            f.path(),
            &role,
            "导入文件，覆盖时间由测试人员填写",
            &state.store.root,
        )?;
        d["attachments"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::to_value(a).unwrap());
    }
    d["previewConfirmed"] = json!(false);
    state.store.put("draft", &id, &d)?;
    Ok(d)
}
#[tauri::command]
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
pub fn read_log(id: String, attachment_id: String, state: State<AppState>) -> Result<String> {
    use std::io::Read;
    let d = state.store.draft(&id)?;
    let a = attachments(&d)?
        .into_iter()
        .find(|a| a.id == attachment_id && a.role == "log")
        .ok_or("日志不存在")?;
    let mut bytes = Vec::new();
    std::fs::File::open(a.path)
        .map_err(|_| "无法读取日志")?
        .take(100_000)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&bytes).to_string();
    let token = state.token.lock().unwrap();
    Ok(if let Some(t) = token.as_ref() {
        text.replace(t, "[PAT 已隐藏]")
    } else {
        text
    })
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
    let text = if repair {
        format!("请修复 Android 缺陷：{}\n业务方向：{}\n云效工作项：{}\n链接：{}\n版本：{}；迭代：{}\nJenkins：{}\nevidenceId：{}\n证据状态：{}",string(&d,"title"),string(&d,"business"),s.workitem_id.unwrap_or_default(),s.url.unwrap_or("云效 API 未返回真实链接，请从项目中打开".into()),d["versions"],string(&d,"sprint"),string(&d,"buildUrl"),s.evidence.id,s.state)
    } else {
        s.url
            .ok_or("云效 API 未返回真实链接；请按工作项 ID 在项目中查找")?
    };
    arboard::Clipboard::new()
        .map_err(|_| "剪贴板不可用")?
        .set_text(text)
        .map_err(|_| "复制失败".into())
}
#[tauri::command]
pub fn open_receipt(id: String, state: State<AppState>) -> Result<()> {
    let s: Submission = state.store.get("submission", &id)?.ok_or("提交不存在")?;
    let url = s.url.ok_or("云效未返回真实缺陷链接")?;
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
