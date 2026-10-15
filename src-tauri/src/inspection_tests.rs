//! Local fake-ADB verification of the inspection command's persistence boundary.
#![cfg(unix)]
use crate::{adb, commands, evidence, model::*, store::Store, validation};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use tauri::Manager;

fn fake_adb(root: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = root.join("inspect-adb.py");
    std::fs::write(&path, r#"#!/usr/bin/env python3
import sys, json, time
from pathlib import Path
root = Path(__file__).parent
args = sys.argv[1:]
mode = (root / 'mode').read_text()
with (root / 'calls.jsonl').open('a') as log:
    log.write(json.dumps(args) + '\n')
if args[0] == 'devices':
    state = 'offline' if mode == 'offline' else 'device'
    print(f'List of devices attached\nUSB-A {state} model:FakeA\nUSB-B device model:FakeB')
    sys.exit(0)
assert args[0] == '-s' and args[1] in ['USB-A', 'USB-B'], args
serial = args[1]
args = args[2:]
assert args[0] == 'shell', args
if args[1:3] == ['dumpsys', 'activity']:
    if mode == 'paused':
        (root / 'ready').touch()
        while not (root / 'release').exists(): time.sleep(.01)
    package = 'com.google.android.apps.nexuslauncher' if mode == 'foreign' else 'com.example.test'
    activity = '.NexusLauncherActivity' if mode == 'foreign' else '.Home'
    print('mResumedActivity: null' if mode == 'missing' else f'mResumedActivity: ActivityRecord{{abc u0 {package}/{activity} t1}}')
elif args[1:3] == ['dumpsys', 'window']:
    package = 'com.google.android.apps.nexuslauncher' if mode == 'foreign' else 'com.example.test'
    activity = '.NexusLauncherActivity' if mode == 'foreign' else '.Home'
    print('mCurrentFocus=null' if mode == 'missing' else f'mCurrentFocus=Window{{abc u0 {package}/{activity}}}')
elif args[1:3] == ['dumpsys', 'package']:
    print('versionName=2.0\nversionCode=0 minSdk=24')
elif args[1:3] == ['am', 'get-current-user']: print('0')
elif args[1:3] == ['pm', 'list']: print('package:com.example.test')
elif args[1] == 'getprop':
    print(('Model-' + serial) if args[2] == 'ro.product.model' else '14')
elif args[1] == 'cat': print('MemTotal: 8192000 kB\nMemAvailable: 4096000 kB')
elif args[1:3] == ['wm', 'size']: print('Physical size: 1080x2400')
elif args[1:3] == ['wm', 'density']: print('Physical density: 420')
elif args[1] == 'date': print('2026-10-09T12:00:00+0800')
elif args[1] in ['df', 'settings']: print('1')
elif args[1:3] == ['dumpsys', 'battery']: print('level: 80')
else: sys.exit(1)
"#).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(root.join("mode"), "normal").unwrap();
    path
}

fn app(root: &Path, adb: PathBuf) -> (tauri::App<tauri::test::MockRuntime>, Value) {
    let state = commands::AppState::new(root.into()).unwrap();
    let metadata: Metadata =
        serde_json::from_str(include_str!("../../tests/fixtures/metadata.json")).unwrap();
    let mut draft: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/valid-draft.json")).unwrap();
    draft["collectionErrors"] = json!(["原有采集缺口"]);
    draft["previewConfirmed"] = json!(true);
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

#[test]
fn saving_an_event_before_session_start_preserves_its_source() {
    let temp = tempfile::tempdir().unwrap();
    let (app, mut draft) = app(temp.path(), PathBuf::from("unused-adb"));
    let store = &app.state::<commands::AppState>().store;
    let id = string(&draft, "id");
    let session_id = uuid();
    draft["sessionId"] = json!(session_id);
    draft["event"]["source"] = json!("auto");
    store.put("draft", &id, &draft).unwrap();
    store
        .put(
            "session",
            &session_id,
            &json!({"startedAt":"2026-10-09T09:00:00+08:00"}),
        )
        .unwrap();
    let saved = commands::save_draft(draft, app.state()).unwrap();
    assert_eq!(saved["event"]["source"], "auto");
    assert!(saved["event"].get("lateCapture").is_none());
    validation::check_shape(&saved).unwrap();
}

#[test]
fn stale_page_observer_generation_cannot_write_the_draft() {
    let temp = tempfile::tempdir().unwrap();
    let (app, draft) = app(temp.path(), PathBuf::from("unused-adb"));
    let store = &app.state::<commands::AppState>().store;
    let id = string(&draft, "id");
    let target = adb::PageObservationTarget {
        store: store.clone(),
        draft_id: id.clone(),
        scope: "alice:test-org:test-project:".into(),
        package: "com.example.test".into(),
    };
    let generation = AtomicU64::new(2);
    assert!(!adb::record_observed_page(
        &target,
        "com.example.test.StaleActivity",
        "stale observer",
        "2026-10-12T09:00:00Z",
        Some((&generation, 1)),
    )
    .unwrap());
    assert_eq!(store.draft(&id).unwrap()["bugPage"], draft["bugPage"]);
}

#[test]
fn startup_removes_a_legacy_automatic_launcher_page() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::new(temp.path()).unwrap();
    let mut draft: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/valid-draft.json")).unwrap();
    let id = string(&draft, "id");
    draft["bugPage"] = json!("com.google.android.apps.nexuslauncher.NexusLauncherActivity");
    draft["sources"]["bugPage"] = json!({
        "status":"collected","at":"2026-10-10T00:00:00Z","detail":"旧版 ADB 前台页面"
    });
    store.put("draft", &id, &draft).unwrap();
    drop(store);
    let state = commands::AppState::new(temp.path().to_path_buf()).unwrap();
    let migrated = state.store.draft(&id).unwrap();
    assert_eq!(migrated["bugPage"], "");
    assert_eq!(migrated["sources"]["bugPage"]["status"], "not_applicable");
    assert!(string(&migrated, "sources.bugPage.detail").contains("NexusLauncherActivity"));
}

#[tokio::test]
async fn inspected_values_survive_save_and_partial_device_switch_without_losing_manual_data() {
    let temp = tempfile::tempdir().unwrap();
    let fake = fake_adb(temp.path());
    let (app, initial) = app(temp.path(), fake);
    let id = string(&initial, "id");
    let snapshot = commands::inspect_device(id.clone(), "USB-A".into(), app.state())
        .await
        .unwrap();
    assert_eq!(snapshot["details"]["foregroundActivity"], ".Home");
    assert_eq!(snapshot["bugPage"], "com.example.test.Home");
    let store = &app.state::<commands::AppState>().store;
    let draft = store.draft(&id).unwrap();
    validation::check_shape(&draft).unwrap();
    assert_eq!(draft["bugPage"], snapshot["bugPage"]);
    assert_eq!(draft["sources"]["bugPage"]["status"], "collected");
    assert_eq!(draft["app"]["versionCode"], "0");
    assert_eq!(draft["attachments"], initial["attachments"]);
    assert_eq!(draft["title"], initial["title"]);
    assert_eq!(draft["previewConfirmed"], false);
    assert_eq!(
        store.get::<Value>("snapshot", &id).unwrap().unwrap(),
        snapshot
    );
    // This assertion exercises stale renderer-save merging, not the live
    // observer's polling cadence. Stop the observer before injecting the next
    // deterministic observation so its immediate first tick cannot race the
    // synthetic DetailActivity value back to the fake ADB's Home value.
    app.state::<commands::AppState>().stop_page_observer();
    let mut stale = draft.clone();
    adb::record_observed_page(
        &adb::PageObservationTarget {
            store: store.clone(),
            draft_id: id.clone(),
            scope: "alice:test-org:test-project:".into(),
            package: "com.example.test".into(),
        },
        "com.example.test.DetailActivity",
        "test observer",
        "2026-10-12T09:00:00Z",
        None,
    )
    .unwrap();
    stale["title"] = json!("设备读取后保留用户的标题编辑");
    let saved = commands::save_draft(stale, app.state()).unwrap();
    assert_eq!(saved["bugPage"], "com.example.test.DetailActivity");
    assert_eq!(saved["app"]["packageName"], "com.example.test");
    assert_eq!(saved["sources"]["app.packageName"]["status"], "manual");
    assert_eq!(
        saved["sources"]["app.packageName"]["detail"],
        "来自设置页当前项目包名映射"
    );
    assert_eq!(saved["sources"]["device.model"]["status"], "collected");
    assert_eq!(saved["sources"]["bugPage"]["status"], "collected");
    std::fs::write(temp.path().join("mode"), "foreign").unwrap();
    let foreign = commands::inspect_device(id.clone(), "USB-B".into(), app.state())
        .await
        .unwrap();
    assert_eq!(foreign["bugPage"], "");
    assert_eq!(foreign["sources"]["bugPage"]["status"], "not_applicable");
    let retained_after_launcher = store.draft(&id).unwrap();
    assert_eq!(
        retained_after_launcher["bugPage"],
        "com.example.test.DetailActivity"
    );
    assert_eq!(
        retained_after_launcher["sources"]["bugPage"]["status"],
        "collected"
    );
    let profile = app
        .state::<commands::AppState>()
        .profile
        .lock()
        .unwrap()
        .clone();
    let report = evidence::report(&retained_after_launcher, &profile);
    assert!(!report.contains("NexusLauncherActivity"));
    assert!(report.contains("com.example.test.DetailActivity"));
    std::fs::write(temp.path().join("mode"), "missing").unwrap();
    commands::inspect_device(id.clone(), "USB-B".into(), app.state())
        .await
        .unwrap();
    let partial = store.draft(&id).unwrap();
    assert_eq!(partial["bugPage"], "com.example.test.DetailActivity");
    assert_eq!(partial["sources"]["bugPage"]["status"], "collected");
    validation::check_shape(&partial).unwrap();
    assert_eq!(partial["device"]["model"], "Model-USB-B");
    assert_eq!(partial["app"]["packageName"], "com.example.test");
    assert_eq!(partial["app"]["versionName"], "2.0");
    assert_eq!(partial["app"]["versionCode"], "0");
    assert_eq!(partial["sources"]["app.packageName"]["status"], "manual");
    assert_eq!(partial["title"], saved["title"]);
    assert!(partial["collectionErrors"]
        .as_array()
        .unwrap()
        .contains(&json!("原有采集缺口")));
    let mut manual = partial;
    manual["bugPage"] = json!("照片恢复结果页");
    manual["sources"]["bugPage"] = json!({"status":"manual","at":now(),"detail":"人工输入"});
    let manual = commands::save_draft(manual, app.state()).unwrap();
    assert_eq!(manual["sources"]["bugPage"]["status"], "manual");
    commands::inspect_device(id.clone(), "USB-B".into(), app.state())
        .await
        .unwrap();
    let retained = store.draft(&id).unwrap();
    assert_eq!(retained["bugPage"], "照片恢复结果页");
    assert_eq!(retained["sources"]["bugPage"], manual["sources"]["bugPage"]);
    std::fs::write(temp.path().join("mode"), "normal").unwrap();
    commands::inspect_device(id.clone(), "USB-B".into(), app.state())
        .await
        .unwrap();
    let refreshed = store.draft(&id).unwrap();
    assert_eq!(refreshed["bugPage"], "照片恢复结果页");
    assert_eq!(refreshed["sources"]["bugPage"]["status"], "manual");
    let mut cleared = refreshed;
    cleared["bugPage"] = json!("");
    cleared["sources"]["bugPage"] = json!({"status":"manual","at":now(),"detail":"人工清空"});
    let cleared = commands::save_draft(cleared, app.state()).unwrap();
    assert_eq!(cleared["bugPage"], "");
    commands::inspect_device(id.clone(), "USB-B".into(), app.state())
        .await
        .unwrap();
    let still_cleared = store.draft(&id).unwrap();
    assert_eq!(still_cleared["bugPage"], "");
    assert_eq!(still_cleared["sources"]["bugPage"]["status"], "manual");
    let calls = std::fs::read_to_string(temp.path().join("calls.jsonl")).unwrap();
    for call in calls.lines() {
        let args: Vec<String> = serde_json::from_str(call).unwrap();
        assert!(
            args[0] == "devices"
                || (args[0] == "-s" && ["USB-A", "USB-B"].contains(&args[1].as_str()))
        );
    }
}

#[test]
fn changed_project_mapping_overrides_renderer_package_without_manual_source_claim() {
    let temp = tempfile::tempdir().unwrap();
    let (app, draft) = app(temp.path(), PathBuf::from("unused-adb"));
    let state = app.state::<commands::AppState>();
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
    let saved = commands::save_draft(draft, state).unwrap();
    assert_eq!(saved["app"]["packageName"], "com.example.changed");
    assert_eq!(
        saved["sources"]["app.packageName"],
        json!({
            "status":"manual",
            "at":saved["sources"]["app.packageName"]["at"],
            "detail":"来自设置页当前项目包名映射"
        })
    );
    assert!(!saved["collectionErrors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|error| error
            .as_str()
            .is_some_and(|error| error.contains("packageName 被人工修改"))));
}

#[tokio::test]
async fn inspection_rejects_offline_wrong_scope_and_concurrent_edits_without_persisting_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let fake = fake_adb(temp.path());
    let (app, initial) = app(temp.path(), fake);
    let id = string(&initial, "id");
    std::fs::write(temp.path().join("mode"), "offline").unwrap();
    assert!(
        commands::inspect_device(id.clone(), "USB-A".into(), app.state())
            .await
            .unwrap_err()
            .contains("离线")
    );
    let trace = std::fs::read_to_string(temp.path().join("calls.jsonl")).unwrap();
    assert_eq!(trace.lines().count(), 1);
    let state = app.state::<commands::AppState>();
    state.user.lock().unwrap().as_mut().unwrap().id = "bob".into();
    assert!(
        commands::inspect_device(id.clone(), "USB-A".into(), app.state())
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(temp.path().join("calls.jsonl")).unwrap(),
        trace
    );
    state.user.lock().unwrap().as_mut().unwrap().id = "alice".into();
    std::fs::write(temp.path().join("mode"), "paused").unwrap();
    let inspect = commands::inspect_device(id.clone(), "USB-A".into(), app.state());
    let edit = async {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !temp.path().join("ready").exists() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let mut draft = initial.clone();
        draft["title"] = json!("并发修改必须保留这份人工编辑");
        commands::save_draft(draft, app.state()).unwrap();
        std::fs::write(temp.path().join("release"), "continue").unwrap();
    };
    let (result, ()) = tokio::join!(inspect, edit);
    assert!(result.unwrap_err().contains("草稿已被修改"));
    assert_eq!(
        state.store.draft(&id).unwrap()["title"],
        "并发修改必须保留这份人工编辑"
    );
    assert!(state.store.get::<Value>("snapshot", &id).unwrap().is_none());
}
