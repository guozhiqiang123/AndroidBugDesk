use crate::{
    commands::AppState,
    diagnostics, evidence,
    model::{attachments, now, string, uuid, Attachment, DeveloperOptions, Result},
    store::Store,
    workspace,
};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use tauri::State;
use zip::write::SimpleFileOptions;

const BUNDLE_SOURCE_LIMIT: usize = 20 * 1024 * 1024;
const BUNDLE_FILE_LIMIT: u64 = 25 * 1024 * 1024;
const TEXT_FILE_LIMIT: usize = 256 * 1024;
const BUGREPORT_DIRECTORY_LIMIT: u64 = 4 * 1024 * 1024;
const BUGREPORT_DIRECTORY_COUNT: usize = 3;
const BUGREPORT_RETENTION: Duration = Duration::from_secs(72 * 60 * 60);

#[tauri::command]
pub async fn collect_software_evidence(id: String, state: State<'_, AppState>) -> Result<Value> {
    uuid::Uuid::parse_str(&id).map_err(|_| "草稿 ID 格式不正确".to_string())?;
    let options = state
        .store
        .get::<DeveloperOptions>("settings", "developer_options")?
        .unwrap_or_default();
    if !options.debug_enabled {
        return Err("请先在设置的开发者选项中开启调试".into());
    }
    state.editable(&id)?;
    let scope = workspace::current_scope(&state, "")?;
    if state.store.get::<String>("draft_scope", &id)?.as_deref() != Some(&scope) {
        return Err("草稿不属于当前账号或项目".into());
    }
    let original = state.store.draft(&id)?;
    let operation = diagnostics::OperationGuard::begin(
        "software_evidence",
        "collect",
        30,
        json!({"draftBound":true}),
    );
    let _ = diagnostics::flush(Duration::from_secs(2));
    prune_bugreport_diagnostics(&state.store.root);

    let adb_path = { state.adb.lock().unwrap().clone() };
    let adb_version = match adb_path {
        Some(path) => crate::adb::output(&path, None, &["version"], 5)
            .await
            .map(|value| String::from_utf8_lossy(&value).into_owned())
            .unwrap_or_else(|error| format!("读取失败：{error}")),
        None => "未配置 ADB".into(),
    };
    let secrets = sensitive_values(&state, &original)?;
    let temp = state
        .store
        .root
        .join(format!(".software-evidence-{}.zip", uuid()));
    let result = build_bundle(&temp, &state.store.root, &adb_version, &secrets).and_then(|()| {
        evidence::inspect(
            &temp,
            "report",
            "Android Bug Desk 软件运行诊断",
            &state.store.root,
        )
    });
    let _ = fs::remove_file(&temp);
    let mut attachment = match result {
        Ok(attachment) => attachment,
        Err(error) => {
            operation.finish("failed", json!({"reason":error}));
            return Err(error);
        }
    };
    attachment.name = format!(
        "software-evidence-{}.zip",
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    );

    let _write = state.store.writes.lock().unwrap();
    state.editable(&id)?;
    if workspace::current_scope(&state, "")? != scope
        || state.store.get::<String>("draft_scope", &id)?.as_deref() != Some(&scope)
    {
        let _ = evidence::remove_managed_attachment(&state.store.root, &attachment);
        operation.finish("stale_workspace", json!({}));
        return Err("账号或项目已变化，请重新收集软件证据".into());
    }
    let mut draft = state.store.draft(&id)?;
    if draft != original || string(&draft, "updatedAt") != string(&original, "updatedAt") {
        let _ = evidence::remove_managed_attachment(&state.store.root, &attachment);
        operation.finish("stale_draft", json!({}));
        return Err("生成软件证据期间草稿已变化，请重试".into());
    }
    let previous_records: Vec<Value> = state
        .store
        .list("capture")?
        .into_iter()
        .filter(|record| {
            string(record, "draftId") == id && string(record, "kind") == "software-diagnostics"
        })
        .collect();
    let previous_ids: HashSet<String> = previous_records
        .iter()
        .flat_map(record_attachment_ids)
        .collect();
    let previous_files: Vec<Attachment> = attachments(&draft)?
        .into_iter()
        .filter(|item| previous_ids.contains(&item.id))
        .collect();
    draft["attachments"]
        .as_array_mut()
        .ok_or("附件数据格式错误")?
        .retain(|item| !previous_ids.contains(&string(item, "id")));
    draft["attachments"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::to_value(&attachment).map_err(|error| error.to_string())?);
    draft["previewConfirmed"] = json!(false);
    draft["updatedAt"] = json!(now());
    crate::validation::check_shape(&draft)?;
    if let Err(error) = state.assert_no_token(&draft) {
        let _ = evidence::remove_managed_attachment(&state.store.root, &attachment);
        operation.finish("secret_rejected", json!({}));
        return Err(error);
    }
    let capture_id = uuid();
    let captured_at = now();
    let capture = json!({
        "id": capture_id,
        "draftId": id,
        "kind": "software-diagnostics",
        "status": "complete",
        "scope": scope,
        "startedAt": captured_at,
        "finishedAt": captured_at,
        "attachments": [attachment.id]
    });
    state
        .store
        .put_many(&[("draft", &id, &draft), ("capture", &capture_id, &capture)])?;
    if cleanup_records_and_files(&state.store, &previous_records, &previous_files).is_err() {
        diagnostics::base_event(
            "software_evidence",
            "replaced_attachment_cleanup_failed",
            json!({"attachmentCount":previous_files.len()}),
        );
    }
    operation.finish(
        "complete",
        json!({"bundleBytes":attachment.size,"replaced":previous_files.len()}),
    );
    Ok(draft)
}

