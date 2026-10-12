use crate::model::{string, Result, Submission};
use serde_json::{json, Value};

/// Fallback route confirmed by the user's real SKII-608 Bug page.
/// It uses serialNumber (the human-readable identifier), never the database ID.
pub fn bug_url(returned_url: Option<&str>, identifier: Option<&str>) -> Option<String> {
    if let Some(value) = returned_url.filter(|value| {
        reqwest::Url::parse(value).ok().is_some_and(|url| {
            url.scheme() == "https"
                && url.host_str() == Some("devops.aliyun.com")
                && url.port_or_known_default() == Some(443)
                && url.username().is_empty()
                && url.password().is_none()
        })
    }) {
        return Some(value.into());
    }
    let identifier = identifier?.trim();
    if identifier.len() > 128
        || !regex::Regex::new(r"^[A-Za-z0-9][A-Za-z0-9_-]*-[0-9]+$")
            .unwrap()
            .is_match(identifier)
    {
        return None;
    }
    Some(format!("https://devops.aliyun.com/projex/bug/{identifier}"))
}

/// Enrich old history responses without rewriting stored submissions/evidence.
pub fn history_with_urls(mut submissions: Vec<Value>) -> Vec<Value> {
    for submission in &mut submissions {
        submission["url"] = json!(bug_url(
            submission["url"].as_str(),
            submission["identifier"].as_str(),
        ));
    }
    submissions
}

