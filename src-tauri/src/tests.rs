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
fn every_base_missing_field_is_blocked() {
    let m = metadata();
    for path in [
        "title",
        "business",
        "workitemTypeId",
        "assignedTo",
        "buildUrl",
        "preconditions",
        "steps",
        "expected",
        "actual",
        "app.packageName",
        "app.versionName",
        "app.versionCode",
        "app.channel",
        "app.environment",
        "app.obfuscation",
        "app.hardening",
        "device.model",
        "device.androidVersion",
        "device.totalMemKb",
        "device.availableMemKb",
        "device.resolution",
        "device.density",
        "event.occurredAt",
        "event.sourceNote",
        "manualNote",
        "cloud.reason",
    ] {
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
fn schema_rejects_injected_keys_and_invalid_hash() {
    let mut d = fixture();
    d["token"] = json!("never-store");
    assert!(validation::check_shape(&d).is_err());
    d.as_object_mut().unwrap().remove("token");
    d["attachments"][0]["sha256"] = json!("fake");
    assert!(validation::check_shape(&d).is_err());
}
#[test]
fn build_links_and_memory_are_honest() {
    assert!(validation::fixed_build_url("http://ci/job/App/lastBuild/").is_none());
    assert!(validation::fixed_build_url("http://ci/job/Folder/job/App/42/").is_some());
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
    assert_eq!(manifest["installedArtifact"]["fingerprints"], Value::Null);
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
                .push((id.clone(), name, body[start..end].to_vec()));
            if state.lose_upload {
                state.lose_upload = false;
                return ResponseTemplate::new(500);
            }
            return ResponseTemplate::new(200).set_body_json(json!({"id":id}));
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
    assert!(s.url.is_none());
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
    });
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
 if '-d' in args:
  for offset in [-125,-120,-119]: print(f'{time.time()+offset:.6f} 11 11 I Mock: secret@example.test before')
 else:
  while True:
   print(f'{time.time():.6f} 22 22 E OtherProcess: secret@example.test live',flush=True);time.sleep(.1)
 sys.exit(0)
if args[:1]==['exec-out']: sys.stdout.buffer.write(Path(PNG).read_bytes());sys.exit(0)
if args[:1]==['bugreport']:
 with zipfile.ZipFile(Path(args[1])/'bugreport.zip','w') as z: z.writestr('report.txt','mock system report')
 sys.exit(0)
if args[:2]==['shell','date']:
 print(int(time.time()) if args[-1]=='+%s' else '2026-10-09T08:00:00+0800');sys.exit(0)
if args[:3]==['shell','am','get-current-user']: print('0');sys.exit(0)
if args[:3]==['shell','pm','list']: print('package:com.example.app');sys.exit(0)
if args[:3]==['shell','dumpsys','package']: print('versionName=1.0\nversionCode=0 minSdk=24');sys.exit(0)
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
    crate::commands::start_session(
        "integration-device".into(),
        "com.example.app".into(),
        string(&d, "buildUrl"),
        string(&d, "business"),
        app.state(),
    )
    .await
    .unwrap();
    let marked = crate::commands::mark_problem(id.clone(), app.state())
        .await
        .unwrap();
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
    assert_eq!(
        store.draft(&id).unwrap()["title"],
        "计时期间的人工补充不能丢失"
    );
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
    assert!(!rows.iter().find(|a| a.id == wid).unwrap().confirmed);
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
    final_d["app"]["channel"] = d["app"]["channel"].clone();
    final_d["app"]["environment"] = d["app"]["environment"].clone();
    final_d["previewConfirmed"] = json!(true);
    validation::check_shape(&final_d).unwrap();
    store.put("draft", &id, &final_d).unwrap();
    let submitted = crate::commands::submit_bug(id.clone(), app.state())
        .await
        .unwrap();
    assert_eq!(submitted.state, "submitted");
    assert_eq!(submitted.uploads.len(), 9, "bundle + original selected log + redacted window + two receipts + PNG/JPEG/MP4 + system report");
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
    crate::commands::start_session(
        "integration-device".into(),
        "com.example.app".into(),
        string(&d, "buildUrl"),
        string(&d, "business"),
        app.state(),
    )
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
