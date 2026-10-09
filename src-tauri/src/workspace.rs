use crate::{commands::AppState, model::*, yunxiao::Api};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use zeroize::Zeroize;

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UserIdentity {
    pub id: String,
    pub name: String,
    pub last_organization: String,
}
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AuthResult {
    pub status: String,
    pub user: Option<UserIdentity>,
    pub message: String,
}
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HistoryItem {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    pub items: Vec<HistoryItem>,
    pub page: usize,
    pub has_more: bool,
}

pub fn empty_profile() -> Profile {
    Profile {
        name: "请选择项目".into(),
        edition: "central".into(),
        organization_id: "unconfigured".into(),
        project_id: "unconfigured".into(),
        business_profiles: vec!["Android 测试".into()],
        required_field_ids: vec![],
        field_mappings: Default::default(),
        enable_versions: true,
        enable_sprints: true,
        attachment_warning_mb: 50.0,
    }
}
fn auth_result(status: &str, user: Option<UserIdentity>, message: &str) -> AuthResult {
    AuthResult {
        status: status.into(),
        user,
        message: message.into(),
    }
}
pub fn invalidate(state: &AppState) {
    let mut user = state.user.lock().unwrap();
    state.auth_epoch.fetch_add(1, Ordering::SeqCst);
    *user = None;
    drop(user);
    state.workspace_epoch.fetch_add(1, Ordering::SeqCst);
    *state.metadata.lock().unwrap() = None;
}
pub fn observed_error<T>(state: &AppState, result: Result<T>) -> Result<T> {
    if result
        .as_ref()
        .err()
        .is_some_and(|e| e.contains("[授权失效]"))
    {
        invalidate(state);
    }
    result
}
fn login_failure(error: &str) -> AuthResult {
    let status = if error.contains("[授权失效]") {
        "invalid"
    } else {
        "unverified"
    };
    // Error strings are produced locally; never return the server body/PAT.
    auth_result(status, None, error)
}
fn accept_identity(state: &AppState, user: UserIdentity, epoch: u64) -> Result<AuthResult> {
    let mut current_user = state.user.lock().unwrap();
    if epoch != state.auth_epoch.load(Ordering::SeqCst) {
        return Err("本次登录已取消，请重新校验".into());
    }
    let p = state
        .store
        .get::<Profile>("settings", &format!("profile:{}", user.id))?
        .unwrap_or_else(empty_profile);
    p.validate()?;
    *state.profile.lock().unwrap() = p;
    *current_user = Some(user.clone());
    Ok(auth_result("valid", Some(user), "令牌校验成功"))
}
pub async fn check_auth(state: &AppState) -> Result<AuthResult> {
    invalidate(state);
    let epoch = state.auth_epoch.load(Ordering::SeqCst);
    let token = state.token.lock().unwrap().clone();
    let Some(mut token) = token else {
        return Ok(auth_result("missing", None, "请输入个人访问令牌"));
    };
    let result = Api::with_base(empty_profile(), &token, &state.api_base)?
        .verify_user()
        .await;
    token.zeroize();
    match result {
        Ok(user) => accept_identity(state, user, epoch),
        Err(e) => Ok(login_failure(&e)),
    }
}
pub async fn login(state: &AppState, mut token: String, remember: bool) -> Result<AuthResult> {
    invalidate(state);
    let collector = { state.collector.lock().unwrap().take() };
    if let Some(collector) = collector {
        crate::adb::stop_logs(collector).await;
    }
    let epoch = state.auth_epoch.load(Ordering::SeqCst);
    token = token.trim().to_string();
    if token.len() < 8 || token.len() > 4096 || token.contains(['\n', '\r']) {
        token.zeroize();
        return Ok(auth_result("invalid", None, "令牌格式不正确"));
    }
    let verified = Api::with_base(empty_profile(), &token, &state.api_base)?
        .verify_user()
        .await;
    let user = match verified {
        Ok(user) => user,
        Err(e) => {
            token.zeroize();
            return Ok(login_failure(&e));
        }
    };
    if epoch != state.auth_epoch.load(Ordering::SeqCst) {
        token.zeroize();
        return Err("本次登录已取消，请重新校验".into());
    }
    if remember {
        let entry = keyring::Entry::new(super::commands::SERVICE, "personal-pat")
            .map_err(|_| "系统安全存储不可用，请选择仅本次使用")?;
        if entry.set_password(&token).is_err() {
            token.zeroize();
            return Ok(auth_result(
                "unverified",
                None,
                "系统安全存储不可用，未保存令牌；请选择仅本次使用",
            ));
        }
    } else if state
        .store
        .get::<bool>("settings", "remembered")?
        .unwrap_or(false)
    {
        let entry = keyring::Entry::new(super::commands::SERVICE, "personal-pat")
            .map_err(|_| "无法移除系统保存的令牌，请检查安全存储")?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(_) => {
                token.zeroize();
                return Err("无法移除系统保存的令牌，请检查安全存储".into());
            }
        }
    }
    state.store.put("settings", "remembered", &remember)?;
    let mut old = state.token.lock().unwrap();
    if let Some(value) = old.as_mut() {
        value.zeroize();
    }
    *old = Some(token);
    drop(old);
    accept_identity(state, user, epoch)
}
pub fn scope_key(user: &UserIdentity, profile: &Profile, type_id: &str) -> String {
    format!(
        "{}:{}:{}:{}",
        user.id, profile.organization_id, profile.project_id, type_id
    )
}
pub fn current_scope(state: &AppState, type_id: &str) -> Result<String> {
    let user = state
        .user
        .lock()
        .unwrap()
        .clone()
        .ok_or("请先验证登录令牌")?;
    let p = state.profile.lock().unwrap().clone();
    if p.project_id == "unconfigured" {
        return Err("请选择组织与项目".into());
    }
    Ok(scope_key(&user, &p, type_id))
}
pub fn check_metadata_scope(state: &AppState, metadata: &Metadata) -> Result<()> {
    let profile = state.profile.lock().unwrap();
    if metadata.profile.organization_id != profile.organization_id
        || metadata.profile.project_id != profile.project_id
    {
        return Err("项目已变化，请重新读取字段".into());
    }
    Ok(())
}
/// Only real selectable values are retained; no description, credentials, or arbitrary text.
pub fn sanitize_selections(selection: &Value, metadata: &Metadata) -> Value {
    let mut result = json!({"workitemTypeId":metadata.type_id,"assignedTo":"","sprint":"","versions":[],"customFields":{}});
    for (field, choices, multiple) in [
        ("assignedTo", &metadata.members, false),
        ("sprint", &metadata.sprints, false),
        ("versions", &metadata.versions, true),
    ] {
        let values = selection[field]
            .as_array()
            .cloned()
            .unwrap_or_else(|| vec![selection[field].clone()]);
        let valid: Vec<Value> = values
            .into_iter()
            .filter(|v| choices.iter().any(|o| v.as_str() == Some(&o.id)))
            .collect();
        result[field] = if multiple {
            json!(valid)
        } else {
            valid.first().cloned().unwrap_or(json!(""))
        };
    }
    let kinds = [
        "功能",
        "UI",
        "多语言",
        "动态过程",
        "崩溃",
        "ANR",
        "性能",
        "网络",
        "数据",
    ];
    if kinds.contains(&string(selection, "kind").as_str()) {
        result["kind"] = selection["kind"].clone();
    }
    if metadata
        .profile
        .business_profiles
        .contains(&string(selection, "business"))
    {
        result["business"] = selection["business"].clone();
    }
    for field in &metadata.fields {
        if !field.show_when_create && !field.required {
            continue;
        }
        let (options, multiple) = match field.format.as_str() {
            "user" => (&metadata.members, false),
            "multiUser" => (&metadata.members, true),
            "sprint" => (&metadata.sprints, false),
            "version" => (&metadata.versions, true),
            "list" => (&field.options, false),
            "multiList" | "tag" => (&field.options, true),
            _ => continue,
        };
        let value = &selection["customFields"][&field.id];
        let values = value
            .as_array()
            .cloned()
            .unwrap_or_else(|| vec![value.clone()]);
        let valid: Vec<Value> = values
            .into_iter()
            .filter(|v| options.iter().any(|o| v.as_str() == Some(&o.id)))
            .collect();
        if !valid.is_empty() {
            result["customFields"][&field.id] = if multiple {
                json!(valid)
            } else {
                valid[0].clone()
            };
        }
    }
    result
}
pub async fn select_project(
    state: &AppState,
    organization_id: String,
    project_id: String,
) -> Result<Profile> {
    if state.collector.lock().unwrap().is_some() || !state.locks.lock().unwrap().is_empty() {
        return Err("请先停止设备采集并等待本次提交结束，再切换项目".into());
    }
    let epoch = state.auth_epoch.load(Ordering::SeqCst);
    let api = state.api()?;
    let user = state
        .user
        .lock()
        .unwrap()
        .clone()
        .ok_or("请先验证登录令牌")?;
    if !api
        .organizations()
        .await?
        .iter()
        .any(|o| o.id == organization_id)
    {
        return Err("组织不在当前令牌可访问列表中".into());
    }
    let mut profile = empty_profile();
    profile.organization_id = organization_id;
    profile.validate()?;
    let mut scoped_api = api;
    scoped_api.profile = profile.clone();
    let projects = scoped_api.projects().await?;
    let project = projects
        .iter()
        .find(|p| p.id == project_id)
        .ok_or("项目不在当前令牌可访问列表中")?;
    let previous = state.profile.lock().unwrap().clone();
    if previous.organization_id == profile.organization_id && previous.project_id == project_id {
        profile = previous;
    }
    profile.name = project.name.clone();
    profile.project_id = project.id.clone();
    profile.validate()?;
    if epoch != state.auth_epoch.load(Ordering::SeqCst) {
        return Err("登录已变化，请重新选择项目".into());
    }
    if state.collector.lock().unwrap().is_some() || !state.locks.lock().unwrap().is_empty() {
        return Err("正在采集或提交，暂时不能切换项目".into());
    }
    state
        .store
        .put("settings", &format!("profile:{}", user.id), &profile)?;
    state.workspace_epoch.fetch_add(1, Ordering::SeqCst);
    *state.profile.lock().unwrap() = profile.clone();
    *state.metadata.lock().unwrap() = None;
    Ok(profile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        matchers::{body_partial_json, header, method, path},
        Mock, MockServer, ResponseTemplate,
    };
    fn state(server: &MockServer, root: &std::path::Path) -> AppState {
        let mut s = AppState::new(root.into()).unwrap();
        s.api_base = server.uri();
        s
    }
    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "requires explicit read-only permission and the named test Keychain credential"]
    async fn authorized_live_readonly_contract() {
        assert_eq!(
            std::env::var("ANDROIDBUGDESK_RUN_AUTHORIZED_READONLY").as_deref(),
            Ok("1")
        );
        let output = std::process::Command::new("security")
            .args([
                "find-generic-password",
                "-s",
                "android-debug-desk-test-commit-bug",
                "-w",
            ])
            .output()
            .expect("cannot access named credential");
        assert!(output.status.success(), "named credential unavailable");
        let mut token = String::from_utf8(output.stdout).expect("credential encoding");
        let mut profile = empty_profile();
        let api = Api::new(profile.clone(), token.trim()).unwrap();
        let user = api.verify_user().await.unwrap();
        let orgs = api.organizations().await.unwrap();
        assert_eq!(
            orgs.len(),
            1,
            "select an explicit organization before extending this test"
        );
        profile.organization_id = orgs[0].id.clone();
        let api = Api::new(profile.clone(), token.trim()).unwrap();
        let projects = api.projects().await.unwrap();
        let project = projects
            .iter()
            .find(|p| p.name == "照片恢复PR")
            .expect("screenshot project unavailable");
        profile.project_id = project.id.clone();
        profile.name = project.name.clone();
        let api = Api::new(profile, token.trim()).unwrap();
        let metadata = api.metadata("").await.unwrap();
        assert!(!metadata.members.is_empty());
        for id in [
            "priority",
            "seriousLevel",
            "verifier",
            "participants",
            "trackers",
            "labels",
        ] {
            let field = metadata
                .fields
                .iter()
                .find(|f| f.id == id)
                .expect("screenshot field missing");
            if ["verifier", "participants", "trackers", "labels"].contains(&id) {
                assert_eq!(
                    crate::validation::native_target(field, &metadata.profile),
                    Some(id)
                );
            } else {
                assert!(!field.options.is_empty());
            }
        }
        let history = api.history(&user.id, 1).await.unwrap();
        println!("READONLY_RESULT organizations={} projects={} members={} fields={} sprints={} versions={} history={} hasMore={} labelOptionsAvailable={} creates=0 uploads=0",
            orgs.len(), projects.len(), metadata.members.len(), metadata.fields.len(), metadata.sprints.len(), metadata.versions.len(), history.items.len(), history.has_more,
            metadata.fields.iter().find(|f|f.id=="labels").is_some_and(|f|f.option_error.is_none()));
        token.zeroize();
    }
    #[tokio::test]
    async fn startup_requires_real_validation_and_rejects_expired_cached_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/oapi/v1/platform/user"))
            .and(header("x-yunxiao-token", "test-personal-pat"))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&server)
            .await;
        let root = tempfile::tempdir().unwrap();
        let s = state(&server, root.path());
        *s.token.lock().unwrap() = Some("test-personal-pat".into());
        assert!(s.api().is_err());
        let result = check_auth(&s).await.unwrap();
        assert_eq!(result.status, "invalid");
        assert!(s.user.lock().unwrap().is_none());
        assert!(s.api().is_err());
    }
    #[tokio::test]
    async fn valid_login_restores_only_that_users_profile_and_never_writes_pat_to_sqlite() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/oapi/v1/platform/user"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({"id":"user-a","name":"测试员","lastOrganization":"org-a"}),
                ),
            )
            .expect(1)
            .mount(&server)
            .await;
        let root = tempfile::tempdir().unwrap();
        let s = state(&server, root.path());
        let mut other = empty_profile();
        other.name = "另一个用户的项目".into();
        s.store.put("settings", "profile:user-b", &other).unwrap();
        let result = login(&s, "test-personal-pat".into(), false).await.unwrap();
        assert_eq!(result.status, "valid");
        assert_eq!(result.user.unwrap().id, "user-a");
        assert_eq!(s.profile.lock().unwrap().project_id, "unconfigured");
        assert!(s.api().is_ok());
        assert!(!serde_json::to_string(&s.store.list("settings").unwrap())
            .unwrap()
            .contains("test-personal-pat"));
        invalidate(&s);
        assert!(s.api().is_err());
    }
    #[tokio::test]
    async fn denied_user_query_is_unverified_and_does_not_cache_candidate() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/oapi/v1/platform/user"))
            .respond_with(ResponseTemplate::new(403).set_body_string("test-personal-pat secret"))
            .expect(1)
            .mount(&server)
            .await;
        let root = tempfile::tempdir().unwrap();
        let s = state(&server, root.path());
        let result = login(&s, "test-personal-pat".into(), false).await.unwrap();
        assert_eq!(result.status, "unverified");
        assert!(!result.message.contains("test-personal-pat"));
        assert!(s.token.lock().unwrap().is_none());
        assert!(s.api().is_err());
    }
    #[tokio::test]
    async fn missing_token_never_calls_remote_and_malformed_success_is_not_valid() {
        let server = MockServer::start().await;
        let root = tempfile::tempdir().unwrap();
        let s = state(&server, root.path());
        assert_eq!(check_auth(&s).await.unwrap().status, "missing");
        assert!(server.received_requests().await.unwrap().is_empty());
        Mock::given(method("GET"))
            .and(path("/oapi/v1/platform/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name":"no-id"})))
            .mount(&server)
            .await;
        assert_eq!(
            login(&s, "test-personal-pat".into(), false)
                .await
                .unwrap()
                .status,
            "unverified"
        );
        assert!(s.user.lock().unwrap().is_none());
    }
    #[test]
    fn selections_are_scoped_and_remove_obsolete_or_arbitrary_values() {
        let mut p = empty_profile();
        p.organization_id = "org-a".into();
        p.project_id = "project-a".into();
        let a = UserIdentity {
            id: "user-a".into(),
            name: "".into(),
            last_organization: "".into(),
        };
        let b = UserIdentity {
            id: "user-b".into(),
            ..a.clone()
        };
        assert_ne!(scope_key(&a, &p, "bug-1"), scope_key(&b, &p, "bug-1"));
        assert_ne!(scope_key(&a, &p, "bug-1"), scope_key(&a, &p, "bug-2"));
        let mut p2 = p.clone();
        p2.project_id = "project-b".into();
        assert_ne!(scope_key(&a, &p, "bug-1"), scope_key(&a, &p2, "bug-1"));
        let m:Metadata=serde_json::from_value(json!({"profile":p,"typeId":"bug-1","types":[],"members":[{"id":"u1","name":"用户"}],"sprints":[],"versions":[],"fetchedAt":"now","fields":[{"id":"priority","name":"优先级","type":"SystemCustomField","format":"list","required":true,"options":[{"id":"p2","name":"中"}]},{"id":"labels","name":"标签","type":"NativeField","format":"tag","required":false,"options":[{"id":"tag1","name":"标签"}]}]})).unwrap();
        let clean = sanitize_selections(
            &json!({"assignedTo":"deleted","title":"secret","token":"secret","kind":"UI","business":"Android 测试","customFields":{"priority":"p2","labels":["tag1","deleted"],"unknown":"secret","description":"secret"}}),
            &m,
        );
        assert_eq!(clean["assignedTo"], "");
        assert_eq!(clean["kind"], "UI");
        assert_eq!(
            clean["customFields"],
            json!({"priority":"p2","labels":["tag1"]})
        );
        assert!(clean["title"].is_null());
        assert!(clean["token"].is_null());
        let root = tempfile::tempdir().unwrap();
        let store = crate::store::Store::new(root.path()).unwrap();
        store
            .put("selection", &scope_key(&a, &m.profile, "bug-1"), &clean)
            .unwrap();
        assert!(store
            .get::<Value>("selection", &scope_key(&b, &m.profile, "bug-1"))
            .unwrap()
            .is_none());
        let mut refreshed = m.clone();
        refreshed.fields[0].options.clear();
        assert!(sanitize_selections(&clean, &refreshed)["customFields"]["priority"].is_null());
    }
    #[tokio::test]
    async fn remote_history_is_current_user_bug_only_and_exposes_next_page() {
        let server = MockServer::start().await;
        let mut profile = empty_profile();
        profile.organization_id = "org-a".into();
        profile.project_id = "project-a".into();
        let api = Api::with_base(profile, "test-personal-pat", &server.uri()).unwrap();
        Mock::given(method("POST")).and(path("/oapi/v1/projex/organizations/org-a/workitems:search"))
            .and(body_partial_json(json!({"category":"Bug","spaceId":"project-a","page":1,"perPage":50})))
            .respond_with(ResponseTemplate::new(200).insert_header("x-total-pages","2").insert_header("x-next-page","2")
                .set_body_json(json!([{"id":"bug-a","serialNumber":"BUG-1","subject":"历史问题","categoryId":"Bug","creator":{"id":"user-a"},"space":{"id":"project-a"},"status":{"displayName":"待处理"}}])))
            .expect(1).mount(&server).await;
        Mock::given(method("POST"))
            .and(path("/oapi/v1/projex/organizations/org-a/workitems:search"))
            .and(body_partial_json(json!({"page":2})))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-total-pages", "2")
                    .set_body_json(json!([])),
            )
            .expect(1)
            .mount(&server)
            .await;
        let first = api.history("user-a", 1).await.unwrap();
        assert!(first.has_more);
        assert_eq!(first.items[0].identifier, "BUG-1");
        assert!(!api.history("user-a", 2).await.unwrap().has_more);
        for request in server.received_requests().await.unwrap() {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let conditions: Value =
                serde_json::from_str(body["conditions"].as_str().unwrap()).unwrap();
            assert_eq!(conditions["conditionGroups"][0][0]["value"][0], "user-a");
        }
    }
    #[tokio::test]
    async fn history_permission_failure_and_scope_mismatch_are_errors_not_empty_success() {
        let server = MockServer::start().await;
        let mut profile = empty_profile();
        profile.organization_id = "org-a".into();
        profile.project_id = "project-a".into();
        let api = Api::with_base(profile, "test-personal-pat", &server.uri()).unwrap();
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"page":1})))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        Mock::given(method("POST")).and(body_partial_json(json!({"page":2})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"id":"bug-a","space":{"id":"other-project"},"creator":{"id":"user-a"},"categoryId":"Bug"}])))
            .mount(&server).await;
        assert!(api
            .history("user-a", 1)
            .await
            .unwrap_err()
            .contains("权限不足"));
        assert!(api
            .history("user-a", 2)
            .await
            .unwrap_err()
            .contains("范围不一致"));
    }
}