pub fn receipt_text(submission: &Submission, draft: &Value, repair: bool) -> Result<String> {
    let url = bug_url(submission.url.as_deref(), submission.identifier.as_deref());
    if repair {
        Ok(format!(
            "请修复 Android 缺陷：{}\n云效工作项：{}\n链接：{}\n版本：{}；迭代：{}\nevidenceId：{}\n证据状态：{}",
            string(draft, "title"),
            submission.workitem_id.as_deref().unwrap_or_default(),
            url.as_deref().unwrap_or("云效暂未提供可用工作项编号或链接，请从项目中打开"),
            draft["versions"],
            string(draft, "sprint"),
            submission.evidence.id,
            submission.state,
        ))
    } else {
        url.ok_or("云效暂未提供可用工作项编号或链接；请按工作项 ID 在项目中查找".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_confirmed_serial_number_builds_the_bug_page() {
        assert_eq!(
            bug_url(None, Some("SKII-608")).as_deref(),
            Some("https://devops.aliyun.com/projex/bug/SKII-608"),
        );
        assert_eq!(
            bug_url(Some(""), Some(" SKII-608 ")),
            bug_url(None, Some("SKII-608"))
        );
    }

    #[test]
    fn valid_returned_url_takes_precedence() {
        let url = "https://devops.aliyun.com/projex/bug/SKII-608#";
        assert_eq!(bug_url(Some(url), Some("OTHER-1")).as_deref(), Some(url));
    }

    #[test]
    fn invalid_links_fall_back_to_serial_number() {
        for url in [
            "http://devops.aliyun.com/projex/bug/SKII-608",
            "https://example.com/projex/bug/SKII-608",
            "https://devops.aliyun.com.evil.test/bug/SKII-608",
            "https://user:secret@devops.aliyun.com/bug/SKII-608",
            "https://devops.aliyun.com:8080/bug/SKII-608",
        ] {
            assert_eq!(
                bug_url(Some(url), Some("SKII-608")),
                bug_url(None, Some("SKII-608"))
            );
        }
    }

    #[test]
    fn missing_or_invalid_serials_do_not_invent_links() {
        for value in [
            "",
            "48ba2a3934a4855d907e9d8e79",
            "../SKII-608",
            "SKII-608#other",
            "SKII-608?x=1",
            "SKII 608",
            "SKII-A",
        ] {
            assert_eq!(bug_url(None, Some(value)), None, "{value}");
        }
        assert_eq!(bug_url(None, None), None);
        assert_eq!(bug_url(None, Some(&format!("{}-1", "A".repeat(128)))), None);
    }

    #[test]
    fn old_history_gets_a_url_without_changing_workitem_identity() {
        let row = json!({"identifier":"SKII-608","url":null,"workitemId":"48ba2a3934a4855d907e9d8e79","state":"submitted"});
        let rows = history_with_urls(vec![row.clone(), json!({"identifier":null,"url":null})]);
        assert_eq!(
            rows[0]["url"],
            "https://devops.aliyun.com/projex/bug/SKII-608"
        );
        assert_eq!(rows[0]["workitemId"], row["workitemId"]);
        assert_eq!(rows[0]["state"], row["state"]);
        assert!(row["url"].is_null());
        assert!(rows[1]["url"].is_null());
    }

    #[test]
    fn old_submission_bootstrap_and_copy_share_the_real_bug_route() {
        use crate::{commands::AppState, model::Metadata, store::Store};
        use std::{
            collections::HashSet,
            sync::{atomic::AtomicU64, Mutex},
        };
        use tauri::Manager;

        let temp = tempfile::tempdir().unwrap();
        let store = Store::new(temp.path()).unwrap();
        let draft: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/valid-draft.json")).unwrap();
        let metadata: Metadata =
            serde_json::from_str(include_str!("../../tests/fixtures/metadata.json")).unwrap();
        let id = string(&draft, "id");
        let old = json!({
            "draftId":id,"profile":metadata.profile,"state":"submitted",
            "workitemId":"48ba2a3934a4855d907e9d8e79","identifier":"SKII-608","url":null,
            "evidence":{"id":"80ba8a06-5e08-4ab2-9fb0-1d01b8fbaa6f","directory":"mock-evidence","zipPath":"mock-evidence.zip","sha256":"unchanged","files":[]},
            "uploads":[],"error":null,"intendedAt":"2026-10-10T00:00:00Z","updatedAt":"2026-10-10T00:00:00Z"
        });
        store.put("draft", &id, &draft).unwrap();
        store.put("submission", &id, &old).unwrap();
        let state = AppState {
            store: store.clone(),
            profile: Mutex::new(metadata.profile.clone()),
            token: Mutex::new(None),
            user: Mutex::new(None),
            auth_epoch: AtomicU64::new(0),
            workspace_epoch: AtomicU64::new(0),
            oauth: Mutex::new(None),
            oauth_flow: Default::default(),
            api_base: "http://127.0.0.1:1".into(),
            metadata: Mutex::new(Some(metadata.clone())),
            adb: Mutex::new(None),
            collector: Mutex::new(None),
            page_observer: Mutex::new(None),
            page_observer_generation: std::sync::Arc::new(AtomicU64::new(0)),
            locks: Mutex::new(HashSet::new()),
        };
        let app = tauri::test::mock_builder()
            .manage(state)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let bootstrap = crate::commands::bootstrap(app.state()).unwrap();
        let saved: Submission = store.get("submission", &id).unwrap().unwrap();
        let copied_url = receipt_text(&saved, &draft, false).unwrap();
        let copied_repair = receipt_text(&saved, &draft, true).unwrap();
        assert_eq!(bootstrap["submissions"][0]["url"], copied_url);
        assert_eq!(copied_url, "https://devops.aliyun.com/projex/bug/SKII-608");
        assert!(copied_repair.contains(&format!("链接：{copied_url}")));
        assert!(copied_repair.contains("云效工作项：48ba2a3934a4855d907e9d8e79"));
        assert!(!copied_repair.contains("API 未返回"));
        assert_eq!(store.get::<Value>("submission", &id).unwrap().unwrap(), old);
        assert_eq!(store.list("submission").unwrap().len(), 1);
        let mut missing = saved;
        missing.identifier = None;
        assert!(receipt_text(&missing, &draft, false).is_err());
        assert!(receipt_text(&missing, &draft, true)
            .unwrap()
            .contains("云效暂未提供可用工作项编号或链接"));

        if let Ok(path) = std::env::var("BUG_LINKS_BROWSER_FIXTURE") {
            std::fs::write(
                path,
                serde_json::to_vec_pretty(&json!({
                    "bootstrap":bootstrap,"draft":draft,"metadata":metadata,
                    "copiedRepair":copied_repair,"copiedUrl":copied_url,
                }))
                .unwrap(),
            )
            .unwrap();
        }
    }
}