fn build_bundle(path: &Path, root: &Path, adb_version: &str, secrets: &[String]) -> Result<()> {
    let mut entries: Vec<(String, Vec<u8>)> = vec![
        (
            "README.txt".into(),
            "Android Bug Desk 软件运行诊断。包含受限轮转日志、当前运行任务、主机与 ADB 客户端信息；不包含云效凭据、草稿正文和设备业务数据。\n"
                .as_bytes()
                .to_vec(),
        ),
        (
            "host.json".into(),
            serde_json::to_vec_pretty(&json!({
                "appVersion": env!("CARGO_PKG_VERSION"),
                "os": std::env::consts::OS,
                "arch": std::env::consts::ARCH,
                "family": std::env::consts::FAMILY,
                "timezoneOffset": chrono::Local::now().offset().to_string(),
                "debugEnabled": diagnostics::debug_enabled(),
                "runId": diagnostics::run_id(),
                "adbVersion": redact(adb_version, secrets),
            }))
            .map_err(|error| error.to_string())?,
        ),
        (
            "active-operations.json".into(),
            serde_json::to_vec_pretty(&diagnostics::active_operations())
                .map_err(|error| error.to_string())?,
        ),
    ];
    if let Some(directory) = diagnostics::directory() {
        for name in [
            "runtime.log",
            "runtime.log.1",
            "debug.jsonl",
            "debug.jsonl.1",
            "debug.jsonl.2",
            "debug.jsonl.3",
            "debug.jsonl.4",
        ] {
            let file = directory.join(name);
            if file.is_file() {
                let bytes = read_tail(&file, TEXT_FILE_LIMIT)?;
                entries.push((format!("logs/{name}"), redact_bytes(bytes, secrets)));
            }
        }
    }
    append_bugreport_summaries(root, secrets, &mut entries)?;
    let manifest = json!({
        "createdAt": now(),
        "limits": {"bundleBytes":BUNDLE_FILE_LIMIT,"sourceBytes":BUNDLE_SOURCE_LIMIT,"textTailBytes":TEXT_FILE_LIMIT},
        "files": entries.iter().map(|(name,data)| json!({"name":name,"bytes":data.len()})).collect::<Vec<_>>()
    });
    entries.push((
        "manifest.json".into(),
        serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?,
    ));
    let source_bytes: usize = entries.iter().map(|(_, data)| data.len()).sum();
    if source_bytes > BUNDLE_SOURCE_LIMIT {
        return Err("软件证据源数据超过 20 MB，已停止打包".into());
    }
    let file = File::create(path).map_err(|error| error.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, data) in entries {
        zip.start_file(name, options)
            .map_err(|error| error.to_string())?;
        zip.write_all(&data).map_err(|error| error.to_string())?;
    }
    zip.finish().map_err(|error| error.to_string())?;
    if fs::metadata(path).map_err(|error| error.to_string())?.len() > BUNDLE_FILE_LIMIT {
        let _ = fs::remove_file(path);
        return Err("软件证据压缩包超过 25 MB，已停止保存".into());
    }
    Ok(())
}