#[cfg(test)]
mod field_tests {
    use super::*;
    use wiremock::{
        matchers::{method, path, query_param},
        Mock, MockServer, ResponseTemplate,
    };
    #[tokio::test]
    async fn metadata_reads_all_member_and_label_pages_for_web_fields() {
        let server = MockServer::start().await;
        let mut p = empty_profile();
        p.organization_id = "org-a".into();
        p.project_id = "project-a".into();
        p.enable_versions = false;
        p.enable_sprints = false;
        let api = Api::with_base(p, "mock-token", &server.uri()).unwrap();
        let prefix = "/oapi/v1/projex/organizations/org-a/projects/project-a";
        Mock::given(method("GET"))
            .and(path(format!("{prefix}/workitemTypes")))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!([{"id":"bug-type","name":"缺陷"}])),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET")).and(path(format!("{prefix}/workitemTypes/bug-type/fields")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"id":"labels","name":"标签","type":"NativeField","format":"tag","required":false,"showWhenCreate":true},
                {"id":"priority","name":"优先级","type":"SystemCustomField","format":"list","required":true,"options":[{"id":"p2","displayValue":"中"}]}
            ]))).mount(&server).await;
        for page in [1, 2] {
            for (endpoint, id_key, name_key) in
                [("members", "userId", "userName"), ("labels", "id", "name")]
            {
                Mock::given(method("GET"))
                    .and(path(format!("{prefix}/{endpoint}")))
                    .and(query_param("page", page.to_string()))
                    .respond_with(
                        ResponseTemplate::new(200)
                            .insert_header("x-total-pages", "2")
                            .insert_header("x-next-page", if page == 1 { "2" } else { "3" })
                            .set_body_json(
                                json!([{id_key:format!("{endpoint}-{page}"),name_key:"同名选项"}]),
                            ),
                    )
                    .expect(1)
                    .mount(&server)
                    .await;
            }
        }
        let m = api.metadata("").await.unwrap();
        assert_eq!(m.members.len(), 2);
        assert_eq!(m.fields[0].options.len(), 2);
        assert_eq!(m.fields[1].options[0].name, "中");
        assert!(m.fields[0].show_when_create);
    }
    #[test]
    fn native_tag_selection_is_validated_before_payload_and_preserves_remote_ids() {
        let mut m: Metadata =
            serde_json::from_str(include_str!("../../tests/fixtures/metadata.json")).unwrap();
        m.fields.push(serde_json::from_value(json!({"id":"labels","name":"标签","type":"Application","format":"tag","required":false,"options":[{"id":"label-1","name":"功能"}]})).unwrap());
        m.fields.push(serde_json::from_value(json!({"id":"verifier","name":"验证者","type":"Role","format":"user","required":false,"options":[]})).unwrap());
        let mut d: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/valid-draft.json")).unwrap();
        d["customFields"]["labels"] = json!(["label-1"]);
        d["customFields"]["verifier"] = json!(m.members[0].id);
        assert!(crate::validation::validate(&d, &m.profile, Some(&m), true, true).is_empty());
        let payload = crate::yunxiao::create_payload(&d, &m, "description".into()).unwrap();
        assert_eq!(payload["labels"], json!(["label-1"]));
        assert_eq!(payload["verifier"], json!(m.members[0].id));
        d["customFields"]["labels"] = json!(["deleted-label"]);
        assert!(
            crate::validation::validate(&d, &m.profile, Some(&m), true, true)
                .iter()
                .any(|e| e.field == "customFields.labels")
        );
    }
    #[tokio::test]
    async fn empty_optional_labels_do_not_block_fields_and_empty_final_pages_stop() {
        let server = MockServer::start().await;
        let mut p = empty_profile();
        p.organization_id = "org-a".into();
        p.project_id = "project-a".into();
        let api = Api::with_base(p, "mock-token", &server.uri()).unwrap();
        let prefix = "/oapi/v1/projex/organizations/org-a/projects/project-a";
        Mock::given(method("GET"))
            .and(path(format!("{prefix}/workitemTypes")))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!([{"id":"bug-type","name":"缺陷"}])),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET")).and(path(format!("{prefix}/workitemTypes/bug-type/fields")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"id":"labels","name":"标签","type":"Application","format":"tag","required":false},
                {"id":"priority","name":"优先级","type":"SystemCustomField","format":"list","required":true,"options":[{"id":"p1","displayValue":"高"}]}
            ]))).mount(&server).await;
        Mock::given(method("GET"))
            .and(path(format!("{prefix}/labels")))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        for endpoint in ["members", "versions", "sprints"] {
            Mock::given(method("GET"))
                .and(path(format!("{prefix}/{endpoint}")))
                .and(wiremock::matchers::query_param("page", "1"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("x-total-pages", "0")
                        .insert_header("x-next-page", "2")
                        .set_body_json(json!([])),
                )
                .expect(1)
                .mount(&server)
                .await;
        }
        let metadata = api.metadata("").await.unwrap();
        assert!(metadata.fields[0].options.is_empty());
        assert!(metadata.fields[0].option_error.is_some());
        assert_eq!(metadata.fields[1].options[0].id, "p1");
        assert!(metadata.versions.is_empty());
    }
}
