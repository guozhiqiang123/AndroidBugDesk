use crate::{commands::AppState, model::*, workspace};
use serde_json::Value;
use std::{collections::HashSet, fs, path::PathBuf};
use tauri::State;

#[tauri::command]
pub fn delete_draft(id: String, state: State<'_, AppState>) -> Result<()> {
    delete_for_state(&state, &id)
}

#[tauri::command]
pub fn delete_drafts(ids: Vec<String>, state: State<'_, AppState>) -> Result<()> {
    delete_many_for_state(&state, &ids)
}

pub(crate) fn delete_for_state(state: &AppState, id: &str) -> Result<()> {
    delete_many_for_state(state, &[id.to_owned()])
}

pub(crate) fn delete_many_for_state(state: &AppState, ids: &[String]) -> Result<()> {
    if ids.is_empty() {
        return Err("请选择要删除的草稿".into());
    }
    for id in ids {
        uuid::Uuid::parse_str(id).map_err(|_| "草稿 ID 格式不正确".to_string())?;
    }
    let _write = state.store.writes.lock().unwrap();
    // Keep submission admission blocked until the atomic deletion commits.
    let locks = state.locks.lock().unwrap();
    let user = state.user.lock().unwrap();
    let user = user.as_ref().ok_or("请先验证登录令牌")?;
    let profile = state.profile.lock().unwrap();
    let scope = workspace::scope_key(user, &profile, "");
    // Validate the whole confirmed set before removing any record.
    for id in ids {
        if locks.contains(id) {
            return Err("草稿正在提交，不能删除".into());
        }
        if state.store.get::<String>("draft_scope", id)?.as_deref() != Some(scope.as_str()) {
            return Err("只能删除当前账号、组织和项目的草稿".into());
        }
        let draft = state.store.draft(id)?;
        if state.store.get::<Value>("submission", id)?.is_some() {
            return Err("提交证据已固定，不能删除；请在提交记录中核对或恢复".into());
        }
        if crate::capture::pending(&state.store, id)? {
            return Err("证据正在采集，不能删除草稿".into());
        }
        let session_id = string(&draft, "sessionId");
        if !session_id.is_empty() {
            let session = state
                .store
                .get::<Value>("session", &session_id)?
                .ok_or("草稿绑定的设备会话不存在，无法确认采集状态")?;
            if string(&session, "stoppedAt").is_empty() {
                return Err("请先停止设备采集，再删除草稿".into());
            }
        }
    }
    let capture_records = state.store.list("capture")?;
    let target_ids: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let target_captures: Vec<&Value> = capture_records
        .iter()
        .filter(|task| target_ids.contains(string(task, "draftId").as_str()))
        .collect();
    let captures: Vec<String> = target_captures
        .iter()
        .map(|task| string(task, "id"))
        .collect();
    let generated_attachment_directories = generated_attachment_directories(
        &state.store.root,
        &target_ids,
        &target_captures,
        &capture_records,
        &state.store.list("draft")?,
    )?;
    let markdown_asset_directories = ids
        .iter()
        .map(|id| crate::markdown_assets::draft_directory(&state.store.root, id))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let mut objects: Vec<(&str, &str)> = ids
        .iter()
        .flat_map(|id| {
            [
                ("draft", id.as_str()),
                ("draft_scope", id.as_str()),
                ("snapshot", id.as_str()),
            ]
        })
        .collect();
    objects.extend(captures.iter().map(|capture| ("capture", capture.as_str())));
    // Exported evidence, imported attachments, and session files are intentionally retained.
    // Only post-capture attachments recorded by the two collection actions are removed.
    state.store.remove_many(&objects)?;
    remove_generated_attachment_directories(&generated_attachment_directories)?;
    crate::markdown_assets::remove_directories(&markdown_asset_directories);
    Ok(())
}