fn append_bugreport_summaries(
    root: &Path,
    secrets: &[String],
    entries: &mut Vec<(String, Vec<u8>)>,
) -> Result<()> {
    let base = root.join("bugreports");
    let mut directories = recent_directories(&base);
    directories.truncate(BUGREPORT_DIRECTORY_COUNT);
    for (index, directory) in directories.into_iter().enumerate() {
        let receipt_path = directory.join("receipt.json");
        let mut local_secrets = secrets.to_vec();
        let sanitized_receipt = if receipt_path.is_file() {
            let bytes = read_tail(&receipt_path, TEXT_FILE_LIMIT)?;
            serde_json::from_slice::<Value>(&bytes)
                .ok()
                .map(|mut receipt| {
                    redact_receipt_fields(&mut receipt, &mut local_secrets);
                    serde_json::to_vec_pretty(&receipt).unwrap_or_default()
                })
        } else {
            None
        };
        for name in ["receipt.json", "stdout.log", "stderr.log"] {
            let file = directory.join(name);
            if file.is_file() {
                let bytes = if name == "receipt.json" {
                    sanitized_receipt
                        .clone()
                        .unwrap_or(read_tail(&file, TEXT_FILE_LIMIT)?)
                } else {
                    read_tail(&file, TEXT_FILE_LIMIT)?
                };
                entries.push((
                    format!("adb-attempts/{index}/{name}"),
                    redact_bytes(bytes, &local_secrets),
                ));
            }
        }
    }
    Ok(())
}

fn redact_receipt_fields(value: &mut Value, secrets: &mut Vec<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                redact_receipt_fields(item, secrets);
            }
        }
        Value::Object(fields) => {
            for (key, value) in fields {
                let normalized = key.to_ascii_lowercase();
                if [
                    "serial",
                    "targetpackage",
                    "packagename",
                    "path",
                    "directory",
                    "zippath",
                    "jenkinspackageurl",
                    "title",
                ]
                .contains(&normalized.as_str())
                {
                    if let Some(secret) = value.as_str().filter(|secret| secret.len() >= 4) {
                        secrets.push(secret.to_owned());
                    }
                    *value = Value::String("<REDACTED>".into());
                } else {
                    redact_receipt_fields(value, secrets);
                }
            }
        }
        _ => {}
    }
}

fn sensitive_values(state: &AppState, draft: &Value) -> Result<Vec<String>> {
    let mut values = Vec::new();
    if let Some(token) = state.token.lock().unwrap().as_ref() {
        values.push(token.clone());
    }
    for key in [
        "jenkinsPackageUrl",
        "title",
        "actual",
        "expected",
        "preconditions",
        "steps",
        "app.packageName",
    ] {
        values.push(string(draft, key));
    }
    for session in state.store.list("session")? {
        values.push(string(&session, "serial"));
        values.push(string(&session, "packageName"));
    }
    values.retain(|value| value.len() >= 4);
    values.sort_by_key(|value| std::cmp::Reverse(value.len()));
    values.dedup();
    Ok(values)
}

fn redact_bytes(bytes: Vec<u8>, secrets: &[String]) -> Vec<u8> {
    redact(&String::from_utf8_lossy(&bytes), secrets).into_bytes()
}

fn redact(value: &str, secrets: &[String]) -> String {
    let mut value = diagnostics::redact_payload_text(value);
    for secret in secrets {
        value = value.replace(secret, "<REDACTED>");
    }
    value
}

