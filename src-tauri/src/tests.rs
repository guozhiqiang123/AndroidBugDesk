use crate::{
    adb, evidence, model::*, store::Store, submission::Submitter, validation, yunxiao::Api,
};
use serde_json::{json, Value};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};
fn fixture() -> Value {
    serde_json::from_str(include_str!("../../tests/fixtures/valid-draft.json")).unwrap()
}
fn metadata() -> Metadata {
    serde_json::from_str(include_str!("../../tests/fixtures/metadata.json")).unwrap()
}
fn setup() -> (tempfile::TempDir, Store, Value) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::new(temp.path()).unwrap();
    let log = temp.path().join("logcat.log");
    std::fs::write(&log, "1728460800.000 100 100 E App: test crash\n").unwrap();
    let mut a = evidence::inspect(&log, "log", "导入测试日志", temp.path()).unwrap();
    a.confirmed = true;
    a.coverage_start = "2026-10-09T07:59:00+08:00".into();
    a.coverage_end = "2026-10-09T08:01:00+08:00".into();
    let mut d = fixture();
    d["attachments"] = json!([a]);
    store.put("draft", &string(&d, "id"), &d).unwrap();
    (temp, store, d)
}
#[test]
fn rust_and_ui_fixture_accept_version_zero() {
    let m = metadata();
    let d = fixture();
    assert!(validation::validate(&d, &m.profile, Some(&m), true, true).is_empty());
}

#[test]
fn preview_dialog_does_not_require_a_second_confirmation_flag() {
    let m = metadata();
    let mut d = fixture();
    d["previewConfirmed"] = json!(false);
    assert!(validation::validate(&d, &m.profile, Some(&m), true, true).is_empty());
}

#[test]
fn developer_options_are_persisted_outside_login_state() {
    use tauri::Manager;

    let root = tempfile::tempdir().unwrap();
    let state = crate::commands::AppState::new(root.path().to_path_buf()).unwrap();
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let saved = DeveloperOptions {
        debug_enabled: true,
        agent_access_enabled: false,
    };
    crate::commands::persist_developer_options(&saved, &app.state()).unwrap();
    assert!(saved.debug_enabled);
    assert!(!saved.agent_access_enabled);
    let stored: DeveloperOptions = app
        .state::<crate::commands::AppState>()
        .store
        .get("settings", "developer_options")
        .unwrap()
        .unwrap();
    assert!(stored.debug_enabled);
    assert!(!stored.agent_access_enabled);
    assert_eq!(
        crate::commands::bootstrap(app.state()).unwrap()["developerOptions"]["debugEnabled"],
        true
    );
}

#[test]
fn shortcut_settings_are_persisted_and_reject_conflicts_or_editing_keys() {
    use tauri::Manager;

    let root = tempfile::tempdir().unwrap();
    let state = crate::commands::AppState::new(root.path().to_path_buf()).unwrap();
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let saved = crate::commands::set_shortcut_settings(
        ShortcutSettings {
            version: 1,
            overrides: [("newIssue".into(), "Primary+Shift+1".into())]
                .into_iter()
                .collect(),
        },
        app.state(),
    )
    .unwrap();
    assert_eq!(saved.overrides["newIssue"], "Primary+Shift+1");
    assert_eq!(
        crate::commands::bootstrap(app.state()).unwrap()["shortcutSettings"]["overrides"]
            ["newIssue"],
        "Primary+Shift+1"
    );

    for overrides in [
        [("newIssue".into(), "Primary+e".into())]
            .into_iter()
            .collect(),
        [
            ("newIssue".into(), "Primary+Shift+e".into()),
            ("postCapture".into(), "Primary+Shift+e".into()),
        ]
        .into_iter()
        .collect(),
        [("newIssue".into(), "Primary+Shift+v".into())]
            .into_iter()
            .collect(),
        [("unknown".into(), "Primary+Shift+n".into())]
            .into_iter()
            .collect(),
    ] {
        assert!(crate::commands::set_shortcut_settings(
            ShortcutSettings {
                version: 1,
                overrides,
            },
            app.state(),
        )
        .is_err());
    }
}

#[test]
fn late_capture_field_is_removed_from_draft_and_report() {
    let mut d = fixture();
    assert!(d["event"].get("lateCapture").is_none());
    validation::check_shape(&d).unwrap();
    assert!(!evidence::report(&d, &metadata().profile).contains("事后补采"));
    d["event"]["lateCapture"] = json!(true);
    assert!(validation::check_shape(&d).is_err());
}

#[test]
fn attachments_are_optional_for_all_kinds() {
    let m = metadata();
    for kind in [
        "功能",
        "UI",
        "多语言",
        "动态过程",
        "崩溃",
        "ANR",
        "广告",
        "性能",
        "网络",
        "数据",
    ] {
        let mut d = fixture();
        d["kind"] = json!(kind);
        d["attachments"] = json!([]);
        d["network"] = json!({"operation":"请求接口","type":"Wi-Fi","error":"超时","collectionStatus":"已记录"});
        d["data"] = json!({"type":"图片","count":"1","source":"本地测试数据"});
        for online in [false, true] {
            let issues = validation::validate(&d, &m.profile, Some(&m), true, online);
            assert!(issues.is_empty(), "{kind}, online={online}: {issues:?}");
        }
    }
}