fn capture_attachment_ids(task: &Value) -> HashSet<String> {
    let kind = string(task, "kind");
    if kind == "software-diagnostics" {
        return task["attachments"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
    }
    if kind != "post-capture" {
        return HashSet::new();
    }
    let mut ids = HashSet::new();
    if let Some(items) = task["items"].as_object() {
        for item in items.values() {
            for id in item["attachments"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                ids.insert(id.to_owned());
            }
        }
    }
    if let Some(id) = task["summaryAttachmentId"]
        .as_str()
        .filter(|id| !id.is_empty())
    {
        ids.insert(id.to_owned());
    }
    ids
}

fn draft_attachment_ids(draft: &Value) -> HashSet<String> {
    draft["attachments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|attachment| attachment["id"].as_str())
        .map(str::to_owned)
        .collect()
}

fn generated_attachment_directories(
    root: &std::path::Path,
    target_draft_ids: &HashSet<&str>,
    target_captures: &[&Value],
    all_captures: &[Value],
    all_drafts: &[Value],
) -> Result<Vec<PathBuf>> {
    let mut generated: HashSet<String> = target_captures
        .iter()
        .flat_map(|task| capture_attachment_ids(task))
        .collect();
    if generated.is_empty() {
        return Ok(Vec::new());
    }

    // A duplicated or inherited attachment can be referenced from another draft. Keep any such
    // file, including an older capture result that is no longer in that draft's current list.
    let mut retained = HashSet::new();
    for draft in all_drafts {
        if !target_draft_ids.contains(string(draft, "id").as_str()) {
            retained.extend(draft_attachment_ids(draft));
        }
    }
    for task in all_captures {
        if !target_draft_ids.contains(string(task, "draftId").as_str()) {
            retained.extend(capture_attachment_ids(task));
        }
    }
    generated.retain(|id| !retained.contains(id));

    let root = root
        .canonicalize()
        .map_err(|_| "本地证据根目录不存在".to_string())?;
    let attachments = root.join("attachments");
    if !attachments.exists() {
        return Ok(Vec::new());
    }
    let attachments = attachments
        .canonicalize()
        .map_err(|_| "本地附件目录无法读取".to_string())?;
    if !attachments.starts_with(&root) {
        return Err("本地附件目录不能离开证据根目录".into());
    }

    let mut directories = Vec::new();
    for id in generated {
        let Ok(parsed) = uuid::Uuid::parse_str(&id) else {
            continue;
        };
        // IDs produced by the app are normalized UUIDs. Requiring the exact representation keeps
        // filesystem lookup independent from untrusted task data.
        if parsed.to_string() != id {
            continue;
        }
        let directory = attachments.join(&id);
        let Ok(metadata) = fs::symlink_metadata(&directory) else {
            continue;
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("采集附件目录格式异常，已停止删除".into());
        }
        let canonical = directory
            .canonicalize()
            .map_err(|_| "采集附件目录无法读取".to_string())?;
        if canonical.parent() != Some(attachments.as_path()) {
            return Err("采集附件目录不能离开本地附件目录".into());
        }
        directories.push(canonical);
    }
    Ok(directories)
}

fn remove_generated_attachment_directories(directories: &[PathBuf]) -> Result<()> {
    let mut failures = Vec::new();
    for directory in directories {
        if let Err(error) = fs::remove_dir_all(directory) {
            failures.push(format!("{}: {error}", directory.display()));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "草稿已删除，但部分采集附件清理失败：{}",
            failures.join("；")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn state() -> (tempfile::TempDir, AppState, String) {
        let dir = tempdir().unwrap();
        let state = AppState::new(dir.path().into()).unwrap();
        *state.user.lock().unwrap() = Some(workspace::UserIdentity {
            id: "alice".into(),
            ..Default::default()
        });
        let mut profile = workspace::empty_profile();
        profile.organization_id = "org".into();
        profile.project_id = "project".into();
        *state.profile.lock().unwrap() = profile;
        let id = uuid();
        state
            .store
            .put("draft", &id, &json!({"id":id,"sessionId":null}))
            .unwrap();
        state
            .store
            .put(
                "draft_scope",
                &id,
                &workspace::current_scope(&state, "").unwrap(),
            )
            .unwrap();
        (dir, state, id)
    }

    fn stored_attachment(root: &std::path::Path, id: &str, name: &str) -> PathBuf {
        let directory = root.join("attachments").join(id);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(name), name.as_bytes()).unwrap();
        directory
    }

    #[test]
    fn deletion_is_persistent_and_only_removes_related_local_records() {
        let (dir, state, id) = state();
        let markdown_directory = dir.path().join("markdown-assets").join(&id).join(uuid());
        std::fs::create_dir_all(&markdown_directory).unwrap();
        std::fs::write(markdown_directory.join("image.png"), b"image").unwrap();
        state.store.put("snapshot", &id, &json!({})).unwrap();
        state
            .store
            .put(
                "capture",
                "own",
                &json!({"id":"own","draftId":id,"status":"complete"}),
            )
            .unwrap();
        state
            .store
            .put(
                "capture",
                "other",
                &json!({"id":"other","draftId":"other","status":"complete"}),
            )
            .unwrap();
        let exported = dir.path().join("export.zip");
        std::fs::write(&exported, b"immutable evidence").unwrap();
        delete_for_state(&state, &id).unwrap();
        let reopened = crate::store::Store::new(dir.path()).unwrap();
        for kind in ["draft", "draft_scope", "snapshot"] {
            assert!(reopened.get::<Value>(kind, &id).unwrap().is_none());
        }
        assert!(reopened.get::<Value>("capture", "own").unwrap().is_none());
        assert!(reopened.get::<Value>("capture", "other").unwrap().is_some());
        assert_eq!(std::fs::read(exported).unwrap(), b"immutable evidence");
        assert!(!dir.path().join("markdown-assets").join(&id).exists());
    }

    #[test]
    fn deletion_removes_only_generated_capture_attachments() {
        let (dir, state, id) = state();
        let imported_id = uuid();
        let environment_id = uuid();
        let supplemental_id = uuid();
        let old_supplemental_id = uuid();
        let summary_id = uuid();
        let software_id = uuid();
        let imported = stored_attachment(dir.path(), &imported_id, "imported.png");
        let environment = stored_attachment(dir.path(), &environment_id, "environment.txt");
        let supplemental = stored_attachment(dir.path(), &supplemental_id, "logs.zip");
        let old_supplemental =
            stored_attachment(dir.path(), &old_supplemental_id, "older-logs.zip");
        let summary = stored_attachment(dir.path(), &summary_id, "summary.zip");
        let software = stored_attachment(dir.path(), &software_id, "software.zip");
        let exported = dir
            .path()
            .join("evidence")
            .join(uuid())
            .join("evidence.zip");
        std::fs::create_dir_all(exported.parent().unwrap()).unwrap();
        std::fs::write(&exported, b"exported evidence").unwrap();
        state
            .store
            .put(
                "draft",
                &id,
                &json!({
                    "id": id,
                    "sessionId": null,
                    "attachments": [
                        {"id": imported_id},
                        {"id": environment_id},
                        {"id": supplemental_id},
                        {"id": summary_id},
                        {"id": software_id}
                    ]
                }),
            )
            .unwrap();
        state
            .store
            .put(
                "capture",
                "software-job",
                &json!({
                    "id": "software-job",
                    "draftId": id,
                    "kind": "software-diagnostics",
                    "status": "complete",
                    "attachments": [software_id]
                }),
            )
            .unwrap();
        state
            .store
            .put(
                "capture",
                "environment-job",
                &json!({
                    "id": "environment-job",
                    "draftId": id,
                    "kind": "post-capture",
                    "status": "complete",
                    "items": {"environment": {"attachments": [environment_id]}}
                }),
            )
            .unwrap();
        state
            .store
            .put(
                "capture",
                "supplemental-job",
                &json!({
                    "id": "supplemental-job",
                    "draftId": id,
                    "kind": "post-capture",
                    "status": "complete",
                    "items": {
                        "logs": {"attachments": [supplemental_id]},
                        "screenshot": {"attachments": [old_supplemental_id]}
                    },
                    "summaryAttachmentId": summary_id
                }),
            )
            .unwrap();

        delete_for_state(&state, &id).unwrap();

        assert!(
            imported.exists(),
            "user-imported attachment must be retained"
        );
        assert!(exported.exists(), "exported evidence must be retained");
        assert!(!environment.exists());
        assert!(!supplemental.exists());
        assert!(!old_supplemental.exists());
        assert!(!summary.exists());
        assert!(!software.exists());
    }

    #[test]
    fn deletion_keeps_generated_attachment_referenced_by_a_surviving_draft() {
        let (dir, state, id) = state();
        let sibling = sibling(&state);
        let shared_id = uuid();
        let shared = stored_attachment(dir.path(), &shared_id, "shared.zip");
        state
            .store
            .put(
                "draft",
                &sibling,
                &json!({
                    "id": sibling,
                    "sessionId": null,
                    "attachments": [{"id": shared_id}]
                }),
            )
            .unwrap();
        state
            .store
            .put(
                "capture",
                "target-job",
                &json!({
                    "id": "target-job",
                    "draftId": id,
                    "kind": "post-capture",
                    "status": "complete",
                    "items": {"logs": {"attachments": [shared_id]}}
                }),
            )
            .unwrap();

        delete_for_state(&state, &id).unwrap();

        assert!(shared.exists());
        assert!(state.store.draft(&sibling).is_ok());
    }

    #[test]
    fn deletion_ignores_untrusted_or_unrelated_capture_attachment_ids() {
        let (dir, state, id) = state();
        let unrelated_id = uuid();
        let unrelated = stored_attachment(dir.path(), &unrelated_id, "unrelated.zip");
        let outside = dir.path().join("outside.txt");
        std::fs::write(&outside, b"outside").unwrap();
        state
            .store
            .put(
                "capture",
                "legacy-job",
                &json!({
                    "id": "legacy-job",
                    "draftId": id,
                    "kind": "legacy-capture",
                    "status": "complete",
                    "items": {"environment": {"attachments": [unrelated_id, "../../outside.txt"]}}
                }),
            )
            .unwrap();

        delete_for_state(&state, &id).unwrap();

        assert!(unrelated.exists());
        assert_eq!(std::fs::read(outside).unwrap(), b"outside");
    }

    #[test]
    fn other_account_organization_project_and_unbound_drafts_cannot_be_deleted() {
        let (_dir, state, id) = state();
        *state.user.lock().unwrap() = None;
        assert!(delete_for_state(&state, &id).is_err());
        *state.user.lock().unwrap() = Some(workspace::UserIdentity {
            id: "alice".into(),
            ..Default::default()
        });
        state.user.lock().unwrap().as_mut().unwrap().id = "bob".into();
        assert!(delete_for_state(&state, &id).is_err());
        state.user.lock().unwrap().as_mut().unwrap().id = "alice".into();
        state.profile.lock().unwrap().organization_id = "other".into();
        assert!(delete_for_state(&state, &id).is_err());
        state.profile.lock().unwrap().organization_id = "org".into();
        state.profile.lock().unwrap().project_id = "other".into();
        assert!(delete_for_state(&state, &id).is_err());
        state.profile.lock().unwrap().project_id = "project".into();
        state.store.remove_many(&[("draft_scope", &id)]).unwrap();
        assert!(delete_for_state(&state, &id).is_err());
        assert!(state.store.draft(&id).is_ok());
    }

    #[test]
    fn frozen_unknown_or_in_flight_submission_cannot_be_deleted() {
        for status in ["submitted", "create_unknown", "partial", "creating"] {
            let (_dir, state, id) = state();
            state
                .store
                .put("submission", &id, &json!({"state":status}))
                .unwrap();
            assert!(delete_for_state(&state, &id).is_err());
            assert!(state.store.draft(&id).is_ok());
        }
        let (_dir, state, id) = state();
        state.locks.lock().unwrap().insert(id.clone());
        assert!(delete_for_state(&state, &id).is_err());
    }

    #[test]
    fn pending_jobs_and_running_sessions_block_deletion() {
        let (_dir, state, id) = state();
        state
            .store
            .put("capture", "job", &json!({"draftId":id,"status":"pending"}))
            .unwrap();
        assert!(delete_for_state(&state, &id).is_err());
        state.store.remove_many(&[("capture", "job")]).unwrap();
        state
            .store
            .put("draft", &id, &json!({"id":id,"sessionId":"session"}))
            .unwrap();
        assert!(delete_for_state(&state, &id).is_err());
        assert!(state.store.draft(&id).is_ok());
        state
            .store
            .put("session", "session", &json!({"stoppedAt":null}))
            .unwrap();
        assert!(delete_for_state(&state, &id).is_err());
        state
            .store
            .put(
                "session",
                "session",
                &json!({"stoppedAt":"2026-10-09T00:00:00Z"}),
            )
            .unwrap();
        delete_for_state(&state, &id).unwrap();
    }

    fn sibling(state: &AppState) -> String {
        let id = uuid();
        state
            .store
            .put("draft", &id, &json!({"id":id,"sessionId":null}))
            .unwrap();
        state
            .store
            .put(
                "draft_scope",
                &id,
                &workspace::current_scope(state, "").unwrap(),
            )
            .unwrap();
        id
    }

    #[test]
    fn bulk_deletion_removes_only_confirmed_drafts_and_their_records() {
        let (dir, state, first) = state();
        let second = sibling(&state);
        let later = sibling(&state);
        for id in [&first, &second] {
            state.store.put("snapshot", id, &json!({})).unwrap();
            state
                .store
                .put(
                    "capture",
                    id,
                    &json!({"id":id,"draftId":id,"status":"complete"}),
                )
                .unwrap();
        }
        delete_many_for_state(&state, &[first.clone(), second.clone()]).unwrap();
        let reopened = crate::store::Store::new(dir.path()).unwrap();
        for id in [&first, &second] {
            for kind in ["draft", "draft_scope", "snapshot", "capture"] {
                assert!(reopened.get::<Value>(kind, id).unwrap().is_none());
            }
        }
        assert!(reopened.draft(&later).is_ok());
    }

    #[test]
    fn bulk_deletion_rejects_foreign_scope_without_partial_deletion() {
        let (_dir, state, first) = state();
        let foreign = sibling(&state);
        state
            .store
            .put("draft_scope", &foreign, &"foreign-project")
            .unwrap();
        assert!(delete_many_for_state(&state, &[first.clone(), foreign.clone()]).is_err());
        assert!(state.store.draft(&first).is_ok());
        assert!(state.store.draft(&foreign).is_ok());
    }

    #[test]
    fn bulk_deletion_preserves_every_draft_if_any_target_is_protected() {
        for protection in [
            "submission",
            "capture",
            "session",
            "lock",
            "missing-session",
        ] {
            let (_dir, state, first) = state();
            let second = sibling(&state);
            match protection {
                "submission" => state
                    .store
                    .put("submission", &second, &json!({"state":"create_unknown"}))
                    .unwrap(),
                "capture" => state
                    .store
                    .put(
                        "capture",
                        "job",
                        &json!({"draftId":second,"status":"pending"}),
                    )
                    .unwrap(),
                "session" | "missing-session" => {
                    state
                        .store
                        .put("draft", &second, &json!({"id":second,"sessionId":"active"}))
                        .unwrap();
                    if protection == "session" {
                        state
                            .store
                            .put("session", "active", &json!({"stoppedAt":null}))
                            .unwrap();
                    }
                }
                "lock" => {
                    state.locks.lock().unwrap().insert(second.clone());
                }
                _ => unreachable!(),
            }
            assert!(delete_many_for_state(&state, &[first.clone(), second.clone()]).is_err());
            assert!(state.store.draft(&first).is_ok(), "{protection}");
            assert!(state.store.draft(&second).is_ok(), "{protection}");
        }
    }

    #[test]
    fn bulk_deletion_rejects_empty_or_invalid_confirmation() {
        let (_dir, state, first) = state();
        assert!(delete_many_for_state(&state, &[]).is_err());
        assert!(delete_many_for_state(&state, &[first.clone(), "invalid".into()]).is_err());
        assert!(state.store.draft(&first).is_ok());
    }
}