fn read_tail(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let length = file.metadata().map_err(|error| error.to_string())?.len();
    if length > limit as u64 {
        file.seek(SeekFrom::End(-(limit as i64)))
            .map_err(|error| error.to_string())?;
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

fn record_attachment_ids(record: &Value) -> impl Iterator<Item = String> + '_ {
    record["attachments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
}

pub(crate) fn cleanup_removed(store: &Store, draft_id: &str, removed: &[Attachment]) -> Result<()> {
    let removed_ids: HashSet<&str> = removed.iter().map(|item| item.id.as_str()).collect();
    let records: Vec<Value> = store
        .list("capture")?
        .into_iter()
        .filter(|record| {
            string(record, "draftId") == draft_id
                && string(record, "kind") == "software-diagnostics"
                && record_attachment_ids(record).any(|id| removed_ids.contains(id.as_str()))
        })
        .collect();
    let owned_ids: HashSet<String> = records.iter().flat_map(record_attachment_ids).collect();
    let owned_files: Vec<Attachment> = removed
        .iter()
        .filter(|attachment| owned_ids.contains(&attachment.id))
        .cloned()
        .collect();
    cleanup_records_and_files(store, &records, &owned_files)
}

fn cleanup_records_and_files(
    store: &Store,
    records: &[Value],
    removed: &[Attachment],
) -> Result<()> {
    let record_ids: Vec<String> = records.iter().map(|record| string(record, "id")).collect();
    let keys: Vec<(&str, &str)> = record_ids
        .iter()
        .map(|id| ("capture", id.as_str()))
        .collect();
    store.remove_many(&keys)?;
    let referenced: HashSet<String> = store
        .list("draft")?
        .iter()
        .flat_map(|draft| {
            draft["attachments"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|item| item["id"].as_str())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect();
    let mut failures = Vec::new();
    for attachment in removed {
        if referenced.contains(&attachment.id) {
            continue;
        }
        if let Err(error) = evidence::remove_managed_attachment(&store.root, attachment) {
            failures.push(error);
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "软件证据记录已移除，但部分文件清理失败：{}",
            failures.join("；")
        ))
    }
}

pub(crate) fn prune_bugreport_diagnostics(root: &Path) {
    let base = root.join("bugreports");
    let directories = recent_directories(&base);
    let now = SystemTime::now();
    for (index, directory) in directories.into_iter().enumerate() {
        let modified = directory
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if index >= BUGREPORT_DIRECTORY_COUNT
            || now.duration_since(modified).unwrap_or_default() > BUGREPORT_RETENTION
        {
            let _ = fs::remove_dir_all(directory);
            continue;
        }
        if directory_size(&directory) > BUGREPORT_DIRECTORY_LIMIT {
            compact_bugreport_directory(&directory);
        }
        if directory_size(&directory) > BUGREPORT_DIRECTORY_LIMIT {
            let _ = fs::remove_dir_all(directory);
        }
    }
}

fn recent_directories(base: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(base) else {
        return Vec::new();
    };
    let mut directories: Vec<PathBuf> = entries
        .flatten()
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|kind| kind.is_dir() && !kind.is_symlink())
                .map(|_| entry.path())
        })
        .collect();
    directories.sort_by_key(|path| {
        std::cmp::Reverse(
            path.metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH),
        )
    });
    directories
}

fn directory_size(path: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| {
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_file() => {
                    entry.metadata().map(|value| value.len()).unwrap_or(0)
                }
                Ok(kind) if kind.is_dir() && !kind.is_symlink() => directory_size(&path),
                _ => 0,
            }
        })
        .sum()
}

