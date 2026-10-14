//! Fake ADB exercises the same jobs used by the desktop commands.
#![cfg(unix)]
use super::*;
use std::{io::Read, path::PathBuf};
use tauri::Manager;

fn fake_adb(root: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = root.join("post-adb.py");
    image::RgbImage::new(1, 1)
        .save(root.join("screen.png"))
        .unwrap();
    std::fs::write(&path, r#"#!/usr/bin/env python3
import sys,json,time,zipfile,base64
from pathlib import Path
root=Path(__file__).parent
a=sys.argv[1:]
mode=(root/'mode').read_text()
with (root/'calls.jsonl').open('a') as f:f.write(json.dumps(a)+'\n')
if a[0]=='devices':
 state=mode if mode in ['offline','unauthorized'] else 'device'
 print('List of devices attached\nUSB-A '+state+' model:Fake')
 sys.exit(0)
assert a[:2]==['-s','USB-A'],a
a=a[2:]
if a[:2]==['exec-out','screencap']:
 if mode=='paused':
  (root/'ready').touch()
  while not (root/'release').exists():time.sleep(.01)
 if mode=='bad-screen':sys.stdout.buffer.write(b'invalid');sys.exit(0)
 sys.stdout.buffer.write((root/'screen.png').read_bytes())
elif a[0]=='bugreport':
 if mode=='bad-report':print('failed',file=sys.stderr);sys.exit(1)
 with zipfile.ZipFile(Path(a[1])/'bugreport-fake.zip','w') as z:z.writestr('bugreport.txt','Bugreport for fake Android')
elif a[0]=='logcat':
 if mode in ['uid-unsupported','legacy-logcat'] and '--uid=10123' in a:
  print('Unrecognized Option',file=sys.stderr)
  print('Usage: logcat [options] [filterspecs]',file=sys.stderr)
  sys.exit(1)
 if mode=='legacy-logcat' and '--pid=321' in a:
  print('Unrecognized Option',file=sys.stderr)
  print('Usage: logcat [options] [filterspecs]',file=sys.stderr)
  sys.exit(1)
 if mode=='legacy-logcat':
  assert '--uid=10123' not in a and '--pid=321' not in a,a
  assert 'threadtime' in a,a
  print('10-13 15:00:00.000   999   999 I OTHER: private other-app line')
  print('10-13 15:00:01.000   321   322 E APP: target failure')
  print('    at com.example.test.Main.run(Main.java:10)')
  print('10-13 15:00:02.000   999   999 I OTHER: another private line')
  sys.exit(0)
 assert '--uid=10123' in a or '--pid=321' in a,a
 if mode=='empty':print('--------- beginning of main')
 else:print('1789102800.000 123 456 E APP: Permission denied is app log content\n1789102900.010 123 456 E APP: java.lang.RuntimeException: test crash')
elif a[:2]==['shell','pidof']:
 assert a[2:]==['com.example.test'],a
 if mode=='legacy-logcat':
  print('pidof: not found',file=sys.stderr)
  sys.exit(127)
 print('321')
elif a[:2]==['shell','ps']:
 print('USER PID PPID VSIZE RSS WCHAN PC NAME')
 print('u0_a123 321 1 1000 100 0 0 com.example.test')
 print('u0_a124 999 1 1000 100 0 0 com.other.private')
elif a[:2]==['shell','uiautomator']:print('UI hierarchy dumped to: '+a[-1])
elif a[:2]==['exec-out','cat']:print('<?xml version="1.0"?><hierarchy><node text="test"/></hierarchy>')
elif a[:2]==['shell','rm']:assert a[2]=='-f' and a[3].startswith('/sdcard/AndroidBugDesk-')
elif a[:3]==['shell','dumpsys','meminfo']:
 print('Permission Denial: no diagnostic permission' if mode=='partial' else '** MEMINFO in pid 123 [com.example.test] **')
elif a[:4]==['shell','dumpsys','activity','exit-info']:
 print('ACTIVITY MANAGER PROCESS EXIT INFO\n' if mode=='empty' else 'ApplicationExitInfo #0:\n reason=CRASH')
elif a[:4]==['shell','pm','list','packages']:
 assert a[4:]==['-U','--user','0','com.example.test'],a
 print('package:com.example.test uid:10123')
elif a[:3]==['shell','dumpsys','package']:print('Packages:\nPackage [com.example.test]\n  userId=10123\nversionCode=42 versionName=2.1\n grantedPermissions: android.permission.INTERNET')
elif a[:3]==['shell','am','get-current-user']:print('0')
elif a[:2]==['shell','getprop']:print('Fake Android 13')
elif a[:2]==['shell','date']:print('1789103000')
elif a[:2]==['shell','wm']:print('Physical '+a[2]+': 1080x2400')
elif a[:3]==['shell','dumpsys','gfxinfo']:print('Graphics info\n---PROFILEDATA---\nFlags,IntendedVsync,Vsync\n0,123,124\n---PROFILEDATA---')
elif a[:3]==['shell','cmd','locale']:print('Locales for com.example.test for user 0 are [en-US]')
elif a[:2]==['shell','df']:print('Filesystem 1K-blocks Used Available\n/data 10000 5000 5000')
elif a[0]=='shell':print('diagnostic snapshot; mResumedActivity=com.android.launcher/.Home')
else:raise AssertionError(a)
"#).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(root.join("mode"), "normal").unwrap();
    path
}
fn app(root: &Path, adb: PathBuf) -> (tauri::App<tauri::test::MockRuntime>, Value) {
    let state = AppState::new(root.into()).unwrap();
    let metadata: Metadata =
        serde_json::from_str(include_str!("../../tests/fixtures/metadata.json")).unwrap();
    let mut draft: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/valid-draft.json")).unwrap();
    let dir = evidence::derived_directory(root, "attachments").unwrap();
    let original = dir.join("existing.txt");
    std::fs::write(&original, "original manual attachment").unwrap();
    draft["attachments"] =
        json!([evidence::inspect(&original, "sample", "原有附件", root).unwrap()]);
    draft["collectionErrors"] = json!(["原有缺口"]);
    *state.profile.lock().unwrap() = metadata.profile;
    *state.user.lock().unwrap() = Some(crate::workspace::UserIdentity {
        id: "alice".into(),
        ..Default::default()
    });
    crate::project_settings::save_for_state(
        &state,
        "test-org",
        crate::project_settings::ProjectSettings {
            entries: vec![crate::project_settings::ProjectSetting {
                project_id: "test-project".into(),
                enabled: true,
                package_name: "com.example.test".into(),
            }],
        },
    )
    .unwrap();
    crate::project_settings::apply_current_package(&state, &mut draft).unwrap();
    *state.adb.lock().unwrap() = Some(adb);
    let id = string(&draft, "id");
    state.store.put("draft", &id, &draft).unwrap();
    state
        .store
        .put("draft_scope", &id, &"alice:test-org:test-project:")
        .unwrap();
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    (app, draft)
}
fn selected(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}
fn trace(root: &Path) -> Vec<Vec<String>> {
    std::fs::read_to_string(root.join("calls.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
fn receipt(a: &Attachment) -> Value {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&a.path).unwrap()).unwrap();
    let mut bytes = Vec::new();
    zip.by_name("receipt.json")
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
#[tokio::test]
async fn all_items_keep_original_identity_time_and_manual_inputs_with_complete_receipts() {
    let temp = tempfile::tempdir().unwrap();
    let (app, original) = app(temp.path(), fake_adb(temp.path()));
    let state = app.state::<AppState>();
    let id = string(&original, "id");
    let job = prepare(&state, &id, "USB-A", selected(ITEMS), None)
        .await
        .unwrap();
    assert!(crate::capture::pending(&state.store, &id).unwrap());
    execute(&state, &string(&job, "id")).await.unwrap();
    let snapshot = post_capture_status(app.state(), id.clone()).unwrap();
    assert_eq!(snapshot["jobs"][0]["status"], "complete");
    let screenshot_result = &snapshot["jobs"][0]["items"]["screenshot"];
    assert_eq!(screenshot_result["compression"]["format"], "PNG");
    assert_eq!(screenshot_result["compression"]["mode"], "lossless");
    assert_eq!(screenshot_result["compression"]["resolutionChanged"], false);
    assert_eq!(screenshot_result["compression"]["width"], 1);
    assert_eq!(screenshot_result["compression"]["height"], 1);
    assert!(
        screenshot_result["compression"]["storedBytes"]
            .as_u64()
            .unwrap()
            <= screenshot_result["compression"]["originalBytes"]
                .as_u64()
                .unwrap()
    );
    let draft = state.store.draft(&id).unwrap();
    for key in [
        "event",
        "sessionId",
        "title",
        "kind",
        "app",
        "device",
        "bugPage",
        "sources",
        "collectionErrors",
        "createdAt",
    ] {
        assert_eq!(draft[key], original[key], "{key}");
    }
    assert_eq!(draft["attachments"][0], original["attachments"][0]);
    assert_eq!(draft["previewConfirmed"], false);
    crate::validation::check_shape(&draft).unwrap();
    let files = attachments(&draft).unwrap();
    assert_eq!(files.len(), ITEMS.len() + 2);
    for file in &files {
        evidence::verify_attachment(file).unwrap();
    }
    for (item, label) in [
        ("environment", "设备与应用环境"),
        ("logs", "现存日志"),
        ("screenshot", "当前截图"),
        ("ui", "当前界面层级"),
        ("app", "应用与进程状态"),
        ("display", "窗口与显示状态"),
        ("locale", "语言与字体配置"),
        ("crash", "崩溃日志"),
        ("exit", "历史进程退出记录"),
        ("memory", "当前应用内存"),
        ("frames", "现存帧统计"),
        ("cpu", "CPU与进程状态"),
        ("network", "网络配置与统计"),
        ("storage", "存储空间与权限"),
        ("bugreport", "系统报告"),
    ] {
        let attachment = match item {
            "environment" => files
                .iter()
                .find(|attachment| attachment.name.starts_with("device-environment-snapshot-"))
                .unwrap(),
            "screenshot" => files
                .iter()
                .find(|attachment| attachment.name == "post-capture-current-screen.png")
                .unwrap(),
            "bugreport" => files
                .iter()
                .find(|attachment| attachment.name == "bugreport-fake.zip")
                .unwrap(),
            _ => files
                .iter()
                .find(|attachment| attachment.name == format!("post-capture-{item}.zip"))
                .unwrap(),
        };
        assert!(
            attachment.source.contains(&format!("采集项：{label}")),
            "{} should identify its capture item in the attachment source",
            attachment.name
        );
        if item == "screenshot" {
            assert!(attachment.source.contains("原始分辨率"));
            assert!(attachment.source.contains("PNG 无损压缩"));
        }
    }
    let summary = files
        .iter()
        .find(|attachment| attachment.name == "post-capture-summary.zip")
        .unwrap();
    assert!(summary.source.contains("包含采集项："));
    assert!(summary.source.contains("现存日志"));
    assert!(summary.source.contains("当前界面层级"));
    let logs = files
        .iter()
        .find(|a| a.name == "post-capture-logs.zip")
        .unwrap();
    let log = receipt(logs);
    assert_eq!(log["targetPackage"], "com.example.test");
    assert_eq!(log["logFilter"]["mode"], "androidUid");
    assert_eq!(log["logFilter"]["uid"], 10123);
    assert_eq!(log["bugOccurredAt"], original["event"]["occurredAt"]);
    assert_eq!(
        log["commands"][0]["logBounds"]["continuousCoverageKnown"],
        false
    );
    assert!(log["commands"][0]["logBounds"]["firstDeviceLogAt"].is_string());
    assert!(log["commands"][0]["startedAt"].is_string());
    assert_eq!(log["commands"][0]["argv"][0], "-s");
    let log_argv = log["commands"][0]["argv"].as_array().unwrap();
    assert!(log_argv.iter().any(|arg| arg == "--uid=10123"));
    assert!(!log_argv.iter().any(|arg| arg == "crash"));
    assert!(files
        .iter()
        .find(|a| a.name == "bugreport-fake.zip")
        .unwrap()
        .source
        .contains("事后补采"));
    let environment = files
        .iter()
        .find(|a| a.name.starts_with("device-environment-snapshot-"))
        .unwrap();
    assert_eq!(environment.role, "sample");
    let environment_text = std::fs::read_to_string(&environment.path).unwrap();
    for evidence in [
        "shell getprop ro.product.manufacturer",
        "shell getprop ro.build.fingerprint",
        "shell getprop persist.sys.locale",
        "shell getprop persist.sys.timezone",
        "shell wm size",
        "shell wm density",
        "shell cat /proc/meminfo",
        "shell df -k /data /sdcard",
        "shell settings get system font_scale",
        "shell dumpsys package com.example.test",
    ] {
        assert!(environment_text.contains(evidence), "{evidence}");
    }
    for omitted in [
        "shell getprop\n",
        "shell settings list system",
        "shell settings list secure",
        "shell settings list global",
        "shell dumpsys netstats detail",
        "shell dumpsys connectivity",
        "shell dumpsys wifi",
    ] {
        assert!(!environment_text.contains(omitted), "{omitted}");
    }
    let calls = trace(temp.path());
    for required in ["shell ps -A", "shell dumpsys netstats detail"] {
        assert!(
            calls.iter().any(|args| args.join(" ").contains(required)),
            "{required}"
        );
    }
    for args in &calls {
        if args[0] == "devices" {
            continue;
        }
        assert_eq!(&args[..2], ["-s", "USB-A"]);
        assert!(!args.iter().any(|a| [
            "-c",
            "root",
            "screenrecord",
            "record",
            "force-stop",
            "start",
            "reset",
            "run-as"
        ]
        .contains(&a.as_str())));
        if args.iter().any(|a| {
            [
                "package",
                "meminfo",
                "gfxinfo",
                "exit-info",
                "processes",
                "get-app-locales",
            ]
            .contains(&a.as_str())
        }) {
            assert!(args.iter().any(|a| a == "com.example.test"));
        }
    }
    assert_eq!(calls.last().unwrap()[2], "bugreport");
    assert_eq!(
        calls
            .iter()
            .filter(|a| a.get(3).is_some_and(|a| a == "rm"))
            .count(),
        1
    );
    assert!(!crate::capture::pending(&state.store, &id).unwrap());
    assert!(std::fs::read_dir(temp.path().join("bugreports"))
        .unwrap()
        .next()
        .is_none());
    assert!(!std::fs::read_dir(temp.path())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .any(|entry| entry
            .file_name()
            .to_string_lossy()
            .starts_with(".post-capture-")));
}
#[tokio::test]
async fn partial_diagnostics_keep_successes_and_only_retry_the_incomplete_item() {
    let temp = tempfile::tempdir().unwrap();
    let (app, original) = app(temp.path(), fake_adb(temp.path()));
    std::fs::write(temp.path().join("mode"), "partial").unwrap();
    let state = app.state::<AppState>();
    let id = string(&original, "id");
    let job = prepare(
        &state,
        &id,
        "USB-A",
        selected(&["screenshot", "memory", "bugreport"]),
        None,
    )
    .await
    .unwrap();
    execute(&state, &string(&job, "id")).await.unwrap();
    let snapshot = post_capture_status(app.state(), id.clone()).unwrap();
    let done = &snapshot["jobs"][0];
    assert_eq!(done["status"], "partial");
    assert_eq!(done["items"]["memory"]["status"], "unavailable");
    assert_eq!(done["items"]["screenshot"]["status"], "complete");
    assert!(prepare(
        &state,
        &id,
        "USB-A",
        selected(&["screenshot"]),
        Some(string(&job, "id"))
    )
    .await
    .is_err());
    let before = state.store.draft(&id).unwrap()["attachments"]
        .as_array()
        .unwrap()
        .clone();
    std::fs::write(temp.path().join("mode"), "normal").unwrap();
    let retry = prepare(
        &state,
        &id,
        "USB-A",
        selected(&["memory"]),
        Some(string(&job, "id")),
    )
    .await
    .unwrap();
    execute(&state, &string(&retry, "id")).await.unwrap();
    let after = state.store.draft(&id).unwrap();
    assert_eq!(after["attachments"].as_array().unwrap().len(), before.len());
    let old_summary = string(done, "summaryAttachmentId");
    assert!(!after["attachments"]
        .as_array()
        .unwrap()
        .iter()
        .any(|attachment| string(attachment, "id") == old_summary));
    assert_eq!(
        trace(temp.path())
            .iter()
            .filter(|a| a.get(2).is_some_and(|a| a == "exec-out"))
            .count(),
        1
    );
}
#[tokio::test]
async fn repeated_collection_keeps_only_latest_item_files_and_one_regular_summary() {
    let temp = tempfile::tempdir().unwrap();
    let (app, original) = app(temp.path(), fake_adb(temp.path()));
    let state = app.state::<AppState>();
    let id = string(&original, "id");

    let first = prepare(&state, &id, "USB-A", selected(&["screenshot"]), None)
        .await
        .unwrap();
    execute(&state, &string(&first, "id")).await.unwrap();
    let first = state
        .store
        .get::<Value>("capture", &string(&first, "id"))
        .unwrap()
        .unwrap();
    let old_screen = first["items"]["screenshot"]["attachments"][0]
        .as_str()
        .unwrap()
        .to_string();
    let old_summary = string(&first, "summaryAttachmentId");
    let first_files = attachments(&state.store.draft(&id).unwrap()).unwrap();
    let old_screen_directory = Path::new(
        &first_files
            .iter()
            .find(|attachment| attachment.id == old_screen)
            .unwrap()
            .path,
    )
    .parent()
    .unwrap()
    .to_path_buf();
    let old_summary_directory = Path::new(
        &first_files
            .iter()
            .find(|attachment| attachment.id == old_summary)
            .unwrap()
            .path,
    )
    .parent()
    .unwrap()
    .to_path_buf();

    let second = prepare(&state, &id, "USB-A", selected(&["screenshot"]), None)
        .await
        .unwrap();
    execute(&state, &string(&second, "id")).await.unwrap();
    let after_screens = attachments(&state.store.draft(&id).unwrap()).unwrap();
    assert_eq!(
        after_screens.len(),
        original["attachments"].as_array().unwrap().len() + 2
    );
    assert!(!after_screens
        .iter()
        .any(|attachment| attachment.id == old_screen || attachment.id == old_summary));
    assert!(!old_screen_directory.exists());
    assert!(!old_summary_directory.exists());
    assert_eq!(
        after_screens
            .iter()
            .filter(|attachment| attachment.name == "post-capture-current-screen.png")
            .count(),
        1
    );
    assert_eq!(
        after_screens
            .iter()
            .filter(|attachment| attachment.name == "post-capture-summary.zip")
            .count(),
        1
    );

    let environment = prepare(&state, &id, "USB-A", selected(&["environment"]), None)
        .await
        .unwrap();
    execute(&state, &string(&environment, "id")).await.unwrap();
    let environment = state
        .store
        .get::<Value>("capture", &string(&environment, "id"))
        .unwrap()
        .unwrap();
    assert!(string(&environment, "summaryAttachmentId").is_empty());
    let old_environment = environment["items"]["environment"]["attachments"][0]
        .as_str()
        .unwrap()
        .to_string();
    let old_environment_directory = Path::new(
        &attachments(&state.store.draft(&id).unwrap())
            .unwrap()
            .iter()
            .find(|attachment| attachment.id == old_environment)
            .unwrap()
            .path,
    )
    .parent()
    .unwrap()
    .to_path_buf();
    let replacement = prepare(&state, &id, "USB-A", selected(&["environment"]), None)
        .await
        .unwrap();
    execute(&state, &string(&replacement, "id")).await.unwrap();
    let final_files = attachments(&state.store.draft(&id).unwrap()).unwrap();
    assert!(!final_files
        .iter()
        .any(|attachment| attachment.id == old_environment));
    assert!(!old_environment_directory.exists());
    assert_eq!(
        final_files
            .iter()
            .filter(|attachment| attachment.name.starts_with("device-environment-snapshot-"))
            .count(),
        1
    );
    assert_eq!(
        final_files
            .iter()
            .filter(|attachment| attachment.name == "post-capture-summary.zip")
            .count(),
        1
    );
    let compact = post_capture_status(app.state(), id).unwrap();
    assert_eq!(compact["jobs"].as_array().unwrap().len(), 2);
    assert!(compact["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|job| job["items"].get("environment").is_some()));
    assert!(compact["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|job| job["items"].get("screenshot").is_some()));
}
#[tokio::test]
async fn absent_history_does_not_claim_coverage_and_missing_mapping_blocks_all_commands() {
    let temp = tempfile::tempdir().unwrap();
    let (app, original) = app(temp.path(), fake_adb(temp.path()));
    let state = app.state::<AppState>();
    let id = string(&original, "id");
    std::fs::write(temp.path().join("mode"), "empty").unwrap();
    let job = prepare(&state, &id, "USB-A", selected(&["logs", "exit"]), None)
        .await
        .unwrap();
    execute(&state, &string(&job, "id")).await.unwrap();
    let done = post_capture_status(app.state(), id.clone()).unwrap();
    assert_eq!(done["jobs"][0]["status"], "failed");
    for item in ["logs", "exit"] {
        assert_eq!(done["jobs"][0]["items"][item]["status"], "unavailable");
    }
    let files = attachments(&state.store.draft(&id).unwrap()).unwrap();
    let log = receipt(
        files
            .iter()
            .find(|a| a.name == "post-capture-logs.zip")
            .unwrap(),
    );
    assert!(log["commands"][0]["logBounds"].is_null());
    let before = trace(temp.path()).len();
    crate::project_settings::save_for_state(
        &state,
        "test-org",
        crate::project_settings::ProjectSettings {
            entries: vec![crate::project_settings::ProjectSetting {
                project_id: "test-project".into(),
                enabled: true,
                package_name: "".into(),
            }],
        },
    )
    .unwrap();
    assert!(prepare(&state, &id, "USB-A", selected(&["logs"]), None)
        .await
        .unwrap_err()
        .contains("未配置 Android 包名"));
    assert_eq!(trace(temp.path()).len(), before);
}

#[tokio::test]
async fn unsupported_uid_logcat_safely_falls_back_to_current_package_pids() {
    let temp = tempfile::tempdir().unwrap();
    let (app, original) = app(temp.path(), fake_adb(temp.path()));
    let state = app.state::<AppState>();
    let id = string(&original, "id");
    std::fs::write(temp.path().join("mode"), "uid-unsupported").unwrap();

    let job = prepare(&state, &id, "USB-A", selected(&["logs", "crash"]), None)
        .await
        .unwrap();
    execute(&state, &string(&job, "id")).await.unwrap();

    let snapshot = post_capture_status(app.state(), id.clone()).unwrap();
    assert_eq!(snapshot["jobs"][0]["status"], "partial");
    for item in ["logs", "crash"] {
        assert_eq!(snapshot["jobs"][0]["items"][item]["status"], "partial");
    }
    let files = attachments(&state.store.draft(&id).unwrap()).unwrap();
    for item in ["logs", "crash"] {
        let result = receipt(
            files
                .iter()
                .find(|attachment| attachment.name == format!("post-capture-{item}.zip"))
                .unwrap(),
        );
        assert_eq!(result["logFilter"]["fallbackMode"], "currentProcessPid");
        assert_eq!(result["logFilter"]["historicalCoverageLimited"], true);
        assert_eq!(result["logFilter"]["processIds"], json!([321]));
        assert!(result["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command["argv"]
                .as_array()
                .unwrap()
                .iter()
                .any(|argument| argument == "--pid=321")));
        assert!(result["commands"][0]["error"]
            .as_str()
            .unwrap()
            .contains("已切换"));
    }
    let calls = trace(temp.path());
    assert!(calls
        .iter()
        .any(|args| args.iter().any(|arg| arg == "--uid=10123")));
    assert!(calls
        .iter()
        .any(|args| args == &["-s", "USB-A", "shell", "pidof", "com.example.test"]));
    assert!(calls
        .iter()
        .any(|args| args.iter().any(|arg| arg == "--pid=321")));
}

#[tokio::test]
async fn legacy_logcat_is_filtered_in_memory_and_never_saves_other_apps() {
    let temp = tempfile::tempdir().unwrap();
    let (app, original) = app(temp.path(), fake_adb(temp.path()));
    let state = app.state::<AppState>();
    let id = string(&original, "id");
    std::fs::write(temp.path().join("mode"), "legacy-logcat").unwrap();

    let job = prepare(&state, &id, "USB-A", selected(&["logs"]), None)
        .await
        .unwrap();
    execute(&state, &string(&job, "id")).await.unwrap();

    let files = attachments(&state.store.draft(&id).unwrap()).unwrap();
    let logs = files
        .iter()
        .find(|attachment| attachment.name == "post-capture-logs.zip")
        .unwrap();
    let result = receipt(logs);
    assert_eq!(result["status"], "partial");
    assert_eq!(
        result["logFilter"]["fallbackMode"],
        "localCurrentProcessPid"
    );
    let local = result["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|command| command["compatibilityFallback"] == "localCurrentProcessPid")
        .unwrap();
    assert_eq!(local["status"], "complete");
    assert_eq!(local["rawDeviceLogPersisted"], false);

    let mut zip = zip::ZipArchive::new(std::fs::File::open(&logs.path).unwrap()).unwrap();
    let name = (0..zip.len())
        .map(|index| zip.by_index(index).unwrap().name().to_string())
        .find(|name| name.ends_with("-logs.txt"))
        .unwrap();
    let mut filtered = String::new();
    zip.by_name(&name)
        .unwrap()
        .read_to_string(&mut filtered)
        .unwrap();
    assert!(filtered.contains("target failure"));
    assert!(filtered.contains("com.example.test.Main.run"));
    assert!(!filtered.contains("private other-app line"));
    assert!(!filtered.contains("another private line"));
}
#[tokio::test]
async fn authorization_injection_scope_and_duplicate_jobs_reject_before_collection() {
    let temp = tempfile::tempdir().unwrap();
    let (app, original) = app(temp.path(), fake_adb(temp.path()));
    let state = app.state::<AppState>();
    let id = string(&original, "id");
    for mode in ["offline", "unauthorized"] {
        std::fs::write(temp.path().join("mode"), mode).unwrap();
        assert!(prepare(&state, &id, "USB-A", selected(&["logs"]), None)
            .await
            .is_err());
    }
    std::fs::write(temp.path().join("mode"), "normal").unwrap();
    for serial in ["-help", "USB-A;reboot", ""] {
        assert!(prepare(&state, &id, serial, selected(&["logs"]), None)
            .await
            .is_err());
    }
    for items in [
        selected(&["shell"]),
        selected(&[]),
        selected(&["logs", "logs"]),
    ] {
        assert!(prepare(&state, &id, "USB-A", items, None).await.is_err());
    }
    assert!(state.store.list("capture").unwrap().is_empty());
    let before = trace(temp.path()).len();
    state.user.lock().unwrap().as_mut().unwrap().id = "bob".into();
    assert!(prepare(&state, &id, "USB-A", selected(&["logs"]), None)
        .await
        .is_err());
    assert_eq!(trace(temp.path()).len(), before);
    state.user.lock().unwrap().as_mut().unwrap().id = "alice".into();
    let job = prepare(&state, &id, "USB-A", selected(&["logs"]), None)
        .await
        .unwrap();
    assert!(prepare(&state, &id, "USB-A", selected(&["logs"]), None)
        .await
        .unwrap_err()
        .contains("仍在采集"));
    state.workspace_epoch.fetch_add(1, Ordering::SeqCst);
    let error = execute(&state, &string(&job, "id")).await.unwrap_err();
    interrupted(&state, &string(&job, "id"), &error).unwrap();
    assert_eq!(state.store.draft(&id).unwrap(), original);
    let stored = state
        .store
        .get::<Value>("capture", &string(&job, "id"))
        .unwrap()
        .unwrap();
    assert_eq!(stored["status"], "interrupted");
    assert_eq!(stored["items"]["logs"]["status"], "interrupted");
}
#[tokio::test]
async fn concurrent_manual_save_is_preserved_and_changed_target_stops_attachment_writes() {
    for change_package in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let (app, original) = app(temp.path(), fake_adb(temp.path()));
        let state = app.state::<AppState>();
        let id = string(&original, "id");
        std::fs::write(temp.path().join("mode"), "paused").unwrap();
        let job = prepare(
            &state,
            &id,
            "USB-A",
            selected(&["screenshot", "logs"]),
            None,
        )
        .await
        .unwrap();
        let editing = async {
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while !temp.path().join("ready").exists() {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            let mut edited = original.clone();
            edited["title"] = json!("采集过程中人工修改的标题");
            if change_package {
                crate::project_settings::save_for_state(
                    &state,
                    "test-org",
                    crate::project_settings::ProjectSettings {
                        entries: vec![crate::project_settings::ProjectSetting {
                            project_id: "test-project".into(),
                            enabled: true,
                            package_name: "com.example.changed".into(),
                        }],
                    },
                )
                .unwrap();
            }
            crate::commands::save_draft(edited, app.state()).unwrap();
            std::fs::write(temp.path().join("release"), "").unwrap();
        };
        let job_id = string(&job, "id");
        let (result, ()) = tokio::join!(execute(&state, &job_id), editing);
        if change_package {
            assert!(result.unwrap_err().contains("包名已变化"));
        } else {
            result.unwrap();
        }
        let after = state.store.draft(&id).unwrap();
        assert_eq!(after["title"], "采集过程中人工修改的标题");
        assert_eq!(after["event"], original["event"]);
        if change_package {
            // Logs completed before the paused screenshot; retain only that
            // earlier success when the target changes during the screenshot.
            assert_eq!(after["attachments"][0], original["attachments"][0]);
            assert_eq!(after["attachments"].as_array().unwrap().len(), 2);
            assert_eq!(after["attachments"][1]["name"], "post-capture-logs.zip");
        } else {
            assert_eq!(after["attachments"].as_array().unwrap().len(), 4);
        }
    }
}
#[tokio::test]
async fn disconnected_and_frozen_drafts_stop_and_restart_marks_pending_jobs_interrupted() {
    let temp = tempfile::tempdir().unwrap();
    let (app, original) = app(temp.path(), fake_adb(temp.path()));
    let state = app.state::<AppState>();
    let id = string(&original, "id");
    let job = prepare(
        &state,
        &id,
        "USB-A",
        selected(&["screenshot", "logs"]),
        None,
    )
    .await
    .unwrap();
    std::fs::write(temp.path().join("mode"), "offline").unwrap();
    let error = execute(&state, &string(&job, "id")).await.unwrap_err();
    assert!(error.contains("离线"));
    interrupted(&state, &string(&job, "id"), &error).unwrap();
    assert_eq!(state.store.draft(&id).unwrap(), original);
    std::fs::write(temp.path().join("mode"), "normal").unwrap();
    let retry = prepare(
        &state,
        &id,
        "USB-A",
        selected(&["logs"]),
        Some(string(&job, "id")),
    )
    .await
    .unwrap();
    crate::capture::recover(&state.store).unwrap();
    let recovered = state
        .store
        .get::<Value>("capture", &string(&retry, "id"))
        .unwrap()
        .unwrap();
    assert_eq!(recovered["status"], "interrupted");
    state.locks.lock().unwrap().insert(id.clone());
    assert!(prepare(&state, &id, "USB-A", selected(&["logs"]), None)
        .await
        .unwrap_err()
        .contains("正在提交"));
    assert!(execute(&state, &string(&retry, "id")).await.is_err());
}
#[tokio::test]
async fn invalid_media_is_not_attached_and_other_items_continue() {
    let temp = tempfile::tempdir().unwrap();
    let (app, original) = app(temp.path(), fake_adb(temp.path()));
    let state = app.state::<AppState>();
    let id = string(&original, "id");
    std::fs::write(temp.path().join("mode"), "bad-screen").unwrap();
    let job = prepare(
        &state,
        &id,
        "USB-A",
        selected(&["screenshot", "logs"]),
        None,
    )
    .await
    .unwrap();
    execute(&state, &string(&job, "id")).await.unwrap();
    let done = post_capture_status(app.state(), id.clone()).unwrap();
    assert_eq!(done["jobs"][0]["items"]["screenshot"]["status"], "failed");
    assert_eq!(done["jobs"][0]["items"]["logs"]["status"], "complete");
    assert_eq!(done["jobs"][0]["status"], "partial");
    let files = attachments(&state.store.draft(&id).unwrap()).unwrap();
    assert_eq!(files.len(), 3);
    assert!(files.iter().all(|a| a.mime != "image/png"));
}

/// Explicit operator-only smoke test; normal test suites never touch attached phones.
#[tokio::test]
#[ignore = "requires explicit user-authorized USB device and isolated output directory"]
async fn real_device_post_capture() {
    let root =
        PathBuf::from(std::env::var("POST_CAPTURE_REAL_OUTPUT").expect("isolated output required"));
    assert!(!root.exists(), "use a new output directory");
    let adb = PathBuf::from(std::env::var("POST_CAPTURE_REAL_ADB").expect("ADB path required"));
    let serial = std::env::var("POST_CAPTURE_REAL_SERIAL").expect("serial required");
    let package = std::env::var("POST_CAPTURE_REAL_PACKAGE").expect("target package required");
    let (app, mut draft) = app(&root, adb);
    let state = app.state::<AppState>();
    let id = string(&draft, "id");
    crate::project_settings::save_for_state(
        &state,
        "test-org",
        crate::project_settings::ProjectSettings {
            entries: vec![crate::project_settings::ProjectSetting {
                project_id: "test-project".into(),
                enabled: true,
                package_name: package.clone(),
            }],
        },
    )
    .unwrap();
    draft["app"]["packageName"] = json!(package);
    state.store.put("draft", &id, &draft).unwrap();
    let job = prepare(&state, &id, &serial, selected(ITEMS), None)
        .await
        .unwrap();
    if let Err(error) = execute(&state, &string(&job, "id")).await {
        interrupted(&state, &string(&job, "id"), &error).unwrap();
        panic!("real device: {error}");
    }
    let snapshot = post_capture_status(app.state(), id.clone()).unwrap();
    let files = attachments(&snapshot["draft"]).unwrap();
    for file in &files {
        evidence::verify_attachment(file).unwrap();
    }
    for key in ["event", "sessionId", "app", "kind", "title", "sources"] {
        assert_eq!(snapshot["draft"][key], draft[key]);
    }
    std::fs::write(
        root.join("result.json"),
        serde_json::to_vec_pretty(&snapshot).unwrap(),
    )
    .unwrap();
    let results = snapshot["jobs"][0]["items"].as_object().unwrap();
    println!(
        "real-device post-capture: status={}, verifiedAttachments={}, items={}",
        snapshot["jobs"][0]["status"],
        files.len(),
        serde_json::to_string(
            &results
                .iter()
                .map(|(key, value)| (key.clone(), value["status"].clone()))
                .collect::<serde_json::Map<_, _>>()
        )
        .unwrap()
    );
    assert_eq!(results["screenshot"]["status"], "complete");
    assert_eq!(results["bugreport"]["status"], "complete");
    assert_eq!(results["logs"]["status"], "complete");
}

#[tokio::test]
#[ignore = "requires explicit user-authorized USB device and isolated output directory"]
async fn real_device_environment_snapshot() {
    let root =
        PathBuf::from(std::env::var("POST_CAPTURE_REAL_OUTPUT").expect("isolated output required"));
    assert!(!root.exists(), "use a new output directory");
    let adb = PathBuf::from(std::env::var("POST_CAPTURE_REAL_ADB").expect("ADB path required"));
    let serial = std::env::var("POST_CAPTURE_REAL_SERIAL").expect("serial required");
    let package = std::env::var("POST_CAPTURE_REAL_PACKAGE").expect("target package required");
    let (app, _) = app(&root, adb);
    let state = app.state::<AppState>();
    let id = string(&state.store.list("draft").unwrap()[0], "id");
    crate::project_settings::save_for_state(
        &state,
        "test-org",
        crate::project_settings::ProjectSettings {
            entries: vec![crate::project_settings::ProjectSetting {
                project_id: "test-project".into(),
                enabled: true,
                package_name: package.clone(),
            }],
        },
    )
    .unwrap();
    let job = prepare(&state, &id, &serial, selected(&["environment"]), None)
        .await
        .unwrap();
    execute(&state, &string(&job, "id")).await.unwrap();
    let snapshot = post_capture_status(app.state(), id).unwrap();
    let item = &snapshot["jobs"][0]["items"]["environment"];
    assert!(["complete", "partial"].contains(&string(item, "status").as_str()));
    let files = attachments(&snapshot["draft"]).unwrap();
    let attachment = files
        .iter()
        .find(|file| file.name.starts_with("device-environment-snapshot-"))
        .expect("environment text attachment");
    evidence::verify_attachment(attachment).unwrap();
    let text = std::fs::read_to_string(&attachment.path).unwrap();
    for expected in [
        &format!("项目配置包名: {package}"),
        "shell getprop ro.product.manufacturer",
        "shell getprop ro.build.fingerprint",
        "shell getprop persist.sys.locale",
        "shell getprop persist.sys.timezone",
        "shell wm size",
        "shell wm density",
        "shell cat /proc/meminfo",
        "shell df -k /data /sdcard",
        "shell dumpsys battery",
        "shell settings get system font_scale",
        &format!("shell dumpsys package {package}"),
    ] {
        assert!(text.contains(expected), "missing {expected}");
    }
    for omitted in [
        "shell getprop\n",
        "shell settings list system",
        "shell settings list secure",
        "shell settings list global",
        "shell dumpsys netstats detail",
        "shell dumpsys connectivity",
        "shell dumpsys wifi",
    ] {
        assert!(!text.contains(omitted), "unexpected {omitted}");
    }
    std::fs::write(
        root.join("result.json"),
        serde_json::to_vec_pretty(&snapshot).unwrap(),
    )
    .unwrap();
    println!(
        "real-device environment snapshot: status={}, attachment={}, bytes={}",
        item["status"], attachment.name, attachment.size
    );
}