#[tokio::test]
async fn remove_all_attachments_uses_an_authoritative_backend_mutation() {
    use tauri::Manager;

    let (server, _remote, api) = mock(Remote::default()).await;
    let (_temp, store, d) = setup();
    let id = string(&d, "id");
    bind_alice(&store, &id);
    let state = submission_state(&store, &server, &api.profile);
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();

    let mut renderer_echo = d;
    renderer_echo["attachments"] = json!([]);
    let preserved = crate::commands::save_draft(renderer_echo, app.state()).unwrap();
    assert_eq!(
        preserved["attachments"].as_array().unwrap().len(),
        1,
        "ordinary draft saves must not replace backend-managed attachments"
    );

    let removed = crate::commands::remove_all_attachments(id.clone(), app.state()).unwrap();
    assert!(removed["attachments"].as_array().unwrap().is_empty());
    assert_eq!(removed["previewConfirmed"], false);
    assert!(store.draft(&id).unwrap()["attachments"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn every_log_is_validated_independently_of_legacy_confirmation() {
    let m = metadata();
    let mut d = fixture();
    let mut invalid = d["attachments"][0].clone();
    invalid["id"] = json!("00000000-0000-4000-8000-000000000077");
    invalid["size"] = json!(0);
    invalid["source"] = json!("");
    invalid["coverageStart"] = json!("");
    invalid["coverageEnd"] = json!("");
    d["attachments"].as_array_mut().unwrap().push(invalid);
    let issues = validation::validate(&d, &m.profile, Some(&m), true, true);
    assert_eq!(
        issues
            .iter()
            .filter(|i| i
                .field
                .starts_with("attachment.00000000-0000-4000-8000-000000000077."))
            .count(),
        3
    );
    d["attachments"][1]["confirmed"] = json!(false);
    assert_eq!(
        validation::validate(&d, &m.profile, Some(&m), true, true)
            .iter()
            .filter(|i| i
                .field
                .starts_with("attachment.00000000-0000-4000-8000-000000000077."))
            .count(),
        3
    );
}

#[tokio::test]
async fn mock_submission_without_imported_attachments_completes() {
    let (_server, remote, api) = mock(Remote::default()).await;
    let (_temp, store, mut d) = setup();
    d["attachments"] = json!([]);
    d["kind"] = json!("动态过程");
    store.put("draft", &string(&d, "id"), &d).unwrap();
    let result = Submitter { store }
        .submit(&string(&d, "id"), &api)
        .await
        .unwrap();
    assert_eq!(result.state, "submitted");
    assert!(result.evidence.files.is_empty());
    let r = remote.lock().unwrap();
    assert_eq!(r.creates, 1);
    assert_eq!(r.files.len(), 1); // Only the client's generated report/manifest ZIP.
}

#[tokio::test]
async fn legacy_confirmation_flag_does_not_exclude_imported_files() {
    let (_server, remote, api) = mock(Remote::default()).await;
    let (_temp, store, mut d) = setup();
    d["attachments"][0]["confirmed"] = json!(false);
    let id = string(&d, "id");
    store.put("draft", &id, &d).unwrap();
    let exported = evidence::build(&store, &d, &api.profile, true).unwrap();
    assert_eq!(exported.files.len(), 1);
    assert_eq!(
        exported.files[0].id,
        d["attachments"][0]["id"].as_str().unwrap()
    );
    let result = Submitter { store }.submit(&id, &api).await.unwrap();
    assert_eq!(result.state, "submitted");
    assert_eq!(
        result.uploads.len(),
        2,
        "original attachment and evidence ZIP"
    );
    assert_eq!(remote.lock().unwrap().files.len(), 2);
}

#[tokio::test]
async fn unified_attachment_import_classifies_mixed_files_and_preserves_batch_on_error() {
    let (server, _remote, api) = mock(Remote::default()).await;
    let (_temp, store, d) = setup();
    let id = string(&d, "id");
    bind_alice(&store, &id);
    let state = submission_state(&store, &server, &api.profile);
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures");
    let json_path = store.root.join("测试 数据.JSON");
    std::fs::write(&json_path, "{\"value\":1}").unwrap();
    let text_path = store.root.join("notes.txt");
    std::fs::write(&text_path, format!("{}中", "x".repeat(4095))).unwrap();
    let zip_path = store.root.join("report.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
    zip.start_file("report.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    std::io::Write::write_all(&mut zip, b"report").unwrap();
    zip.finish().unwrap();
    let paths = [
        fixtures.join("evidence.png"),
        fixtures.join("evidence.jpg"),
        fixtures.join("evidence.mp4"),
        zip_path,
        json_path,
        text_path,
    ];
    let imported = crate::commands::import_attachment_paths(&state, &id, &paths).unwrap();
    let files = attachments(&imported).unwrap();
    assert_eq!(
        files
            .iter()
            .skip(1)
            .map(|a| a.role.as_str())
            .collect::<Vec<_>>(),
        ["image", "image", "video", "report", "sample", "log"]
    );
    for a in files.iter().skip(1) {
        assert!(!a.confirmed);
        assert!(a.coverage_start.is_empty() && a.coverage_end.is_empty());
        evidence::verify_attachment(a).unwrap();
        let verified = crate::commands::preview_attachment_for_state(&state, &id, &a.id).unwrap();
        let mut dispatched = false;
        crate::commands::open_attachment_with(&verified, |path, _| {
            assert_eq!(path, Path::new(&verified.path));
            dispatched = true;
            Ok(())
        })
        .unwrap();
        assert!(dispatched);
    }
    let broken = store.root.join("broken.png");
    std::fs::write(&broken, b"\x89PNG\r\n\x1a\ninvalid").unwrap();
    assert!(
        crate::commands::import_attachment_paths(&state, &id, &[paths[5].clone(), broken]).is_err()
    );
    assert_eq!(store.draft(&id).unwrap(), imported);
    change_identity(&state, &api.profile, "account");
    assert!(crate::commands::import_attachment_paths(&state, &id, &paths).is_err());
    assert_eq!(store.draft(&id).unwrap(), imported);
}

#[tokio::test]
async fn attachment_system_preview_rejects_wrong_scope_changed_files_and_external_paths() {
    let (server, _remote, api) = mock(Remote::default()).await;
    let (_temp, store, d) = setup();
    let id = string(&d, "id");
    bind_alice(&store, &id);
    let state = submission_state(&store, &server, &api.profile);
    let a = attachments(&d).unwrap().remove(0);
    assert!(crate::commands::preview_attachment_for_state(&state, &id, "missing").is_err());
    for dimension in ["account", "organization", "project"] {
        change_identity(&state, &api.profile, dimension);
        assert!(crate::commands::preview_attachment_for_state(&state, &id, &a.id).is_err());
    }
    change_identity(&state, &api.profile, "original");
    let verified = crate::commands::preview_attachment_for_state(&state, &id, &a.id).unwrap();
    let error = crate::commands::open_attachment_with(&verified, |_, _| {
        Err(std::io::Error::from(std::io::ErrorKind::NotFound))
    })
    .unwrap_err();
    assert!(error.contains("默认打开应用"));
    state.locks.lock().unwrap().insert(id.clone());
    assert!(crate::commands::preview_attachment_for_state(&state, &id, &a.id).is_ok());
    state.locks.lock().unwrap().clear();
    let mut foreign = d.clone();
    foreign["attachments"][0]["path"] = json!(store.root.join("logcat.log").to_string_lossy());
    store.put("draft", &id, &foreign).unwrap();
    assert!(crate::commands::preview_attachment_for_state(&state, &id, &a.id).is_err());
    store.put("draft", &id, &d).unwrap();
    std::fs::write(&a.path, "changed").unwrap();
    assert!(crate::commands::preview_attachment_for_state(&state, &id, &a.id).is_err());
    let mut called = false;
    assert!(crate::commands::open_attachment_with(&a, |_, _| {
        called = true;
        Ok(())
    })
    .is_err());
    assert!(!called);
}

#[test]
fn mac_image_preview_uses_preview_instead_of_file_associations() {
    let (_temp, store, _) = setup();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures");
    for name in ["evidence.png", "evidence.jpg"] {
        // Dispatch is based on the inspected image type even with an unrelated extension.
        let source = store.root.join(format!("{name}.txt"));
        std::fs::copy(fixtures.join(name), &source).unwrap();
        let a = evidence::inspect_import(&source, &store.root).unwrap();
        assert_eq!(a.role, "image");
        let expected = cfg!(target_os = "macos").then_some("/System/Applications/Preview.app");
        let mut calls = 0;
        crate::commands::open_attachment_with(&a, |path, application| {
            assert_eq!(path, Path::new(&a.path));
            assert_eq!(application, expected);
            calls += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(calls, 1);
        let error = crate::commands::open_attachment_with(&a, |_, _| {
            Err(std::io::Error::from(std::io::ErrorKind::NotFound))
        })
        .unwrap_err();
        if cfg!(target_os = "macos") {
            assert!(error.contains("macOS“预览”"));
        } else {
            assert!(error.contains("默认打开应用"));
        }
        assert_eq!(
            std::fs::read(&source).unwrap(),
            std::fs::read(&a.path).unwrap()
        );
    }
    let video = evidence::inspect_import(&fixtures.join("evidence.mp4"), &store.root).unwrap();
    crate::commands::open_attachment_with(&video, |_, application| {
        assert_eq!(application, None);
        Ok(())
    })
    .unwrap();
    let log = evidence::inspect_import(&store.root.join("logcat.log"), &store.root).unwrap();
    crate::commands::open_attachment_with(&log, |_, application| {
        assert_eq!(application, None);
        Ok(())
    })
    .unwrap();
    let mut report = log.clone();
    report.role = "report".into();
    report.mime = "application/zip".into();
    assert_eq!(
        crate::commands::uses_macos_quick_look(&report),
        cfg!(target_os = "macos")
    );
    assert!(!crate::commands::uses_macos_quick_look(&log));
}

#[tokio::test]
async fn system_attachment_preview_does_not_execute_programs_or_scripts() {
    let (server, _remote, api) = mock(Remote::default()).await;
    let (_temp, store, d) = setup();
    let id = string(&d, "id");
    bind_alice(&store, &id);
    let state = submission_state(&store, &server, &api.profile);
    for (name, bytes) in [
        ("test.command", b"echo hello".as_slice()),
        ("renamed.bin", b"\x7fELF\0payload".as_slice()),
        ("renamed.txt", b"#!/bin/sh\necho hello".as_slice()),
    ] {
        let path = store.root.join(name);
        std::fs::write(&path, bytes).unwrap();
        let changed = crate::commands::import_attachment_paths(&state, &id, &[path]).unwrap();
        let a = attachments(&changed).unwrap().pop().unwrap();
        assert!(
            crate::commands::preview_attachment_for_state(&state, &id, &a.id)
                .unwrap_err()
                .contains("程序或脚本")
        );
    }
}

#[test]
fn bug_page_is_optional_and_report_uses_the_effective_value() {
    let m = metadata();
    for page in [None, Some(""), Some("恢复结果页")] {
        let mut d = fixture();
        if let Some(page) = page {
            d["bugPage"] = json!(page);
            if !page.is_empty() {
                d["sources"]["bugPage"] = json!({
                    "status":"manual","at":"2026-10-12T00:00:00Z","detail":"人工输入"
                });
            }
        } else {
            d.as_object_mut().unwrap().remove("bugPage");
        }
        assert!(validation::check_shape(&d).is_ok());
        for online in [true, false] {
            assert!(validation::validate(&d, &m.profile, Some(&m), true, online).is_empty());
        }
        let report = evidence::report(&d, &m.profile);
        assert_eq!(
            report.contains("## Bug所在页面"),
            page.is_some_and(|v| !v.is_empty())
        );
        if let Some(page) = page.filter(|v| !v.is_empty()) {
            assert!(report.contains(page));
            assert!(report.contains("采集时间："));
            assert!(report.contains("来源："));
            assert!(report.contains(&string(&d, "sources.bugPage.detail")));
        }
    }
    let mut legacy = fixture();
    legacy["bugPage"] = json!("com.google.android.apps.nexuslauncher.NexusLauncherActivity");
    legacy["sources"]["bugPage"] = json!({
        "status":"collected","at":"2026-10-10T00:00:00Z","detail":"旧版自动采集"
    });
    let report = evidence::report(&legacy, &m.profile);
    assert!(!report.contains("NexusLauncherActivity"));
    assert!(!report.contains("## Bug所在页面"));
}

#[test]
fn cloud_configuration_is_removed_and_old_field_is_rejected() {
    let m = metadata();
    let mut d = fixture();
    assert!(d.get("cloud").is_none());
    assert!(validation::check_shape(&d).is_ok());
    for online in [true, false] {
        assert!(validation::validate(&d, &m.profile, Some(&m), true, online).is_empty());
    }
    assert!(!evidence::report(&d, &m.profile).contains("云控"));
    d["cloud"] = json!({
        "applicability": "no", "reason": "旧云控依据", "name": "",
        "effectiveValue": "", "source": "", "observedAt": "", "override": ""
    });
    assert!(validation::check_shape(&d).is_err());
}

#[test]
fn hardening_status_is_removed_and_old_field_is_rejected() {
    let m = metadata();
    let mut d = fixture();
    assert!(d["app"].get("hardening").is_none());
    assert!(validation::check_shape(&d).is_ok());
    assert!(validation::validate(&d, &m.profile, Some(&m), true, true).is_empty());
    d["app"]["hardening"] = json!("旧加固状态");
    assert!(validation::check_shape(&d).is_err());
}

#[test]
fn test_environment_is_removed_and_old_field_is_rejected() {
    let m = metadata();
    let mut d = fixture();
    assert!(d["app"].get("environment").is_none());
    for online in [true, false] {
        assert!(validation::validate(&d, &m.profile, Some(&m), true, online).is_empty());
    }
    d["app"]["environment"] = json!("旧测试包");
    assert!(validation::check_shape(&d).is_err());
}

#[test]
fn obfuscation_status_is_removed_and_old_field_is_rejected() {
    let m = metadata();
    let mut d = fixture();
    assert!(d["app"].get("obfuscation").is_none());
    for online in [true, false] {
        assert!(validation::validate(&d, &m.profile, Some(&m), true, online).is_empty());
    }
    assert!(!evidence::report(&d, &m.profile).contains("obfuscation"));
    d["app"]["obfuscation"] = json!("已混淆");
    assert!(validation::check_shape(&d).is_err());
}

#[test]
fn removed_submission_fields_are_skipped_but_near_names_remain() {
    let mut m = metadata();
    m.fields.extend([
        Field {
            id: "customAlias".into(),
            name: "自定义标签映射".into(),
            format: "string".into(),
            field_type: "CustomField".into(),
            required: false,
            show_when_create: true,
            option_error: None,
            options: vec![],
        },
        Field {
            id: "labels".into(),
            name: "标签容器".into(),
            format: "string".into(),
            field_type: "CustomField".into(),
            required: false,
            show_when_create: true,
            option_error: None,
            options: vec![],
        },
        Field {
            id: "planStartTimestamp".into(),
            name: "计划开始时间戳".into(),
            format: "string".into(),
            field_type: "CustomField".into(),
            required: false,
            show_when_create: true,
            option_error: None,
            options: vec![],
        },
        Field {
            id: "estimate".into(),
            name: "预计工时".into(),
            format: "string".into(),
            field_type: "CustomField".into(),
            required: false,
            show_when_create: true,
            option_error: None,
            options: vec![],
        },
        Field {
            id: "hiddenOptional".into(),
            name: "隐藏可选字段".into(),
            format: "unsupported".into(),
            field_type: "UnknownField".into(),
            required: false,
            show_when_create: false,
            option_error: None,
            options: vec![],
        },
        Field {
            id: "hiddenRequiredByProfile".into(),
            name: "隐藏但项目必填".into(),
            format: "string".into(),
            field_type: "CustomField".into(),
            required: false,
            show_when_create: false,
            option_error: None,
            options: vec![],
        },
    ]);
    m.profile
        .field_mappings
        .insert("customAlias".into(), "labels".into());
    let mut d = fixture();
    d["customFields"]["customAlias"] = json!("stale-alias");
    d["customFields"]["labels"] = json!("stale-label");
    d["customFields"]["planStartTimestamp"] = json!("2026-01-01");
    d["customFields"]["estimate"] = json!("2h");
    d["customFields"]["hiddenOptional"] = json!("stale-invalid-value");
    d["customFields"]["hiddenRequiredByProfile"] = json!("must-be-sent");
    m.profile
        .required_field_ids
        .push("hiddenRequiredByProfile".into());
    let payload = crate::yunxiao::create_payload(&d, &m, "actual".into()).unwrap();
    assert!(payload["customFieldValues"]["labels"].is_null());
    assert!(payload["customFieldValues"]["customAlias"].is_null());
    assert_eq!(
        payload["customFieldValues"]["planStartTimestamp"],
        "2026-01-01"
    );
    assert!(payload["customFieldValues"]["estimate"].is_null());
    assert!(payload["customFieldValues"]["hiddenOptional"].is_null());
    assert_eq!(
        payload["customFieldValues"]["hiddenRequiredByProfile"],
        "must-be-sent"
    );
    assert!(validation::validate(&d, &m.profile, Some(&m), true, true).is_empty());
    d["customFields"]["hiddenRequiredByProfile"] = json!("");
    assert!(validation::validate(&d, &m.profile, Some(&m), true, true)
        .iter()
        .any(|issue| issue.field == "customFields.hiddenRequiredByProfile"));
}

#[test]
fn removed_required_field_is_rejected_for_validation_and_payload() {
    let mut m = metadata();
    m.fields.push(Field {
        id: "labels".into(),
        name: "标签".into(),
        format: "string".into(),
        field_type: "CustomField".into(),
        required: true,
        show_when_create: true,
        option_error: None,
        options: vec![],
    });
    let d = fixture();
    let errors = validation::validate(&d, &m.profile, Some(&m), true, true);
    assert!(errors
        .iter()
        .any(|e| e.message.contains("已移除字段「标签」设为必填")));
    let error = crate::yunxiao::create_payload(&d, &m, "actual".into()).unwrap_err();
    assert!(error.contains("已移除字段「标签」设为必填"));
}

#[test]
fn removed_estimated_time_required_field_remains_blocking() {
    let mut m = metadata();
    m.fields.push(Field {
        id: "estimatedTime".into(),
        name: "预计工时".into(),
        format: "int".into(),
        field_type: "CustomField".into(),
        required: true,
        show_when_create: true,
        option_error: None,
        options: vec![],
    });
    let d = fixture();
    assert!(validation::validate(&d, &m.profile, Some(&m), true, true)
        .iter()
        .any(|issue| issue.message.contains("已移除字段「预计工时」设为必填")));
    assert!(crate::yunxiao::create_payload(&d, &m, "actual".into())
        .unwrap_err()
        .contains("已移除字段「预计工时」设为必填"));
}

#[test]
fn removed_ids_are_ignored_when_metadata_no_longer_contains_them() {
    let m = metadata();
    let mut d = fixture();
    d["customFields"]["labels"] = json!("stale-invalid-label");
    d["customFields"]["plannedStartTime"] = json!("stale-invalid-time");
    assert!(validation::validate(&d, &m.profile, Some(&m), true, true)
        .iter()
        .all(|issue| !issue.field.starts_with("customFields.labels")
            && !issue.field.starts_with("customFields.plannedStartTime")));
    d["customFields"]["arbitraryUnknown"] = json!("stale-value");
    assert!(validation::validate(&d, &m.profile, Some(&m), true, true)
        .iter()
        .any(|issue| issue.field == "customFields.arbitraryUnknown"));
}

#[test]
fn removed_submission_inputs_are_rejected_by_the_draft_contract() {
    for (field, value) in [
        ("business", json!("旧业务方向")),
        ("manualNote", json!("旧人工补充来源")),
        ("buildUrl", json!("https://ci.example/job/App/42/")),
        ("buildStatus", json!("manual_confirmed")),
        ("buildManifest", json!({})),
    ] {
        let mut d = fixture();
        d[field] = value;
        assert!(validation::check_shape(&d).is_err(), "{field}");
    }
    let mut d = fixture();
    d["app"]["channel"] = json!("internal");
    assert!(validation::check_shape(&d).is_err());
}

#[test]
fn report_omits_all_removed_submission_fields() {
    let mut d = fixture();
    d["business"] = json!("旧业务方向");
    d["manualNote"] = json!("旧人工补充来源");
    d["buildUrl"] = json!("https://ci.example/job/App/42/");
    d["buildStatus"] = json!("manual_confirmed");
    d["buildManifest"] = json!({"legacy":true});
    d["app"]["channel"] = json!("internal");
    let report = evidence::report(&d, &metadata().profile);
    for removed in [
        "方向：",
        "Jenkins 固定构建",
        "人工补充来源",
        "channel",
        "buildUrl",
        "buildStatus",
        "buildManifest",
        "manualNote",
    ] {
        assert!(!report.contains(removed), "{removed}");
    }
}
#[test]
fn every_base_missing_field_is_blocked() {
    let m = metadata();
    for path in ["title", "workitemTypeId", "assignedTo", "actual"] {
        let mut d = fixture();
        let keys: Vec<_> = path.split('.').collect();
        if keys.len() == 2 {
            d[keys[0]][keys[1]] = json!("");
        } else {
            d[path] = json!("");
        }
        assert!(
            !validation::validate(&d, &m.profile, Some(&m), true, true).is_empty(),
            "missing {path}"
        );
    }
}
#[test]
fn optional_reproduction_steps_accept_short_multiline_content() {
    let m = metadata();
    let mut d = fixture();
    d["steps"] = json!("3331111\n444\n555\n666");
    assert!(validation::validate(&d, &m.profile, Some(&m), true, true).is_empty());
    assert!(validation::validate(&d, &m.profile, None, false, false).is_empty());
}
#[test]
fn optional_problem_fields_accept_blank_whitespace_and_placeholder_content() {
    let m = metadata();
    for path in ["jenkinsPackageUrl", "preconditions", "steps", "expected"] {
        for value in ["", " \n\t ", "未知"] {
            let mut d = fixture();
            d[path] = json!(value);
            for online in [true, false] {
                assert!(validation::validate(&d, &m.profile, Some(&m), true, online).is_empty());
            }
        }
    }
}
#[test]
fn required_problem_fields_and_manual_time_submission_gate() {
    let m = metadata();
    for (path, label) in [("title", "问题标题"), ("actual", "问题表现")] {
        let mut d = fixture();
        d[path] = json!(" \n\t ");
        for online in [true, false] {
            let issues = validation::validate(&d, &m.profile, Some(&m), true, online);
            assert!(
                issues
                    .iter()
                    .any(|issue| issue.field == path && issue.message.contains(label)),
                "{path}"
            );
        }
    }
    for time in ["", "2026-10-09T08:00:00", "旧时间记录"] {
        let mut d = fixture();
        d["title"] = json!("闪退");
        d["event"]["occurredAt"] = json!(time);
        d["event"]["source"] = json!("manual");
        d["event"]["sourceNote"] = json!("");
        assert!(validation::check_shape(&d).is_ok());
        for online in [true, false] {
            assert!(validation::validate(&d, &m.profile, Some(&m), true, online).is_empty());
        }
    }
}
#[test]
fn evidence_report_uses_the_problem_form_labels() {
    let report = evidence::report(&fixture(), &metadata().profile);
    for label in ["Jenkins包地址", "复现条件", "期望表现", "问题表现"] {
        assert!(report.contains(&format!("## {label}\n")));
    }
    assert!(report.contains("https://jenkins.example.test/job/android/128/artifact/app.apk"));
    for label in ["前置条件", "预期结果", "实际结果"] {
        assert!(!report.contains(&format!("## {label}\n")));
    }
    assert!(!report.contains("## 采集缺口"));
    assert!(!report.contains("[]"));
    let mut with_gaps = fixture();
    with_gaps["collectionErrors"] = json!(["USB 断开 3 秒", "日志未覆盖问题发生前"]);
    let report = evidence::report(&with_gaps, &metadata().profile);
    assert!(report.contains("## 采集缺口\n\n- USB 断开 3 秒\n- 日志未覆盖问题发生前\n"));
}
#[test]
fn project_package_is_required_while_device_fields_and_page_are_optional() {
    let m = metadata();
    for blank in ["", " ", "\n\t"] {
        let mut d = fixture();
        d["app"]["packageName"] = json!(blank);
        for online in [true, false] {
            let issues = validation::validate(&d, &m.profile, Some(&m), true, online);
            assert_eq!(issues.len(), 1);
            assert_eq!(issues[0].field, "app.packageName");
            assert_eq!(issues[0].message, "请补充当前项目的 Android 包名配置");
        }
    }
    let mut d = fixture();
    d.as_object_mut().unwrap().remove("bugPage");
    for value in d["device"].as_object_mut().unwrap().values_mut() {
        *value = json!("");
    }
    d["app"]["versionName"] = json!("");
    d["app"]["versionCode"] = json!("");
    assert!(validation::check_shape(&d).is_ok());
    assert!(validation::validate(&d, &m.profile, Some(&m), true, true).is_empty());
}
#[test]
fn schema_rejects_injected_keys_and_invalid_hash() {
    let mut d = fixture();
    d["token"] = json!("never-store");
    assert!(validation::check_shape(&d).is_err());
    d.as_object_mut().unwrap().remove("token");
    d["attachments"][0]["sha256"] = json!("fake");
    assert!(validation::check_shape(&d).is_err());
}
#[test]
fn device_parsers_and_identity_validation_are_honest() {
    assert_eq!(
        adb::parse_memory("MemTotal: 100 kB\nMemFree: 20 kB\n"),
        ("100".into(), "".into())
    );
    assert!(adb::valid_serial("serial;rm").is_err());
    assert!(adb::valid_package("com.app;id").is_err());
    assert_eq!(
        adb::parse_screen("Physical size: 1080x2400\nOverride size: 720x1600"),
        "720x1600"
    );
    assert_eq!(
        adb::parse_app(" versionCode=0 minSdk=24\n versionName=1.2"),
        json!({"versionName":"1.2","versionCode":"0","firstInstallTime":"","lastUpdateTime":""})
    );
}
#[test]
fn evidence_hashes_and_managed_merge_are_stable() {
    let (_temp, store, d) = setup();
    let m = metadata();
    let e = evidence::build(&store, &d, &m.profile, false).unwrap();
    let before = evidence::hash_file(Path::new(&e.zip_path)).unwrap().1;
    let source = format!(
        "人工补充：不要覆盖\n{}\n尾部编辑保留",
        evidence::managed(&e, &[], false)
    );
    let merged = evidence::merge_managed(&source, &e, &[], true).unwrap();
    assert!(merged.starts_with("人工补充：不要覆盖"));
    assert!(merged.ends_with("尾部编辑保留"));
    assert!(merged.contains("complete"));
    assert!(!merged.contains("<!-- android-bug-desk:"));
    assert!(merged.contains(&format!("[android-bug-desk-{}-start]:", e.id)));
    let legacy = format!(
        "人工补充\n<!-- android-bug-desk:{}:start -->\n旧证据段\n<!-- android-bug-desk:{}:end -->\n尾部保留",
        e.id, e.id
    );
    let upgraded = evidence::merge_managed(&legacy, &e, &[], true).unwrap();
    assert!(upgraded.starts_with("人工补充"));
    assert!(upgraded.ends_with("尾部保留"));
    assert!(!upgraded.contains("<!-- android-bug-desk:"));
    assert!(upgraded.contains(&evidence::marker(&e.id)));
    assert_eq!(
        before,
        evidence::hash_file(Path::new(&e.zip_path)).unwrap().1
    );
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&e.zip_path).unwrap()).unwrap();
    let mut body = String::new();
    use std::io::Read;
    zip.by_name(&format!("{}/manifest.json", e.id))
        .unwrap()
        .read_to_string(&mut body)
        .unwrap();
    let manifest: Value = serde_json::from_str(&body).unwrap();
    let schema: Value =
        serde_json::from_str(include_str!("../../schemas/manifest.schema.json")).unwrap();
    assert!(jsonschema::is_valid(&schema, &manifest));
    assert_eq!(
        manifest["problem"]["jenkinsPackageUrl"],
        d["jenkinsPackageUrl"]
    );
    assert_eq!(manifest["problem"]["bugPage"], d["bugPage"]);
    let mut report = String::new();
    zip.by_name(&format!("{}/report.md", e.id))
        .unwrap()
        .read_to_string(&mut report)
        .unwrap();
    assert!(report.contains(&string(&d, "bugPage")));
    assert!(manifest.get("cloudConfig").is_none());
    assert!(manifest.get("build").is_none());
    assert!(manifest.get("ciArtifacts").is_none());
    assert!(manifest["scope"].get("businessProfile").is_none());
    assert!(manifest["installedArtifact"].get("fingerprints").is_none());
    assert!(manifest["installedArtifact"].get("status").is_none());
    assert!(manifest["installedArtifact"]["identity"]
        .get("channel")
        .is_none());
    assert!(zip.by_name(&format!("{}/cloud-config.json", e.id)).is_err());
    assert!(!Path::new(&e.directory).join("cloud-config.json").exists());
    assert_eq!(
        manifest["files"][0]["sha256"],
        d["attachments"][0]["sha256"]
    );
    assert!(!body.contains("test-personal-pat"));
}
#[test]
fn corrupt_or_empty_media_cannot_pass() {
    let temp = tempfile::tempdir().unwrap();
    let p = temp.path().join("fake.png");
    std::fs::write(&p, "not a png").unwrap();
    assert!(evidence::inspect(&p, "image", "导入", temp.path()).is_err());
    std::fs::write(&p, []).unwrap();
    assert!(evidence::inspect(&p, "log", "导入", temp.path()).is_err());
}
#[test]
fn boundary_dedup_only_removes_initial_replay() {
    let mut d = adb::BoundaryDeduper::new(b"old-one\nold-two\n");
    assert_eq!(d.push(b"old-t"), b"");
    assert_eq!(d.push(b"wo\nnew-one\nold-two\n"), b"new-one\nold-two\n");
    assert_eq!(d.push(b"old-one\n"), b"old-one\n");
}
#[tokio::test]
async fn all_four_media_types_upload_and_download_exact_content() {
    let (_server, r, api) = mock(Remote::default()).await;
    let (_temp, store, mut d) = setup();
    for (name, role) in [
        ("evidence.png", "image"),
        ("evidence.jpg", "image"),
        ("evidence.mp4", "video"),
    ] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures")
            .join(name);
        let mut a = evidence::inspect(&path, role, "导入模拟媒体", &store.root).unwrap();
        a.confirmed = true;
        d["attachments"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::to_value(a).unwrap());
    }
    store.put("draft", &string(&d, "id"), &d).unwrap();
    let s = Submitter { store }
        .submit(&string(&d, "id"), &api)
        .await
        .unwrap();
    assert_eq!(s.state, "submitted");
    assert_eq!(s.uploads.len(), 5);
    let r = r.lock().unwrap();
    assert_eq!(r.creates, 1);
    assert_eq!(r.files.len(), 5);
    assert!(!r.download_saw_token);
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&s.evidence.zip_path).unwrap()).unwrap();
    for i in 0..zip.len() {
        assert!(!zip.by_index(i).unwrap().name().contains("media/"));
    }
}
#[derive(Default)]
struct Remote {
    creates: usize,
    uploads: usize,
    updates: usize,
    body: Value,
    files: Vec<(String, String, Vec<u8>)>,
    lose_create: bool,
    lose_upload: bool,
    reject_upload: bool,
    expire_upload_auth: bool,
    unsupported_field: bool,
    empty_search: bool,
    download_saw_token: bool,
}
#[derive(Clone)]
struct Simulator {
    remote: Arc<Mutex<Remote>>,
    base: String,
}
impl Respond for Simulator {
    fn respond(&self, r: &Request) -> ResponseTemplate {
        let path = r.url.path();
        let method = r.method.as_str();
        let mut state = self.remote.lock().unwrap();
        if path.starts_with("/download/") {
            state.download_saw_token |= r.headers.contains_key("x-yunxiao-token");
            let id = path.trim_start_matches("/download/");
            let content = state
                .files
                .iter()
                .find(|(fid, _, _)| fid == id)
                .unwrap()
                .2
                .clone();
            return ResponseTemplate::new(200).set_body_bytes(content);
        }
        assert_eq!(
            r.headers.get("x-yunxiao-token").unwrap(),
            "test-personal-pat"
        );
        if path.ends_with("/workitemTypes") {
            return ResponseTemplate::new(200)
                .set_body_json(json!([{"id":"bug-type","name":"缺陷"}]));
        }
        if path.ends_with("/fields") {
            let mut f = json!([{"id":"priority","name":"优先级","type":"SystemCustomField","format":"list","required":true,"options":[{"id":"p2","displayValue":"普通"}]}]);
            if state.unsupported_field {
                f.as_array_mut().unwrap().push(json!({"id":"new-field","name":"新增必填","type":"CustomField","format":"cascading","required":true}));
            }
            return ResponseTemplate::new(200).set_body_json(f);
        }
        if path.ends_with("/members") {
            return ResponseTemplate::new(200)
                .set_body_json(json!([{"userId":"member-1","userName":"同名成员"}]));
        }
        if path.ends_with("/versions") {
            let second = r.url.query_pairs().any(|(k, v)| k == "page" && v == "2");
            return ResponseTemplate::new(200)
                .insert_header("x-total-pages", "2")
                .insert_header("x-next-page", if second { "" } else { "2" })
                .set_body_json(
                    json!([{"id":if second{"version-2"}else{"version-1"},"name":"同名版本"}]),
                );
        }
        if path.ends_with("/sprints") {
            return ResponseTemplate::new(200).set_body_json(json!([]));
        }
        if path.ends_with("/workitems:search") {
            return ResponseTemplate::new(200).set_body_json(
                if state.creates > 0 && !state.empty_search {
                    json!([{"id":"real-id"}])
                } else {
                    json!([])
                },
            );
        }
        if path.ends_with("/workitems") && method == "POST" {
            state.creates += 1;
            state.body = serde_json::from_slice(&r.body).unwrap();
            let desc = format!(
                "{}\n测试人员在云效追加的说明",
                string(&state.body, "description")
            );
            state.body["description"] = json!(desc);
            if state.lose_create {
                state.lose_create = false;
                return ResponseTemplate::new(500);
            }
            return ResponseTemplate::new(200).set_body_json(json!({"id":"real-id"}));
        }
        if path.ends_with("/attachments") && method == "GET" {
            return ResponseTemplate::new(200).set_body_json(state.files.iter().map(|(id,name,bytes)|json!({"id":id,"fileId":format!("file-{id}"),"fileName":name,"size":bytes.len(),"url":format!("{}/download/{id}",self.base)})).collect::<Vec<_>>());
        }
        if path.ends_with("/attachments") && method == "POST" {
            state.uploads += 1;
            if state.expire_upload_auth {
                state.expire_upload_auth = false;
                return ResponseTemplate::new(401);
            }
            if state.reject_upload {
                state.reject_upload = false;
                return ResponseTemplate::new(403);
            }
            let body = &r.body;
            let start = body.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
            let end = body.windows(4).rposition(|w| w == b"\r\n--").unwrap();
            let header = String::from_utf8_lossy(&body[..start]);
            let name = regex::Regex::new(r#"filename="([^"]+)""#)
                .unwrap()
                .captures(&header)
                .unwrap()[1]
                .to_string();
            let id = format!("attachment-{}", state.files.len() + 1);
            state
                .files
                .push((id.clone(), name.clone(), body[start..end].to_vec()));
            if state.lose_upload {
                state.lose_upload = false;
                return ResponseTemplate::new(500);
            }
            return ResponseTemplate::new(200).set_body_json(json!({
                "id":id,
                "embedMarkdown":format!("![{}](https://example.test/embedded/{})", name, id),
                "embedHtml":format!("<img src=\"https://example.test/embedded/{id}\" />"),
                "embedUrl":format!("https://example.test/embedded/{id}")
            }));
        }
        if path.ends_with("/workitems/real-id") && method == "PUT" {
            state.updates += 1;
            let v: Value = serde_json::from_slice(&r.body).unwrap();
            state.body["description"] = v["description"].clone();
            return ResponseTemplate::new(204);
        }
        if path.ends_with("/workitems/real-id") {
            return ResponseTemplate::new(200).set_body_json(json!({"id":"real-id","serialNumber":"TEST-1","space":{"id":"test-project"},"description":state.body["description"],"formatType":"MARKDOWN"}));
        }
        ResponseTemplate::new(404)
    }
}
async fn mock(remote: Remote) -> (MockServer, Arc<Mutex<Remote>>, Api) {
    let server = MockServer::start().await;
    let state = Arc::new(Mutex::new(remote));
    Mock::given(wiremock::matchers::any())
        .respond_with(Simulator {
            remote: state.clone(),
            base: server.uri(),
        })
        .mount(&server)
        .await;
    let api = Api::with_base(metadata().profile, "test-personal-pat", &server.uri()).unwrap();
    (server, state, api)
}
#[tokio::test]
async fn mock_complete_upload_reads_all_pages_and_hashes() {
    let (_server, remote, api) = mock(Remote::default()).await;
    let (_temp, store, d) = setup();
    let m = api.metadata("bug-type").await.unwrap();
    assert_eq!(m.versions.len(), 2);
    let submitter = Submitter {
        store: store.clone(),
    };
    let s = submitter.submit(&string(&d, "id"), &api).await.unwrap();
    assert_eq!(s.state, "submitted");
    assert_eq!(s.identifier.as_deref(), Some("TEST-1"));
    assert_eq!(
        s.url.as_deref(),
        Some("https://devops.aliyun.com/projex/bug/TEST-1")
    );
    let receipt: Value = serde_json::from_slice(
        &std::fs::read(Path::new(&s.evidence.directory).with_extension("receipt.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(receipt["url"], s.url.as_deref().unwrap());
    let mut legacy = s.clone();
    legacy.url = None;
    let text = crate::workitem_links::receipt_text(&legacy, &d, true).unwrap();
    assert!(text.contains("链接：https://devops.aliyun.com/projex/bug/TEST-1"));
    assert!(!text.contains("API 未返回"));
    assert_eq!(
        crate::workitem_links::receipt_text(&legacy, &d, false).unwrap(),
        s.url.as_deref().unwrap()
    );
    let hash = evidence::hash_file(Path::new(&s.evidence.zip_path))
        .unwrap()
        .1;
    let second = submitter.submit(&string(&d, "id"), &api).await.unwrap();
    assert_eq!(second.state, "submitted");
    assert_eq!(hash, s.evidence.sha256);
    let r = remote.lock().unwrap();
    assert_eq!(r.creates, 1);
    assert_eq!(r.files.len(), 2);
    assert!(!r.download_saw_token);
    assert!(string(&r.body, "description").contains("测试人员在云效追加的说明"));
    assert!(string(&r.body, "description")
        .contains(&format!("## Bug所在页面\n\n{}", string(&d, "bugPage"))));
    assert!(!string(&r.body, "description").contains("## 采集缺口"));
    assert!(!string(&r.body, "description").contains("<!-- android-bug-desk:"));
}

#[tokio::test]
async fn markdown_image_is_embedded_in_cloud_description_and_local_evidence() {
    let (_server, remote, api) = mock(Remote::default()).await;
    let (_temp, store, mut draft) = setup();
    let draft_id = string(&draft, "id");
    let asset_id = uuid();
    let directory = store
        .root
        .join("markdown-assets")
        .join(&draft_id)
        .join(&asset_id);
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("image.png");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/evidence.png"),
        &path,
    )
    .unwrap();
    let (size, sha256) = evidence::hash_file(&path).unwrap();
    let asset = MarkdownAsset {
        id: asset_id.clone(),
        field: "actual".into(),
        name: "复现截图.png".into(),
        path: path.to_string_lossy().into_owned(),
        mime: "image/png".into(),
        size,
        sha256,
        width: 1,
        height: 1,
        state: "local".into(),
        remote_id: None,
        embed_markdown: String::new(),
        embed_url: String::new(),
        error: String::new(),
    };
    draft["actual"] = json!(format!(
        "打开页面后崩溃\n\n![复现截图](abd-asset://{asset_id})"
    ));
    draft["markdownAssets"] = json!([asset]);
    store.put("draft", &draft_id, &draft).unwrap();

    let submission = Submitter {
        store: store.clone(),
    }
    .submit(&draft_id, &api)
    .await
    .unwrap();
    assert_eq!(submission.state, "submitted");
    assert_eq!(submission.markdown_assets.len(), 1);
    assert_eq!(submission.markdown_assets[0].state, "uploaded");
    let state = remote.lock().unwrap();
    let description = string(&state.body, "description");
    assert!(description.contains("https://example.test/embedded/attachment-1"));
    assert!(!description.contains("abd-asset://"));
    assert_eq!(state.uploads, 3);
    drop(state);

    let report =
        std::fs::read_to_string(Path::new(&submission.evidence.directory).join("report.md"))
            .unwrap();
    assert!(report.contains("markdown-images/"));
    assert!(!report.contains("abd-asset://"));
    let zip_file = std::fs::File::open(&submission.evidence.zip_path).unwrap();
    let mut archive = zip::ZipArchive::new(zip_file).unwrap();
    assert!((0..archive.len()).any(|index| {
        archive
            .by_index(index)
            .map(|entry| entry.name().contains("/markdown-images/"))
            .unwrap_or(false)
    }));
}
#[tokio::test]
async fn missing_fields_and_new_unsupported_field_make_zero_create_calls() {
    let (_server, r, api) = mock(Remote {
        unsupported_field: true,
        ..Default::default()
    })
    .await;
    let (_temp, store, mut d) = setup();
    let submitter = Submitter {
        store: store.clone(),
    };
    assert!(submitter.submit(&string(&d, "id"), &api).await.is_err());
    d["title"] = json!("");
    store.put("draft", &string(&d, "id"), &d).unwrap();
    assert!(submitter.submit(&string(&d, "id"), &api).await.is_err());
    assert_eq!(r.lock().unwrap().creates, 0);
    assert!(store.list("submission").unwrap().is_empty());
}
#[tokio::test]
async fn lost_create_response_recovers_same_bug_without_repost() {
    let (_server, r, api) = mock(Remote {
        lose_create: true,
        ..Default::default()
    })
    .await;
    let (_temp, store, d) = setup();
    let submitter = Submitter {
        store: store.clone(),
    };
    let id = string(&d, "id");
    let s = submitter.submit(&id, &api).await.unwrap();
    assert_eq!(s.state, "create_unknown");
    assert_eq!(
        submitter.submit(&id, &api).await.unwrap().state,
        "create_unknown"
    );
    store.recover().unwrap();
    let final_state = submitter.reconcile(&id, &api, None).await.unwrap();
    assert_eq!(final_state.state, "submitted");
    assert_eq!(r.lock().unwrap().creates, 1);
}
#[tokio::test]
async fn lost_upload_response_reconciles_content_without_duplicate() {
    let (_server, r, api) = mock(Remote {
        lose_upload: true,
        ..Default::default()
    })
    .await;
    let (_temp, store, d) = setup();
    let submitter = Submitter {
        store: store.clone(),
    };
    let id = string(&d, "id");
    assert_eq!(submitter.submit(&id, &api).await.unwrap().state, "partial");
    store.recover().unwrap();
    assert_eq!(
        submitter.submit(&id, &api).await.unwrap().state,
        "submitted"
    );
    let r = r.lock().unwrap();
    assert_eq!(r.creates, 1);
    assert_eq!(r.uploads, 2);
    assert_eq!(r.files.len(), 2);
}
#[tokio::test]
async fn definite_attachment_rejection_can_be_retried_without_recreate() {
    let (_server, r, api) = mock(Remote {
        reject_upload: true,
        ..Default::default()
    })
    .await;
    let (_temp, store, d) = setup();
    let submitter = Submitter { store };
    let id = string(&d, "id");
    let first = submitter.submit(&id, &api).await.unwrap();
    assert_eq!(first.state, "partial");
    assert_eq!(first.uploads[0].state, "pending");
    assert_eq!(
        submitter.submit(&id, &api).await.unwrap().state,
        "submitted"
    );
    assert_eq!(r.lock().unwrap().creates, 1);
}
#[test]
fn persistent_recovery_keeps_uncertain_intents() {
    let (_temp, store, d) = setup();
    let e = evidence::build(&store, &d, &metadata().profile, false).unwrap();
    let mut s = Submission {
        draft_id: string(&d, "id"),
        profile: metadata().profile,
        state: "creating".into(),
        evidence: e,
        workitem_id: None,
        identifier: None,
        url: None,
        uploads: vec![],
        markdown_assets: vec![],
        error: None,
        intended_at: now(),
        updated_at: now(),
    };
    store.put("submission", &s.draft_id, &s).unwrap();
    store.recover().unwrap();
    s = store.get("submission", &s.draft_id).unwrap().unwrap();
    assert_eq!(s.state, "create_unknown");
    assert!(store.editable(&s.draft_id).is_err());
}

fn submission_state(
    store: &Store,
    server: &MockServer,
    profile: &Profile,
) -> crate::commands::AppState {
    let mut state = crate::commands::AppState::new(store.root.clone()).unwrap();
    state.api_base = server.uri();
    *state.profile.lock().unwrap() = profile.clone();
    *state.token.lock().unwrap() = Some("test-personal-pat".into());
    *state.user.lock().unwrap() = Some(crate::workspace::UserIdentity {
        id: "alice".into(),
        name: "Alice".into(),
        last_organization: profile.organization_id.clone(),
        ..Default::default()
    });
    let package_name = store
        .list("draft")
        .unwrap()
        .first()
        .map(|draft| string(draft, "app.packageName"))
        .filter(|package| !package.is_empty())
        .unwrap_or_else(|| "com.example.test".into());
    crate::project_settings::save_for_state(
        &state,
        &profile.organization_id,
        crate::project_settings::ProjectSettings {
            entries: vec![crate::project_settings::ProjectSetting {
                project_id: profile.project_id.clone(),
                enabled: true,
                package_name,
            }],
        },
    )
    .unwrap();
    state
}
fn bind_alice(store: &Store, id: &str) {
    store
        .put("draft_scope", id, &"alice:test-org:test-project:")
        .unwrap();
}
fn change_identity(state: &crate::commands::AppState, original: &Profile, dimension: &str) {
    let mut profile = original.clone();
    let user = if dimension == "account" {
        "bob"
    } else {
        "alice"
    };
    if dimension == "organization" {
        profile.organization_id = "other-org".into();
    }
    if dimension == "project" {
        profile.project_id = "other-project".into();
    }
    *state.profile.lock().unwrap() = profile;
    state.user.lock().unwrap().as_mut().unwrap().id = user.into();
}
fn write_counts(remote: &Arc<Mutex<Remote>>) -> (usize, usize, usize) {
    let r = remote.lock().unwrap();
    (r.creates, r.uploads, r.updates)
}

#[cfg(unix)]
#[tokio::test]
async fn inspect_device_command_persists_snapshot_and_rejects_unauthorized_before_shell() {
    use std::os::unix::fs::PermissionsExt;
    use tauri::Manager;
    let (server, _remote, api) = mock(Remote::default()).await;
    let (_temp, store, d) = setup();
    let id = string(&d, "id");
    bind_alice(&store, &id);
    let state = submission_state(&store, &server, &api.profile);
    let adb = store.root.join("fake-inspect-adb");
    std::fs::write(&adb, r#"#!/bin/sh
if [ "$1" = devices ]; then printf 'List of devices attached\nUSB-A device product:x\nUSB-B unauthorized\n'; exit 0; fi
if [ "$4" = dumpsys ] && { [ "$5" = activity ] || [ "$5" = window ]; }; then exit 91; fi
if [ "$4" = getprop ] && [ "$5" = ro.product.model ]; then printf 'Pixel Fake\n'; exit 0; fi
if [ "$4" = getprop ] && [ "$5" = ro.build.version.release ]; then printf '14\n'; exit 0; fi
if [ "$4" = cat ]; then printf 'MemTotal: 1234 kB\nMemAvailable: 456 kB\n'; exit 0; fi
if [ "$4" = wm ]; then printf 'Physical size: 1080x2400\nPhysical density: 420\n'; exit 0; fi
if [ "$4" = am ]; then printf '0\n'; exit 0; fi
if [ "$4" = pm ]; then printf 'package:com.example.test\n'; exit 0; fi
if [ "$4" = date ]; then printf '2026-10-09T12:00:00+0800\n'; exit 0; fi
if [ "$4" = dumpsys ] && [ "$6" = com.example.test ]; then printf 'versionName=1.2.3\nversionCode=42\n'; exit 0; fi
exit 0
"#).unwrap();
    std::fs::set_permissions(&adb, std::fs::Permissions::from_mode(0o700)).unwrap();
    *state.adb.lock().unwrap() = Some(adb);
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let snapshot = crate::commands::inspect_device(id.clone(), "USB-A".into(), app.state())
        .await
        .unwrap();
    assert!(snapshot["details"].get("foregroundActivity").is_none());
    assert!(snapshot.get("bugPage").is_none());
    let saved = app
        .state::<crate::commands::AppState>()
        .store
        .draft(&id)
        .unwrap();
    assert_eq!(saved["app"]["packageName"], "com.example.test");
    assert!(crate::validation::check_shape(&saved).is_ok());
    assert!(
        crate::commands::inspect_device(id.clone(), "USB-B".into(), app.state())
            .await
            .unwrap_err()
            .contains("尚未授权")
    );
}
async fn rejected_recovery_matrix(
    server: &MockServer,
    remote: &Arc<Mutex<Remote>>,
    state: &crate::commands::AppState,
    id: &str,
    profile: &Profile,
) {
    let before_requests = server.received_requests().await.unwrap().len();
    let before_counts = write_counts(remote);
    let before = state.store.get::<Value>("submission", id).unwrap();
    for dimension in ["account", "organization", "project"] {
        change_identity(state, profile, dimension);
        assert!(crate::commands::submit_with_state(id.into(), state)
            .await
            .unwrap_err()
            .contains("其他账号"));
        for remote_id in [None, Some("real-id".into())] {
            assert!(
                crate::commands::reconcile_with_state(id.into(), remote_id, state)
                    .await
                    .unwrap_err()
                    .contains("其他账号")
            );
        }
        assert_eq!(
            write_counts(remote),
            before_counts,
            "write on {dimension} mismatch"
        );
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            before_requests,
            "network on {dimension} mismatch"
        );
        assert_eq!(state.store.get::<Value>("submission", id).unwrap(), before);
        assert!(state.locks.lock().unwrap().is_empty());
    }
    change_identity(state, profile, "original");
}

#[tokio::test]
async fn command_new_submission_checks_scope_before_any_network() {
    let (server, remote, api) = mock(Remote::default()).await;
    let (_temp, store, d) = setup();
    let id = string(&d, "id");
    bind_alice(&store, &id);
    let state = submission_state(&store, &server, &api.profile);
    rejected_recovery_matrix(&server, &remote, &state, &id, &api.profile).await;
    let result = crate::commands::submit_with_state(id, &state)
        .await
        .unwrap();
    assert_eq!(result.state, "submitted");
    assert_eq!(write_counts(&remote), (1, 2, 1));
}
#[tokio::test]
async fn command_partial_recovery_requires_original_account_and_keeps_evidence() {
    let (server, remote, api) = mock(Remote {
        reject_upload: true,
        ..Default::default()
    })
    .await;
    let (_temp, store, d) = setup();
    let id = string(&d, "id");
    bind_alice(&store, &id);
    let state = submission_state(&store, &server, &api.profile);
    let partial = crate::commands::submit_with_state(id.clone(), &state)
        .await
        .unwrap();
    assert_eq!(partial.state, "partial");
    rejected_recovery_matrix(&server, &remote, &state, &id, &api.profile).await;
    assert_eq!(
        evidence::hash_file(Path::new(&partial.evidence.zip_path))
            .unwrap()
            .1,
        partial.evidence.sha256
    );
    let resumed = crate::commands::submit_with_state(id, &state)
        .await
        .unwrap();
    assert_eq!(resumed.state, "submitted");
    assert_eq!(resumed.workitem_id, partial.workitem_id);
    assert_eq!(resumed.evidence.sha256, partial.evidence.sha256);
    assert_eq!(write_counts(&remote), (1, 3, 1));
    assert!(Path::new(&resumed.evidence.directory)
        .with_extension("receipt.json")
        .exists());
}

#[tokio::test]
async fn command_auth_expiry_during_partial_recovery_returns_to_login_boundary() {
    let (server, remote, api) = mock(Remote {
        reject_upload: true,
        ..Default::default()
    })
    .await;
    let (_temp, store, d) = setup();
    let id = string(&d, "id");
    bind_alice(&store, &id);
    let state = submission_state(&store, &server, &api.profile);

    let partial = crate::commands::submit_with_state(id.clone(), &state)
        .await
        .unwrap();
    assert_eq!(partial.state, "partial");
    assert!(state.user.lock().unwrap().is_some());
    remote.lock().unwrap().expire_upload_auth = true;

    let error = crate::commands::submit_with_state(id.clone(), &state)
        .await
        .unwrap_err();
    assert!(error.contains("[授权失效]"));
    assert!(state.user.lock().unwrap().is_none());
    assert!(state.metadata.lock().unwrap().is_none());
    assert!(state.locks.lock().unwrap().is_empty());

    let saved: Submission = store.get("submission", &id).unwrap().unwrap();
    assert_eq!(saved.state, "partial");
    assert!(saved
        .error
        .as_deref()
        .is_some_and(|message| message.contains("[授权失效]")));
    assert!(Path::new(&saved.evidence.zip_path).is_file());
}
#[tokio::test]
async fn command_unknown_creation_checks_account_before_search_or_explicit_id() {
    let (server, remote, api) = mock(Remote {
        lose_create: true,
        ..Default::default()
    })
    .await;
    let (_temp, store, d) = setup();
    let id = string(&d, "id");
    bind_alice(&store, &id);
    let state = submission_state(&store, &server, &api.profile);
    let unknown = crate::commands::submit_with_state(id.clone(), &state)
        .await
        .unwrap();
    assert_eq!(unknown.state, "create_unknown");
    rejected_recovery_matrix(&server, &remote, &state, &id, &api.profile).await;
    let resumed = crate::commands::reconcile_with_state(id, None, &state)
        .await
        .unwrap();
    assert_eq!(resumed.state, "submitted");
    assert_eq!(resumed.workitem_id.as_deref(), Some("real-id"));
    assert_eq!(resumed.evidence.sha256, unknown.evidence.sha256);
    assert_eq!(write_counts(&remote), (1, 2, 1));
}
#[tokio::test]
async fn command_empty_candidates_keeps_unknown_without_recreate() {
    let (server, remote, api) = mock(Remote {
        lose_create: true,
        empty_search: true,
        ..Default::default()
    })
    .await;
    let (_temp, store, d) = setup();
    let id = string(&d, "id");
    bind_alice(&store, &id);
    let state = submission_state(&store, &server, &api.profile);
    let unknown = crate::commands::submit_with_state(id.clone(), &state)
        .await
        .unwrap();
    for _ in 0..2 {
        let searched = crate::commands::reconcile_with_state(id.clone(), None, &state)
            .await
            .unwrap();
        assert_eq!(searched.state, "create_unknown");
        assert!(searched.workitem_id.is_none());
        assert_eq!(searched.evidence.sha256, unknown.evidence.sha256);
        let request_count = server.received_requests().await.unwrap().len();
        assert_eq!(
            crate::commands::submit_with_state(id.clone(), &state)
                .await
                .unwrap()
                .state,
            "create_unknown"
        );
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            request_count
        );
        assert_eq!(write_counts(&remote), (1, 0, 0));
    }
    let verified = crate::commands::reconcile_with_state(id, Some("real-id".into()), &state)
        .await
        .unwrap();
    assert_eq!(verified.state, "submitted");
    assert_eq!(write_counts(&remote), (1, 2, 1));
    assert_eq!(
        evidence::hash_file(Path::new(&unknown.evidence.zip_path))
            .unwrap()
            .1,
        unknown.evidence.sha256
    );
}
#[tokio::test]
async fn command_legacy_identity_stays_unknown_without_losing_records() {
    for mode in ["draft", "partial", "create_unknown"] {
        let (server, remote, api) = mock(Remote {
            reject_upload: mode == "partial",
            lose_create: mode == "create_unknown",
            ..Default::default()
        })
        .await;
        let (_temp, store, d) = setup();
        let id = string(&d, "id");
        if mode != "draft" {
            Submitter {
                store: store.clone(),
            }
            .submit(&id, &api)
            .await
            .unwrap();
        }
        let state = submission_state(&store, &server, &api.profile);
        let before = store.get::<Value>("submission", &id).unwrap();
        let counts = write_counts(&remote);
        let requests = server.received_requests().await.unwrap().len();
        let error = crate::commands::submit_with_state(id.clone(), &state)
            .await
            .unwrap_err();
        assert!(error.contains(if mode == "draft" {
            "旧草稿缺少账号绑定"
        } else {
            "身份未知"
        }));
        for remote_id in [None, Some("real-id".into())] {
            assert!(
                crate::commands::reconcile_with_state(id.clone(), remote_id, &state)
                    .await
                    .is_err()
            );
        }
        assert_eq!(store.get::<Value>("submission", &id).unwrap(), before);
        assert_eq!(store.draft(&id).unwrap(), d);
        assert!(store.get::<String>("draft_scope", &id).unwrap().is_none());
        if let Some(row) = before {
            let s: Submission = serde_json::from_value(row).unwrap();
            assert_eq!(
                evidence::hash_file(Path::new(&s.evidence.zip_path))
                    .unwrap()
                    .1,
                s.evidence.sha256
            );
        }
        assert_eq!(server.received_requests().await.unwrap().len(), requests);
        assert_eq!(write_counts(&remote), counts);
        assert!(state.locks.lock().unwrap().is_empty());
    }
}
#[tokio::test]
async fn command_recovery_rejects_inconsistent_saved_project_binding() {
    let (server, remote, api) = mock(Remote {
        reject_upload: true,
        ..Default::default()
    })
    .await;
    let (_temp, store, d) = setup();
    let id = string(&d, "id");
    Submitter {
        store: store.clone(),
    }
    .submit(&id, &api)
    .await
    .unwrap();
    let state = submission_state(&store, &server, &api.profile);
    change_identity(&state, &api.profile, "project");
    store
        .put("draft_scope", &id, &"alice:test-org:other-project:")
        .unwrap();
    let requests = server.received_requests().await.unwrap().len();
    let counts = write_counts(&remote);
    assert!(crate::commands::submit_with_state(id.clone(), &state)
        .await
        .unwrap_err()
        .contains("原提交项目不一致"));
    assert!(crate::commands::reconcile_with_state(id, None, &state)
        .await
        .unwrap_err()
        .contains("原提交项目不一致"));
    assert_eq!(server.received_requests().await.unwrap().len(), requests);
    assert_eq!(write_counts(&remote), counts);
}
#[tokio::test]
async fn command_submission_api_snapshot_does_not_mix_switched_credentials() {
    let (server, remote, api) = mock(Remote::default()).await;
    let (_temp, store, d) = setup();
    let id = string(&d, "id");
    bind_alice(&store, &id);
    let state = submission_state(&store, &server, &api.profile);
    let original_api = crate::workspace::submission_api(&state, &id).unwrap();
    change_identity(&state, &api.profile, "account");
    *state.token.lock().unwrap() = Some("other-personal-pat".into());
    let done = Submitter { store }
        .submit(&id, &original_api)
        .await
        .unwrap();
    assert_eq!(done.state, "submitted");
    assert_eq!(write_counts(&remote), (1, 2, 1));
    assert!(crate::workspace::submission_api(&state, &id).is_err());
}

#[tokio::test]
async fn review_tauri_command_account_scope_matrix() {
    use tauri::Manager;
    for mode in ["new", "partial", "create_unknown"] {
        let (server, remote, api) = mock(Remote {
            reject_upload: mode == "partial",
            lose_create: mode == "create_unknown",
            ..Default::default()
        })
        .await;
        let (_temp, store, d) = setup();
        let id = string(&d, "id");
        bind_alice(&store, &id);
        let app = tauri::test::mock_builder()
            .manage(submission_state(&store, &server, &api.profile))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let initial = if mode != "new" {
            let s = crate::commands::submit_bug(id.clone(), app.state())
                .await
                .unwrap();
            assert_eq!(s.state, mode);
            Some(s)
        } else {
            None
        };
        let before = store.get::<Value>("submission", &id).unwrap();
        let requests = server.received_requests().await.unwrap().len();
        let counts = write_counts(&remote);
        for dimension in ["account", "organization", "project"] {
            change_identity(&app.state(), &api.profile, dimension);
            assert!(crate::commands::submit_bug(id.clone(), app.state())
                .await
                .is_err());
            for remote_id in [None, Some("real-id".into())] {
                assert!(
                    crate::commands::reconcile_submission(id.clone(), remote_id, app.state())
                        .await
                        .is_err()
                );
            }
            assert_eq!(server.received_requests().await.unwrap().len(), requests);
            assert_eq!(write_counts(&remote), counts);
            assert_eq!(store.get::<Value>("submission", &id).unwrap(), before);
            assert!(app
                .state::<crate::commands::AppState>()
                .locks
                .lock()
                .unwrap()
                .is_empty());
            eprintln!("Tauri boundary: mode={mode}, mismatch={dimension}, create/upload/update delta=0, all HTTP delta=0, persistent state unchanged");
        }
        change_identity(&app.state(), &api.profile, "original");
        let done = if mode == "create_unknown" {
            crate::commands::reconcile_submission(id.clone(), None, app.state())
                .await
                .unwrap()
        } else {
            crate::commands::submit_bug(id.clone(), app.state())
                .await
                .unwrap()
        };
        assert_eq!(done.state, "submitted");
        assert_eq!(done.workitem_id.as_deref(), Some("real-id"));
        assert_eq!(remote.lock().unwrap().creates, 1);
        if let Some(s) = initial {
            assert_eq!(s.evidence.sha256, done.evidence.sha256);
            assert_eq!(
                evidence::hash_file(Path::new(&s.evidence.zip_path))
                    .unwrap()
                    .1,
                s.evidence.sha256
            );
        }
        eprintln!(
            "Tauri boundary: mode={mode}, original account resumed real-id, create calls exactly 1"
        );
    }
}
#[tokio::test]
async fn review_tauri_command_legacy_unknown_identity_is_preserved() {
    use tauri::Manager;
    let (server, remote, api) = mock(Remote {
        lose_create: true,
        ..Default::default()
    })
    .await;
    let (_temp, store, d) = setup();
    let id = string(&d, "id");
    let unknown = Submitter {
        store: store.clone(),
    }
    .submit(&id, &api)
    .await
    .unwrap();
    let app = tauri::test::mock_builder()
        .manage(submission_state(&store, &server, &api.profile))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let before = store.get::<Value>("submission", &id).unwrap();
    let requests = server.received_requests().await.unwrap().len();
    let counts = write_counts(&remote);
    assert!(crate::commands::submit_bug(id.clone(), app.state())
        .await
        .unwrap_err()
        .contains("身份未知"));
    assert!(
        crate::commands::reconcile_submission(id.clone(), Some("real-id".into()), app.state())
            .await
            .unwrap_err()
            .contains("身份未知")
    );
    assert_eq!(store.get::<Value>("submission", &id).unwrap(), before);
    assert_eq!(store.draft(&id).unwrap(), d);
    assert!(store.get::<String>("draft_scope", &id).unwrap().is_none());
    assert_eq!(
        evidence::hash_file(Path::new(&unknown.evidence.zip_path))
            .unwrap()
            .1,
        unknown.evidence.sha256
    );
    assert_eq!(server.received_requests().await.unwrap().len(), requests);
    assert_eq!(write_counts(&remote), counts);
    eprintln!("Tauri boundary: legacy identity unknown, all HTTP delta=0, draft/unknown state/ZIP preserved, no silent rebinding");
}

#[tokio::test]
async fn review_account_switch_must_not_resume_another_users_partial_submission() {
    use tauri::Manager;
    let (server, remote, api) = mock(Remote {
        reject_upload: true,
        ..Default::default()
    })
    .await;
    let (temp, store, d) = setup();
    let id = string(&d, "id");
    let s = Submitter {
        store: store.clone(),
    }
    .submit(&id, &api)
    .await
    .unwrap();
    assert_eq!(s.state, "partial");
    store
        .put("draft_scope", &id, &"alice:test-org:test-project:")
        .unwrap();
    let mut state = crate::commands::AppState::new(temp.path().to_path_buf()).unwrap();
    state.api_base = server.uri();
    *state.profile.lock().unwrap() = api.profile.clone();
    *state.token.lock().unwrap() = Some("test-personal-pat".into());
    *state.user.lock().unwrap() = Some(crate::workspace::UserIdentity {
        id: "bob".into(),
        name: "Bob".into(),
        last_organization: "test-org".into(),
        ..Default::default()
    });
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let before = remote.lock().unwrap().uploads;
    let result = crate::commands::submit_bug(id, app.state()).await;
    let after = remote.lock().unwrap().uploads;
    eprintln!("review: switched from Alice to Bob; outcome={:?}; upload_calls_before={before}; upload_calls_after={after}; create_calls={}", result.as_ref().map(|s| s.state.as_str()), remote.lock().unwrap().creates);
    assert!(result.is_err(), "another user resumed a partial submission");
    assert_eq!(
        after, before,
        "account mismatch must make zero upload calls"
    );
}

#[cfg(unix)]
fn integration_fake_adb(root: &Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = root.join("fake-adb.py");
    let png = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/evidence.png");
    let body = r#"#!/usr/bin/env python3
import sys,time,os,zipfile
from pathlib import Path
args=sys.argv[1:]
with open(Path(__file__).with_suffix('.argv'),'a') as f: f.write(repr(args)+'\n')
if args[:1]==['-s']:
 assert args[1]=='integration-device'
 args=args[2:]
if args[:1]==['devices']: print('List of devices attached\nintegration-device device model:Mock');sys.exit(0)
if args[:1]==['logcat']:
 if 'crash' in args and '--uid=10123' not in args:
  print('10-16 19:00:00.000 999 999 F DEBUG: *** *** *** ***',flush=True)
  print('10-16 19:00:00.001 999 999 F DEBUG: Cmdline: com.example.test',flush=True)
  print('10-16 19:00:00.002 777 777 F DEBUG: target native crash',flush=True)
  if '-d' in args: sys.exit(0)
  while True: time.sleep(.1)
 assert '--uid=10123' in args,args
 if '-d' in args:
  for offset in [-125,-120,-119]: print(f'{time.time()+offset:.6f} 11 11 I Mock: secret@example.test before')
 else:
  while True:
   print(f'{time.time():.6f} 22 22 E OtherProcess: secret@example.test live',flush=True);time.sleep(.1)
 sys.exit(0)
if args[:1]==['exec-out']: sys.stdout.buffer.write(Path(PNG).read_bytes());sys.exit(0)
if args[:1]==['bugreport']:
 if Path(__file__).with_suffix('.fail').exists():
  Path(args[1],'partial.txt').write_text('partial mock report');sys.exit(7)
 with zipfile.ZipFile(Path(args[1])/'bugreport.zip','w') as z: z.writestr('report.txt','mock system report')
 sys.exit(0)
if args[:2]==['shell','date']:
 print(int(time.time()) if args[-1]=='+%s' else '2026-10-09T08:00:00+0800');sys.exit(0)
if args[:3]==['shell','am','get-current-user']: print('0');sys.exit(0)
if args[:3]==['shell','pm','list']: print('package:com.example.test');sys.exit(0)
if args[:3]==['shell','dumpsys','package']: print('Package [com.example.test]\n  userId=10123\nversionName=1.0\nversionCode=0 minSdk=24');sys.exit(0)
if args[:2]==['shell','getprop']: print('14' if args[-1]=='ro.build.version.release' else 'mock-model');sys.exit(0)
if args[:2]==['shell','cat']: print('MemTotal: 8192000 kB\nMemAvailable: 4096000 kB');sys.exit(0)
if args[:3]==['shell','wm','size']: print('Physical size: 1080x2400');sys.exit(0)
if args[:3]==['shell','wm','density']: print('Physical density: 420');sys.exit(0)
print('unknown fake command',file=sys.stderr);sys.exit(1)
"#;
    let body = body.replace(
        "Path(PNG)",
        &format!(
            "Path({})",
            serde_json::to_string(&png.to_string_lossy()).unwrap()
        ),
    );
    std::fs::write(&path, body).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}

#[cfg(unix)]
fn legacy_stream_fake_adb(root: &Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = root.join("legacy-stream-adb.py");
    std::fs::write(
        &path,
        r#"#!/usr/bin/env python3
import sys,time
args=sys.argv[1:]
if args[:1]==['-s']:
 assert args[1]=='legacy-device'
 args=args[2:]
if args[:3]==['shell','pm','list']:
 print('package:com.example.test uid:10123');sys.exit(0)
if args[:3]==['shell','dumpsys','package']:
 print('Package [com.example.test]\n userId=10123');sys.exit(0)
if args[:2]==['shell','am']:
 print('0');sys.exit(0)
if args[:2]==['shell','pidof']:
 print('321');sys.exit(0)
if args[:2]==['shell','ps']:
 print('USER PID PPID NAME\nu0_a123 321 1 com.example.test');sys.exit(0)
if args[:1]==['logcat'] and any(x.startswith('--uid=') for x in args):
 print('Unrecognized Option',file=sys.stderr);sys.exit(1)
if args[:1]==['logcat'] and '-d' in args and 'crash' in args:
 print('10-16 19:00:00.000 999 999 F DEBUG: *** *** *** ***')
 print('10-16 19:00:00.001 999 999 F DEBUG: Cmdline: com.other.private')
 print('10-16 19:01:00.000 888 888 F DEBUG: *** *** *** ***')
 print('10-16 19:01:00.001 888 888 F DEBUG: Cmdline: com.example.test')
 print('10-16 19:01:00.002 777 777 F DEBUG: target crash')
 sys.exit(0)
if args[:1]==['logcat'] and '-d' in args:
 print('1789102800.000 999 999 I OTHER: private existing')
 print('1789102801.000 321 322 E APP: target existing')
 sys.exit(0)
if args[:1]==['logcat'] and 'crash' in args:
 print('10-16 19:02:00.000 999 999 F DEBUG: *** *** *** ***',flush=True)
 print('10-16 19:02:00.001 999 999 F DEBUG: Cmdline: com.other.private',flush=True)
 print('10-16 19:03:00.000 888 888 F DEBUG: *** *** *** ***',flush=True)
 print('10-16 19:03:00.001 888 888 F DEBUG: Cmdline: com.example.test',flush=True)
 print('10-16 19:03:00.002 777 777 F DEBUG: target live crash',flush=True)
 while True: time.sleep(.1)
if args[:1]==['logcat']:
 while True:
  print('1789102802.000 999 999 I OTHER: private live',flush=True)
  print('1789102803.000 321 322 E APP: target live',flush=True)
  time.sleep(.05)
print('unknown',args,file=sys.stderr);sys.exit(1)
"#,
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}

#[cfg(unix)]
#[tokio::test]
async fn legacy_realtime_logs_filter_before_disk_and_keep_target_crash_records() {
    let (temp, store, _) = setup();
    let adb = legacy_stream_fake_adb(temp.path());
    let session = json!({
        "id":"legacy-session",
        "serial":"legacy-device",
        "packageName":"com.example.test",
        "startedAt":now(),
        "stoppedAt":null,
        "snapshot":{},
        "logFiles":[],
        "gaps":[]
    });
    let collector = crate::adb::start_logs(adb, "legacy-device".into(), store.clone(), session)
        .await
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let live_crash_persisted = store
            .get::<Value>("session", "legacy-session")
            .ok()
            .flatten()
            .and_then(|stored| stored["logFiles"].as_array().cloned())
            .is_some_and(|paths| {
                paths.iter().any(|path| {
                    path.as_str()
                        .and_then(|path| std::fs::read_to_string(path).ok())
                        .is_some_and(|contents| contents.contains("target live crash"))
                })
            });
        if live_crash_persisted || std::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    crate::adb::stop_logs(collector).await;

    let stored: Value = store.get("session", "legacy-session").unwrap().unwrap();
    assert_eq!(stored["logFilter"]["mode"], "localTargetIdentity");
    assert_eq!(stored["logFilter"]["rawDeviceLogPersisted"], false);
    let mut combined = String::new();
    for path in stored["logFiles"].as_array().unwrap() {
        combined.push_str(&std::fs::read_to_string(path.as_str().unwrap()).unwrap());
    }
    assert!(combined.contains("target existing"));
    assert!(combined.contains("target live"));
    assert!(combined.contains("target crash"));
    assert!(combined.contains("target live crash"));
    assert!(!combined.contains("private existing"));
    assert!(!combined.contains("private live"));
    assert!(!combined.contains("com.other.private"));
}
#[cfg(unix)]
#[tokio::test]
async fn integrated_tauri_auto_window_redaction_and_all_media_submit() {
    use tauri::Manager;
    let (server, remote, api) = mock(Remote::default()).await;
    let (_temp, store, d) = setup();
    let id = string(&d, "id");
    bind_alice(&store, &id);
    let state = submission_state(&store, &server, &api.profile);
    *state.adb.lock().unwrap() = Some(integration_fake_adb(&store.root));
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    crate::commands::start_session("integration-device".into(), app.state())
        .await
        .unwrap();
    let marked = crate::commands::mark_problem(id.clone(), app.state())
        .await
        .unwrap();
    assert!(marked["event"].get("lateCapture").is_none());
    let problem_screenshot = marked["attachments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|attachment| string(attachment, "mime") == "image/png")
        .unwrap();
    assert!(string(problem_screenshot, "source").contains("原始分辨率"));
    assert!(string(problem_screenshot, "source").contains("PNG 无损压缩"));
    validation::check_shape(&marked).unwrap();
    assert!(crate::capture::pending(&store, &id).unwrap());
    assert!(crate::commands::submit_bug(id.clone(), app.state())
        .await
        .unwrap_err()
        .contains("采集"));
    assert_eq!(write_counts(&remote), (0, 0, 0));
    let mut edited = marked;
    edited["title"] = json!("计时期间的人工补充不能丢失");
    crate::commands::save_draft(edited, app.state()).unwrap();
    let report = crate::commands::capture_bugreport(id.clone(), app.state())
        .await
        .unwrap();
    assert!(report["attachments"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| string(a, "mime") == "application/zip"));
    tokio::time::timeout(std::time::Duration::from_secs(38), async {
        while crate::capture::pending(&store, &id).unwrap() {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    let task = store
        .list("capture")
        .unwrap()
        .into_iter()
        .find(|t| string(t, "kind") == "log-window")
        .unwrap();
    assert_eq!(task["status"], "complete");
    assert!(task["summary"]["selectedRecords"].as_u64().unwrap() > 0);
    // Initial-buffer continuity is uncertain; elapsed time alone must not claim complete coverage.
    assert_eq!(task["summary"]["rangeCovered"], false);
    let completed = store.draft(&id).unwrap();
    let generated_logs = attachments(&completed)
        .unwrap()
        .into_iter()
        .filter(|attachment| attachment.source.starts_with("ADB 原始日志固定副本"))
        .collect::<Vec<_>>();
    assert!(!generated_logs.is_empty());
    assert!(generated_logs.iter().all(|attachment| {
        !attachment.coverage_start.is_empty()
            && !attachment.coverage_end.is_empty()
            && attachment.source.contains("自动填写覆盖时间")
    }));
    assert_eq!(completed["title"], "计时期间的人工补充不能丢失");
    let wid = string(&task, "windowAttachmentId");
    let original = attachments(&store.draft(&id).unwrap())
        .unwrap()
        .into_iter()
        .find(|a| a.id == wid)
        .unwrap();
    let before = evidence::hash_file(Path::new(&original.path)).unwrap();
    let redacted = crate::commands::redact_log(
        id.clone(),
        wid.clone(),
        "secret@example.test".into(),
        app.state(),
    )
    .unwrap();
    assert_eq!(
        evidence::hash_file(Path::new(&original.path)).unwrap(),
        before
    );
    let derived = attachments(&redacted)
        .unwrap()
        .into_iter()
        .find(|a| a.role == "log" && a.derived_from.as_deref() == Some(&wid))
        .unwrap();
    assert!(!std::fs::read_to_string(&derived.path)
        .unwrap()
        .contains("secret@example.test"));
    let confirmed = crate::commands::confirm_attachment(
        id.clone(),
        derived.id.clone(),
        true,
        derived.coverage_start.clone(),
        derived.coverage_end.clone(),
        app.state(),
    )
    .unwrap();
    let rows = attachments(&confirmed).unwrap();
    assert_eq!(
        rows.iter()
            .filter(|a| a.role == "report"
                && a.derived_from.as_deref() == Some(&derived.id)
                && a.confirmed)
            .count(),
        2,
        "redaction and inherited window receipts travel with selected derivative"
    );
    assert!(rows.iter().all(|a| a.id != wid));
    for receipt in rows
        .iter()
        .filter(|a| a.role == "report" && a.derived_from.as_deref() == Some(&derived.id))
    {
        assert!(
            crate::commands::remove_attachment(id.clone(), receipt.id.clone(), app.state())
                .unwrap_err()
                .contains("关联副本")
        );
        assert_eq!(
            store.draft(&id).unwrap()["attachments"],
            confirmed["attachments"]
        );
    }
    {
        let mut broken = confirmed.clone();
        let receipt_id = &rows
            .iter()
            .find(|a| a.role == "report" && a.derived_from.as_deref() == Some(&derived.id))
            .unwrap()
            .id;
        broken["attachments"]
            .as_array_mut()
            .unwrap()
            .retain(|a| string(a, "id") != *receipt_id);
        assert!(evidence::build(&store, &broken, &api.profile, false).is_err());
        store.put("draft", &id, &broken).unwrap();
        assert!(crate::commands::submit_bug(id.clone(), app.state())
            .await
            .is_err());
        assert_eq!(write_counts(&remote), (0, 0, 0));
    }
    store.put("draft", &id, &confirmed).unwrap();
    let exported = evidence::build(&store, &confirmed, &api.profile, true).unwrap();
    assert_eq!(
        exported
            .files
            .iter()
            .filter(|a| a.role == "report" && a.derived_from.as_deref() == Some(&derived.id))
            .count(),
        2
    );
    let removed =
        crate::commands::remove_attachment(id.clone(), derived.id.clone(), app.state()).unwrap();
    assert!(!attachments(&removed)
        .unwrap()
        .iter()
        .any(|a| a.id == derived.id || a.derived_from.as_deref() == Some(&derived.id)));
    assert_eq!(
        evidence::hash_file(Path::new(&original.path)).unwrap(),
        before
    );
    store.put("draft", &id, &confirmed).unwrap();
    let mut final_d = confirmed;
    for (name, role) in [
        ("evidence.png", "image"),
        ("evidence.jpg", "image"),
        ("evidence.mp4", "video"),
    ] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures")
            .join(name);
        let mut a = evidence::inspect(&path, role, "模拟媒体", &store.root).unwrap();
        a.confirmed = true;
        final_d["attachments"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::to_value(a).unwrap());
    }
    for row in final_d["attachments"].as_array_mut().unwrap() {
        if string(row, "source") == "ADB 按需系统报告；未脱敏" {
            row["confirmed"] = json!(true);
        }
    }
    final_d["previewConfirmed"] = json!(true);
    // Every listed attachment uploads without a per-file confirmation gate.
    for row in final_d["attachments"].as_array_mut().unwrap() {
        if string(row, "role") == "log" && string(row, "coverageStart").is_empty() {
            row["coverageStart"] = d["attachments"][0]["coverageStart"].clone();
            row["coverageEnd"] = d["attachments"][0]["coverageEnd"].clone();
        }
    }
    let expected_uploads = attachments(&final_d).unwrap().len() + 1;
    validation::check_shape(&final_d).unwrap();
    store.put("draft", &id, &final_d).unwrap();
    let (first, second) = tokio::join!(
        crate::commands::submit_bug(id.clone(), app.state()),
        crate::commands::submit_bug(id.clone(), app.state())
    );
    let (submitted, duplicate) = match (first, second) {
        (Ok(submitted), Err(error)) | (Err(error), Ok(submitted)) => (submitted, error),
        other => panic!("concurrent submit must admit exactly one call: {other:?}"),
    };
    assert!(duplicate.contains("请勿重复点击"));
    assert_eq!(submitted.state, "submitted");
    assert_eq!(
        submitted.uploads.len(),
        expected_uploads,
        "bundle + every draft attachment, regardless of the legacy confirmation flag"
    );
    assert_eq!(remote.lock().unwrap().creates, 1);
    assert!(!remote.lock().unwrap().download_saw_token);
    let again = crate::commands::submit_bug(id.clone(), app.state())
        .await
        .unwrap();
    assert_eq!(again.evidence.sha256, submitted.evidence.sha256);
    assert_eq!(remote.lock().unwrap().creates, 1);
    assert!(
        crate::commands::redact_log(id.clone(), derived.id, "secret".into(), app.state()).is_err()
    );
    crate::commands::stop_session(app.state()).await.unwrap();
}
#[test]
fn integrated_restart_keeps_last_observation_and_reports_missing_window() {
    let (_temp, store, d) = setup();
    let id = string(&d, "id");
    let sid = uuid();
    let marked = "2026-10-09T08:00:00+08:00";
    let window = adb::ProblemWindow::new(marked).unwrap();
    store.put("session",&sid,&json!({"id":sid,"startedAt":marked,"stoppedAt":null,"logFiles":[d["attachments"][0]["path"]],"gaps":[]})).unwrap();
    let taskid = uuid();
    store.put("capture",&taskid,&json!({"id":taskid,"draftId":id,"sessionId":sid,"kind":"log-window","status":"pending","window":window,"observedUntil":marked,"offsetMicros":null,"clockNote":"时钟未知"})).unwrap();
    let state = crate::commands::AppState::new(store.root.clone()).unwrap();
    let task: Value = state.store.get("capture", &taskid).unwrap().unwrap();
    assert_eq!(task["status"], "interrupted");
    assert_eq!(task["observedUntil"], marked);
    let _write = state.store.writes.lock().unwrap();
    crate::capture::retry_interrupted(&state.store, &id).unwrap();
    let task: Value = state.store.get("capture", &taskid).unwrap().unwrap();
    assert_eq!(task["summary"]["observedUntil"], marked);
    assert_eq!(task["summary"]["rangeCovered"], false);
    let count = state.store.draft(&id).unwrap()["attachments"]
        .as_array()
        .unwrap()
        .len();
    crate::capture::retry_interrupted(&state.store, &id).unwrap();
    assert_eq!(
        state.store.draft(&id).unwrap()["attachments"]
            .as_array()
            .unwrap()
            .len(),
        count
    );
}

#[cfg(unix)]
#[tokio::test]
async fn integrated_early_stop_interrupts_window_and_never_pads_time() {
    use tauri::Manager;
    let (server, remote, api) = mock(Remote::default()).await;
    let (_temp, store, d) = setup();
    let id = string(&d, "id");
    bind_alice(&store, &id);
    let state = submission_state(&store, &server, &api.profile);
    *state.adb.lock().unwrap() = Some(integration_fake_adb(&store.root));
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    crate::commands::start_session("integration-device".into(), app.state())
        .await
        .unwrap();
    crate::commands::mark_problem(id.clone(), app.state())
        .await
        .unwrap();
    assert!(crate::commands::mark_problem(id.clone(), app.state())
        .await
        .is_err());
    crate::commands::stop_session(app.state()).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(4), async {
        while crate::capture::pending(&store, &id).unwrap() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let task = store.list("capture").unwrap().pop().unwrap();
    assert_eq!(task["status"], "interrupted");
    assert_eq!(task["summary"]["rangeCovered"], false);
    let observed =
        chrono::DateTime::parse_from_rfc3339(&string(&task, "summary.observedUntil")).unwrap();
    let end = chrono::DateTime::parse_from_rfc3339(&string(&task, "summary.requestedEnd")).unwrap();
    assert!(observed < end);
    assert_eq!(write_counts(&remote), (0, 0, 0));
}

#[cfg(unix)]
#[tokio::test]
async fn integrated_bugreport_failure_records_gap_and_preserves_device_binding() {
    use tauri::Manager;
    let (server, remote, api) = mock(Remote::default()).await;
    let (_temp, store, mut d) = setup();
    let id = string(&d, "id");
    bind_alice(&store, &id);
    let sid = uuid();
    d["sessionId"] = json!(sid);
    store.put("draft", &id, &d).unwrap();
    store
        .put(
            "session",
            &sid,
            &json!({"id":sid,"serial":"integration-device","stoppedAt":now(),"gaps":[]}),
        )
        .unwrap();
    let state = submission_state(&store, &server, &api.profile);
    let fake = integration_fake_adb(&store.root);
    std::fs::write(fake.with_extension("fail"), "simulate disconnect").unwrap();
    *state.adb.lock().unwrap() = Some(fake.clone());
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let failed = crate::commands::capture_bugreport(id.clone(), app.state())
        .await
        .unwrap();
    assert_eq!(failed["attachments"], d["attachments"]);
    assert!(!failed["collectionErrors"].as_array().unwrap().is_empty());
    assert!(!crate::capture::pending(&store, &id).unwrap());
    assert_eq!(store.list("capture").unwrap()[0]["status"], "failed");
    let trace = std::fs::read_to_string(fake.with_extension("argv")).unwrap();
    assert!(trace.contains("'-s', 'integration-device', 'bugreport'"));
    change_identity(&app.state(), &api.profile, "account");
    assert!(crate::commands::capture_bugreport(id, app.state())
        .await
        .is_err());
    assert_eq!(
        std::fs::read_to_string(fake.with_extension("argv")).unwrap(),
        trace
    );
    assert_eq!(write_counts(&remote), (0, 0, 0));
}
