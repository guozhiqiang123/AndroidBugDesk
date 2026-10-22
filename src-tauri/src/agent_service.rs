use crate::{
    commands::{self, AppState},
    model::{attachments, string, Attachment, Result},
    post_capture, project_settings, workspace,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::HashMap, path::PathBuf};
use tauri::{AppHandle, Emitter};

pub const PROTOCOL_VERSION: u8 = 1;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DraftIdParams {
    draft_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RevisionParams {
    draft_id: String,
    expected_revision: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EventPatch {
    occurred_at: Option<String>,
    source_note: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FrequencyPatch {
    first_discovery: Option<bool>,
    attempts: Option<String>,
    occurrences: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DraftPatch {
    jenkins_package_url: Option<String>,
    title: Option<String>,
    kind: Option<String>,
    bug_page: Option<String>,
    preconditions: Option<String>,
    steps: Option<String>,
    expected: Option<String>,
    actual: Option<String>,
    event: Option<EventPatch>,
    frequency: Option<FrequencyPatch>,
    workitem_type_id: Option<String>,
    assigned_to: Option<String>,
    sprint: Option<String>,
    versions: Option<Vec<String>>,
    custom_fields: Option<HashMap<String, Value>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PatchDraftParams {
    draft_id: String,
    expected_revision: String,
    patch: DraftPatch,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CaptureParams {
    draft_id: String,
    expected_revision: String,
    serial: String,
    #[serde(default)]
    items: Vec<String>,
    retry_of: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ImportAttachmentParams {
    draft_id: String,
    expected_revision: String,
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemoveAttachmentParams {
    draft_id: String,
    expected_revision: String,
    attachment_id: String,
}

pub async fn dispatch<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    method: &str,
    params: Value,
) -> Result<Value> {
    match method {
        "workspace_status" => workspace_status(state),
        "list_devices" => {
            workspace::current_scope(state, "")?;
            project_settings::current_package_for_state(state)?;
            Ok(crate::adb::devices(&state.adb_path()?).await?)
        }
        "create_draft" => {
            reject_nonempty_params(&params)?;
            let draft = commands::new_draft_for_state(state)?;
            notify_draft(app, &draft, "created");
            draft_result(draft)
        }
        "get_draft" => {
            let request: DraftIdParams = parse(params)?;
            let draft = scoped_draft(state, &request.draft_id)?;
            draft_result(draft)
        }
        "patch_draft" => {
            let request: PatchDraftParams = parse(params)?;
            let draft = patch_draft(state, request)?;
            notify_draft(app, &draft, "patched");
            draft_result(draft)
        }
        "validate_draft" => {
            let request: DraftIdParams = parse(params)?;
            let draft = scoped_draft(state, &request.draft_id)?;
            let issues = commands::validate_draft_for_state(&request.draft_id, state)?;
            Ok(json!({
                "draftId": request.draft_id,
                "revision": string(&draft, "updatedAt"),
                "ready": issues.is_empty(),
                "issues": issues,
            }))
        }
        "capture_environment" => {
            let mut request: CaptureParams = parse(params)?;
            if !request.items.is_empty() && request.items != ["environment"] {
                return Err("设备环境快照只能采集 environment 项".into());
            }
            request.items = vec!["environment".into()];
            start_capture(app, state, request).await
        }
        "capture_evidence" => {
            let request: CaptureParams = parse(params)?;
            start_capture(app, state, request).await
        }
        "capture_status" => {
            let request: DraftIdParams = parse(params)?;
            let mut snapshot = post_capture::status_for_state(state, &request.draft_id)?;
            snapshot["draft"] = sanitize_draft(snapshot["draft"].clone());
            Ok(snapshot)
        }
        "import_attachment" => {
            let request: ImportAttachmentParams = parse(params)?;
            assert_revision(state, &request.draft_id, &request.expected_revision)?;
            let path = validate_import_path(&request.path)?;
            let draft = commands::import_attachment_paths_if_revision(
                state,
                &request.draft_id,
                &[path],
                &request.expected_revision,
            )?;
            notify_draft(app, &draft, "attachment-imported");
            draft_result(draft)
        }
        "list_attachments" => {
            let request: DraftIdParams = parse(params)?;
            let draft = scoped_draft(state, &request.draft_id)?;
            Ok(json!({
                "draftId": request.draft_id,
                "revision": string(&draft, "updatedAt"),
                "attachments": attachment_metadata(&draft)?,
            }))
        }
        "remove_attachment" => {
            let request: RemoveAttachmentParams = parse(params)?;
            let draft = commands::remove_attachment_if_revision(
                &request.draft_id,
                &request.attachment_id,
                &request.expected_revision,
                state,
            )?;
            notify_draft(app, &draft, "attachment-removed");
            draft_result(draft)
        }
        "prepare_submission" => {
            let request: RevisionParams = parse(params)?;
            let draft = scoped_draft(state, &request.draft_id)?;
            assert_revision_value(&draft, &request.expected_revision)?;
            let issues = commands::validate_draft_for_state(&request.draft_id, state)?;
            let profile = state.profile.lock().unwrap().clone();
            Ok(json!({
                "draftId": request.draft_id,
                "revision": string(&draft, "updatedAt"),
                "ready": issues.is_empty(),
                "issues": issues,
                "preview": {
                    "project": {"id": profile.project_id, "name": profile.name},
                    "title": draft["title"],
                    "kind": draft["kind"],
                    "jenkinsPackageUrl": draft["jenkinsPackageUrl"],
                    "bugPage": draft["bugPage"],
                    "preconditions": draft["preconditions"],
                    "steps": draft["steps"],
                    "expected": draft["expected"],
                    "actual": draft["actual"],
                    "app": draft["app"],
                    "device": draft["device"],
                    "event": draft["event"],
                    "attachmentCount": draft["attachments"].as_array().map_or(0, Vec::len),
                    "attachments": attachment_metadata(&draft)?,
                },
                "nextAction": "请在 Android Bug Desk 的提交预览弹窗中核对并点击“提交 Bug”",
            }))
        }
        _ => Err("不支持的 AI 工具方法".into()),
    }
}

fn workspace_status(state: &AppState) -> Result<Value> {
    let user = state.user.lock().unwrap().clone();
    let profile = state.profile.lock().unwrap().clone();
    let authorized = user.is_some();
    let current_scope = workspace::current_scope(state, "").ok();
    let package_name = project_settings::current_package_for_state(state).ok();
    let drafts = if let Some(scope) = current_scope.as_deref() {
        state
            .store
            .list("draft")?
            .into_iter()
            .filter(|draft| {
                let id = string(draft, "id");
                state
                    .store
                    .get::<String>("draft_scope", &id)
                    .ok()
                    .flatten()
                    .as_deref()
                    == Some(scope)
            })
            .map(|draft| {
                json!({
                    "id": draft["id"],
                    "title": draft["title"],
                    "kind": draft["kind"],
                    "revision": draft["updatedAt"],
                    "attachmentCount": draft["attachments"].as_array().map_or(0, Vec::len),
                })
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    Ok(json!({
        "protocolVersion": PROTOCOL_VERSION,
        "authorized": authorized,
        "user": user.map(|identity| json!({"name":identity.name,"nickName":identity.nick_name})),
        "project": {
            "configured": authorized && profile.project_id != "unconfigured",
            "organizationId": profile.organization_id,
            "projectId": profile.project_id,
            "name": profile.name,
            "packageName": package_name,
        },
        "adb": {"configured": state.adb.lock().unwrap().is_some()},
        "drafts": drafts,
        "submissionPolicy": "prepare-only",
    }))
}

fn patch_draft(state: &AppState, request: PatchDraftParams) -> Result<Value> {
    let mut draft = scoped_draft(state, &request.draft_id)?;
    assert_revision_value(&draft, &request.expected_revision)?;
    let patch = request.patch;
    assign_string(&mut draft, "jenkinsPackageUrl", patch.jenkins_package_url);
    assign_string(&mut draft, "title", patch.title);
    assign_string(&mut draft, "kind", patch.kind);
    if let Some(value) = patch.bug_page {
        draft["bugPage"] = json!(value);
        draft["sources"]["bugPage"] =
            json!({"status":"manual","at":crate::model::now(),"detail":"AI 辅助填写，待用户核对"});
    }
    assign_string(&mut draft, "preconditions", patch.preconditions);
    assign_string(&mut draft, "steps", patch.steps);
    assign_string(&mut draft, "expected", patch.expected);
    assign_string(&mut draft, "actual", patch.actual);
    if let Some(event) = patch.event {
        assign_string(&mut draft["event"], "occurredAt", event.occurred_at);
        assign_string(&mut draft["event"], "sourceNote", event.source_note);
        draft["event"]["source"] = json!("manual");
    }
    if let Some(frequency) = patch.frequency {
        if let Some(value) = frequency.first_discovery {
            draft["frequency"]["firstDiscovery"] = json!(value);
        }
        assign_string(&mut draft["frequency"], "attempts", frequency.attempts);
        assign_string(
            &mut draft["frequency"],
            "occurrences",
            frequency.occurrences,
        );
    }
    assign_string(&mut draft, "workitemTypeId", patch.workitem_type_id);
    assign_string(&mut draft, "assignedTo", patch.assigned_to);
    assign_string(&mut draft, "sprint", patch.sprint);
    if let Some(value) = patch.versions {
        draft["versions"] = json!(value);
    }
    if let Some(value) = patch.custom_fields {
        draft["customFields"] = json!(value);
    }
    commands::save_draft_if_revision(draft, &request.expected_revision, state)
}

async fn start_capture<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    request: CaptureParams,
) -> Result<Value> {
    assert_revision(state, &request.draft_id, &request.expected_revision)?;
    let job = post_capture::start_for_state(
        app.clone(),
        state,
        request.draft_id,
        request.serial,
        request.items,
        request.retry_of,
        true,
    )
    .await?;
    Ok(json!({
        "jobId": job["id"],
        "status": job["status"],
        "items": job["items"],
        "startedAt": job["startedAt"],
    }))
}

fn scoped_draft(state: &AppState, id: &str) -> Result<Value> {
    uuid::Uuid::parse_str(id).map_err(|_| "草稿 ID 格式不正确".to_string())?;
    let current = workspace::current_scope(state, "")?;
    if state.store.get::<String>("draft_scope", id)?.as_deref() != Some(current.as_str()) {
        return Err("草稿不属于当前账号、组织或项目".into());
    }
    state.store.draft(id)
}

fn assert_revision(state: &AppState, id: &str, expected: &str) -> Result<()> {
    let draft = scoped_draft(state, id)?;
    assert_revision_value(&draft, expected)
}

fn assert_revision_value(draft: &Value, expected: &str) -> Result<()> {
    let latest = string(draft, "updatedAt");
    if latest != expected {
        return Err(format!(
            "草稿已被其他操作更新，请重新读取后再修改；latestRevision={latest}"
        ));
    }
    Ok(())
}

fn validate_import_path(raw: &str) -> Result<PathBuf> {
    if raw.trim().is_empty() || raw.contains(['\r', '\n']) {
        return Err("附件路径不正确".into());
    }
    let path = PathBuf::from(raw);
    let metadata = std::fs::symlink_metadata(&path).map_err(|_| "附件文件不存在")?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("只能导入普通文件，不能导入目录或符号链接".into());
    }
    path.canonicalize().map_err(|_| "附件路径无法解析".into())
}

fn attachment_metadata(draft: &Value) -> Result<Vec<Value>> {
    attachments(draft).map(|rows| rows.iter().map(safe_attachment).collect())
}

fn safe_attachment(attachment: &Attachment) -> Value {
    json!({
        "id": attachment.id,
        "name": attachment.name,
        "mime": attachment.mime,
        "size": attachment.size,
        "sha256": attachment.sha256,
        "role": attachment.role,
        "source": attachment.source,
        "coverageStart": attachment.coverage_start,
        "coverageEnd": attachment.coverage_end,
        "derivedFrom": attachment.derived_from,
    })
}

fn sanitize_draft(mut draft: Value) -> Value {
    if let Some(rows) = draft["attachments"].as_array_mut() {
        for row in rows {
            if let Some(object) = row.as_object_mut() {
                object.remove("path");
            }
        }
    }
    if let Some(rows) = draft["markdownAssets"].as_array_mut() {
        for row in rows {
            if let Some(object) = row.as_object_mut() {
                object.remove("path");
                object.remove("embedUrl");
            }
        }
    }
    draft
}

fn draft_result(draft: Value) -> Result<Value> {
    Ok(json!({
        "draftId": draft["id"],
        "revision": draft["updatedAt"],
        "draft": sanitize_draft(draft),
    }))
}

fn assign_string(target: &mut Value, key: &str, value: Option<String>) {
    if let Some(value) = value {
        target[key] = json!(value);
    }
}

fn parse<T: for<'de> Deserialize<'de>>(params: Value) -> Result<T> {
    serde_json::from_value(params).map_err(|error| format!("AI 工具参数不正确：{error}"))
}

fn reject_nonempty_params(params: &Value) -> Result<()> {
    if params.is_null() || params.as_object().is_some_and(serde_json::Map::is_empty) {
        Ok(())
    } else {
        Err("此工具不接受参数".into())
    }
}

fn notify_draft<R: tauri::Runtime>(app: &AppHandle<R>, draft: &Value, reason: &str) {
    let _ = app.emit(
        "agent-draft-updated",
        json!({"draftId":string(draft, "id"),"revision":string(draft, "updatedAt"),"reason":reason}),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::Profile,
        project_settings::{ProjectSetting, ProjectSettings},
        workspace::UserIdentity,
    };

    fn state() -> (tempfile::TempDir, AppState) {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::new(root.path().to_path_buf()).unwrap();
        *state.user.lock().unwrap() = Some(UserIdentity {
            id: "agent-test-user".into(),
            name: "Agent Test".into(),
            nick_name: String::new(),
            avatar_png: None,
            last_organization: "test-org".into(),
        });
        *state.profile.lock().unwrap() = Profile {
            name: "Test Project".into(),
            edition: "central".into(),
            organization_id: "test-org".into(),
            project_id: "test-project".into(),
            required_field_ids: Vec::new(),
            field_mappings: HashMap::new(),
            enable_versions: true,
            enable_sprints: true,
            attachment_warning_mb: 50.0,
        };
        project_settings::save_for_state(
            &state,
            "test-org",
            ProjectSettings {
                entries: vec![ProjectSetting {
                    project_id: "test-project".into(),
                    enabled: true,
                    package_name: "com.example.agent".into(),
                }],
            },
        )
        .unwrap();
        (root, state)
    }

    #[test]
    fn patch_is_strict_and_revision_conflicts_do_not_overwrite() {
        let (_root, state) = state();
        let draft = commands::new_draft_for_state(&state).unwrap();
        let revision = string(&draft, "updatedAt");
        let patched = patch_draft(
            &state,
            PatchDraftParams {
                draft_id: string(&draft, "id"),
                expected_revision: revision.clone(),
                patch: DraftPatch {
                    title: Some("真实标题".into()),
                    actual: Some("真实问题表现".into()),
                    ..Default::default()
                },
            },
        )
        .unwrap();
        assert_eq!(patched["title"], "真实标题");
        assert_eq!(patched["actual"], "真实问题表现");
        assert_ne!(string(&patched, "updatedAt"), revision);

        let error = patch_draft(
            &state,
            PatchDraftParams {
                draft_id: string(&draft, "id"),
                expected_revision: revision,
                patch: DraftPatch {
                    title: Some("不应覆盖".into()),
                    ..Default::default()
                },
            },
        )
        .unwrap_err();
        assert!(error.contains("latestRevision="));
        assert_eq!(
            state.store.draft(&string(&draft, "id")).unwrap()["title"],
            "真实标题"
        );
        assert!(parse::<PatchDraftParams>(json!({
            "draftId": string(&draft, "id"),
            "expectedRevision": string(&patched, "updatedAt"),
            "patch": {"title":"x","attachments":[]}
        }))
        .is_err());
    }

    #[test]
    fn sanitized_draft_never_returns_managed_paths() {
        let value = sanitize_draft(json!({
            "attachments":[{"id":"a","path":"/private/attachment","name":"a.png"}],
            "markdownAssets":[{"id":"m","path":"/private/markdown","embedUrl":"file:///private"}]
        }));
        assert!(value["attachments"][0].get("path").is_none());
        assert!(value["markdownAssets"][0].get("path").is_none());
        assert!(value["markdownAssets"][0].get("embedUrl").is_none());
    }
}