fn compact_bugreport_directory(path: &Path) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let file = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() && !kind.is_symlink() {
            let _ = fs::remove_dir_all(file);
            continue;
        }
        if !kind.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if matches!(name.as_ref(), "stdout.log" | "stderr.log") {
            if let Ok(tail) = read_tail(&file, TEXT_FILE_LIMIT) {
                let _ = fs::write(file, tail);
            }
        } else if name != "receipt.json" {
            let _ = fs::remove_file(file);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn bugreport_retention_has_a_hard_count_and_size_bound() {
        let root = tempdir().unwrap();
        let base = root.path().join("bugreports");
        fs::create_dir_all(&base).unwrap();
        for index in 0..5 {
            let directory = base.join(format!("job-{index}"));
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("partial.zip"), vec![0u8; 5 * 1024 * 1024]).unwrap();
            fs::write(directory.join("stderr.log"), vec![b'x'; 512 * 1024]).unwrap();
            fs::write(directory.join("receipt.json"), b"{}").unwrap();
        }
        prune_bugreport_diagnostics(root.path());
        let directories = recent_directories(&base);
        assert!(directories.len() <= BUGREPORT_DIRECTORY_COUNT);
        assert!(directories
            .iter()
            .all(|directory| directory_size(directory) <= BUGREPORT_DIRECTORY_LIMIT));
    }

    #[test]
    fn bundle_redacts_exact_business_values() {
        let value = redact(
            "token=secret-value package=com.example.private /Users/person/data",
            &["secret-value".into(), "com.example.private".into()],
        );
        assert!(!value.contains("secret-value"));
        assert!(!value.contains("com.example.private"));
        assert!(!value.contains("/Users/person"));

        let mut receipt = json!({
            "serial":"DEVICE-1234",
            "directory":"C:\\Users\\tester\\AppData\\Local\\report",
            "nested":{"targetPackage":"com.example.private"},
            "timedOut":true
        });
        let mut discovered = Vec::new();
        redact_receipt_fields(&mut receipt, &mut discovered);
        assert_eq!(receipt["serial"], "<REDACTED>");
        assert_eq!(receipt["nested"]["targetPackage"], "<REDACTED>");
        assert_eq!(receipt["timedOut"], true);
        assert!(discovered.contains(&"DEVICE-1234".to_string()));
    }

    #[test]
    fn cleanup_deletes_only_software_evidence_owned_by_capture_record() {
        let root = tempdir().unwrap();
        let store = Store::new(root.path()).unwrap();
        let imported_source = root.path().join("imported.txt");
        fs::write(&imported_source, b"user supplied attachment").unwrap();
        let imported =
            evidence::inspect(&imported_source, "log", "用户导入附件", root.path()).unwrap();
        let software_source = root.path().join("software.zip");
        {
            let file = File::create(&software_source).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            zip.start_file("manifest.json", SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"{}").unwrap();
            zip.finish().unwrap();
        }
        let software = evidence::inspect(
            &software_source,
            "report",
            "Android Bug Desk 软件运行诊断",
            root.path(),
        )
        .unwrap();
        store
            .put(
                "draft",
                "draft-a",
                &json!({"id":"draft-a","attachments":[]}),
            )
            .unwrap();
        store
            .put(
                "capture",
                "capture-a",
                &json!({
                    "id":"capture-a","draftId":"draft-a","kind":"software-diagnostics",
                    "attachments":[software.id]
                }),
            )
            .unwrap();
        let imported_path = PathBuf::from(&imported.path);
        let software_path = PathBuf::from(&software.path);

        cleanup_removed(&store, "draft-a", &[imported.clone(), software.clone()]).unwrap();

        assert!(imported_path.exists());
        assert!(!software_path.exists());
        assert!(store
            .get::<Value>("capture", "capture-a")
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn collecting_twice_replaces_the_managed_attachment_and_remove_all_cleans_it() {
        use tauri::Manager;

        let root = tempdir().unwrap();
        let state = AppState::new(root.path().to_path_buf()).unwrap();
        *state.adb.lock().unwrap() = None;
        *state.user.lock().unwrap() = Some(workspace::UserIdentity {
            id: "developer".into(),
            ..Default::default()
        });
        let mut profile = workspace::empty_profile();
        profile.organization_id = "org".into();
        profile.project_id = "project".into();
        *state.profile.lock().unwrap() = profile;
        let mut draft: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/valid-draft.json")).unwrap();
        let id = uuid();
        draft["id"] = json!(id);
        draft["attachments"] = json!([]);
        state.store.put("draft", &id, &draft).unwrap();
        let scope = workspace::current_scope(&state, "").unwrap();
        state.store.put("draft_scope", &id, &scope).unwrap();
        state
            .store
            .put(
                "settings",
                "developer_options",
                &DeveloperOptions {
                    debug_enabled: true,
                },
            )
            .unwrap();
        let app = tauri::test::mock_builder()
            .manage(state)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();

        let first = collect_software_evidence(id.clone(), app.state())
            .await
            .unwrap();
        let first_attachment = attachments(&first).unwrap().pop().unwrap();
        assert!(Path::new(&first_attachment.path).is_file());

        let second = collect_software_evidence(id.clone(), app.state())
            .await
            .unwrap();
        let software: Vec<_> = attachments(&second)
            .unwrap()
            .into_iter()
            .filter(|item| item.source == "Android Bug Desk 软件运行诊断")
            .collect();
        assert_eq!(software.len(), 1);
        assert_ne!(software[0].id, first_attachment.id);
        assert!(!Path::new(&first_attachment.path).exists());
        assert_eq!(
            app.state::<AppState>()
                .store
                .list("capture")
                .unwrap()
                .into_iter()
                .filter(|record| string(record, "kind") == "software-diagnostics")
                .count(),
            1
        );

        let current_path = software[0].path.clone();
        crate::commands::remove_all_attachments(id, app.state()).unwrap();
        assert!(!Path::new(&current_path).exists());
        assert!(app
            .state::<AppState>()
            .store
            .list("capture")
            .unwrap()
            .into_iter()
            .all(|record| string(&record, "kind") != "software-diagnostics"));
    }
}
