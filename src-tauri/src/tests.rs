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
    body: Value,
    files: Vec<(String, String, Vec<u8>)>,
    lose_create: bool,
    lose_upload: bool,
    reject_upload: bool,
    unsupported_field: bool,
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
            return ResponseTemplate::new(200).set_body_json(if state.creates > 0 {
                json!([{"id":"real-id"}])
            } else {
                json!([])
            });
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
