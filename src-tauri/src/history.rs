use crate::model::{string, Metadata, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::Ordering;

pub async fn list_for_state(
    state: &crate::commands::AppState,
    page: usize,
    filters: HistoryFilters,
    project_id: Option<&str>,
) -> Result<crate::workspace::HistoryPage> {
    filters.validate()?;
    // Capture identity, scope and token together; don't mix different logins.
    let (user_id, scope, api, epoch, workspace_epoch) = {
        let user = state.user.lock().unwrap();
        let user = user.as_ref().ok_or("请先验证登录令牌")?;
        let profile = state.profile.lock().unwrap();
        if profile.project_id == "unconfigured" {
            return Err("请在主页面选择项目".into());
        }
        if project_id.is_some_and(|id| id != profile.project_id) {
            return Err("主页面项目已变化，请刷新历史".into());
        }
        let token = state.token.lock().unwrap();
        (
            user.id.clone(),
            crate::workspace::scope_key(user, &profile, ""),
            crate::yunxiao::Api::with_base(
                profile.clone(),
                token.as_deref().ok_or("请先验证登录令牌")?,
                &state.api_base,
            )?
            .with_oauth(state.oauth.lock().unwrap().clone()),
            state.auth_epoch.load(Ordering::SeqCst),
            state.workspace_epoch.load(Ordering::SeqCst),
        )
    };
    let metadata = state.metadata.lock().unwrap().clone().filter(|m| {
        m.profile.organization_id == api.profile.organization_id
            && m.profile.project_id == api.profile.project_id
    });
    let mut known_kinds = HashMap::new();
    if !filters.kind.is_empty() {
        for submission in state.store.list("submission")? {
            if string(&submission, "profile.organizationId") != api.profile.organization_id
                || string(&submission, "profile.projectId") != api.profile.project_id
            {
                continue;
            }
            let draft_id = string(&submission, "draftId");
            if state
                .store
                .get::<String>("draft_scope", &draft_id)?
                .as_deref()
                != Some(&scope)
            {
                continue;
            }
            let workitem_id = string(&submission, "workitemId");
            if !workitem_id.is_empty() {
                if let Some(draft) = state.store.get::<Value>("draft", &draft_id)? {
                    let kind = string(&draft, "kind");
                    if KINDS.contains(&kind.as_str()) {
                        known_kinds.insert(workitem_id, kind);
                    }
                }
            }
        }
    }
    let result = api
        .history_filtered(&user_id, page, &filters, metadata.as_ref(), &known_kinds)
        .await;
    // A late 401 must not invalidate a replacement account; stale success must
    // not appear in another project's list, including an A -> B -> A switch.
    if epoch != state.auth_epoch.load(Ordering::SeqCst)
        || workspace_epoch != state.workspace_epoch.load(Ordering::SeqCst)
        || crate::workspace::current_scope(state, "")? != scope
    {
        return Err("账号或主页面项目已变化，请刷新历史".into());
    }
    crate::workspace::observed_error(state, result)
}

pub const KINDS: &[&str] = &[
    "功能",
    "UI",
    "多语言",
    "动态过程",
    "崩溃",
    "ANR",
    "性能",
    "网络",
    "数据",
    "未记录",
];
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct HistoryFilters {
    pub kind: Vec<String>,
    pub workitem_type_id: Vec<String>,
    pub assigned_to: Vec<String>,
    pub sprint: Vec<String>,
    pub verifier: Vec<String>,
    pub priority: Vec<String>,
    pub severity: Vec<String>,
    pub participants: Vec<String>,
    pub trackers: Vec<String>,
    pub keyword: String,
}
impl HistoryFilters {
    pub fn validate(&self) -> Result<()> {
        if self.keyword.chars().count() > 250
            || self.kind.len() > KINDS.len()
            || self.kind.iter().any(|v| !KINDS.contains(&v.as_str()))
        {
            return Err("历史筛选条件不正确".into());
        }
        for values in [
            &self.workitem_type_id,
            &self.assigned_to,
            &self.sprint,
            &self.verifier,
            &self.priority,
            &self.severity,
            &self.participants,
            &self.trackers,
        ] {
            if values.len() > 100 {
                return Err("历史筛选选项过多".into());
            }
            for value in values {
                crate::yunxiao::encode_id(value)?;
            }
        }
        Ok(())
    }
    pub fn conditions(&self, user_id: &str) -> Value {
        let mut group = vec![
            json!({"fieldIdentifier":"creator","operator":"CONTAINS","value":[user_id],"toValue":null,"className":"user","format":"list"}),
        ];
        // These condition shapes are documented by SearchWorkitems. Other fields
        // are matched exactly against the bounded response, without guessing the
        // web application's undocumented custom-field query classes.
        if !self.assigned_to.is_empty() {
            group.push(json!({"fieldIdentifier":"assignedTo","operator":"CONTAINS","value":self.assigned_to,"toValue":null,"className":"user","format":"list"}));
        }
        if !self.keyword.trim().is_empty() {
            group.push(json!({"fieldIdentifier":"subject","operator":"CONTAINS","value":[self.keyword.trim()],"toValue":null,"className":"string","format":"input"}));
        }
        json!({"conditionGroups":[group]})
    }
    pub fn matches(
        &self,
        row: &Value,
        metadata: Option<&Metadata>,
        known_kinds: &HashMap<String, String>,
    ) -> bool {
        let description = string(row, "description");
        let recorded_kind = description
            .lines()
            .collect::<Vec<_>>()
            .windows(3)
            .find_map(|lines| {
                (lines[0].trim() == "## 问题类型" && KINDS.contains(&lines[2].trim()))
                    .then(|| lines[2].trim().to_string())
            });
        let kind = recorded_kind
            .or_else(|| known_kinds.get(&string(row, "id")).cloned())
            .unwrap_or_else(|| "未记录".into());
        if !any(&self.kind, &[kind])
            || !any(&self.workitem_type_id, &[string(row, "workitemType.id")])
            || !any(&self.assigned_to, &[string(row, "assignedTo.id")])
            || !any(&self.sprint, &[string(row, "sprint.id")])
        {
            return false;
        }
        let keyword = self.keyword.trim().to_lowercase();
        if !keyword.is_empty() && !string(row, "subject").to_lowercase().contains(&keyword) {
            return false;
        }
        for (key, wanted) in [
            ("verifier", &self.verifier),
            ("priority", &self.priority),
            ("severity", &self.severity),
            ("participants", &self.participants),
            ("trackers", &self.trackers),
        ] {
            let mut actual = row_values(&row[key]);
            let ids = field_ids(key, metadata);
            for field in row["customFieldValues"].as_array().into_iter().flatten() {
                if ids.contains(&string(field, "fieldId")) {
                    for value in field["values"].as_array().into_iter().flatten() {
                        actual.push(string(value, "identifier"));
                    }
                }
            }
            if !any(wanted, &actual) {
                return false;
            }
        }
        true
    }
}
fn any(wanted: &[String], actual: &[String]) -> bool {
    wanted.is_empty() || wanted.iter().any(|v| actual.contains(v))
}
fn row_values(value: &Value) -> Vec<String> {
    if let Some(items) = value.as_array() {
        items.iter().flat_map(row_values).collect()
    } else if let Some(id) = value.as_str() {
        vec![id.to_string()]
    } else {
        let id = string(value, "id");
        if id.is_empty() {
            vec![]
        } else {
            vec![id]
        }
    }
}
fn field_ids(key: &str, metadata: Option<&Metadata>) -> Vec<String> {
    let mut ids = vec![key.to_string()];
    if let Some(metadata) = metadata {
        for field in &metadata.fields {
            if crate::validation::native_target(field, &metadata.profile) == Some(key)
                || (key == "priority" && field.name.trim() == "优先级")
                || (key == "severity"
                    && ["严重程度", "严重等级", "严重性"].contains(&field.name.trim()))
            {
                ids.push(field.id.clone());
            }
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        matchers::{body_partial_json, method},
        Mock, MockServer, ResponseTemplate,
    };
    fn api(server: &MockServer) -> crate::yunxiao::Api {
        let mut profile = crate::workspace::empty_profile();
        profile.organization_id = "org-a".into();
        profile.project_id = "project-a".into();
        crate::yunxiao::Api::with_base(profile, "mock-token", &server.uri()).unwrap()
    }
    fn row(id: &str, kind: &str) -> Value {
        json!({"id":id,"categoryId":"Bug","space":{"id":"project-a"},"creator":{"id":"user-a"},"subject":"历史图片问题","description":format!("## 问题类型\n\n{kind}\n\n")})
    }
    fn state(server: &MockServer, root: &std::path::Path) -> crate::commands::AppState {
        let mut state = crate::commands::AppState::new(root.into()).unwrap();
        state.api_base = server.uri();
        *state.profile.lock().unwrap() = api(server).profile;
        *state.token.lock().unwrap() = Some("mock-token".into());
        *state.user.lock().unwrap() = Some(crate::workspace::UserIdentity {
            id: "user-a".into(),
            ..Default::default()
        });
        state
    }
    #[test]
    fn filters_intersect_and_empty_selections_do_not_narrow_results() {
        let row = json!({"id":"bug-a","workitemType":{"id":"type-a"},"assignedTo":{"id":"user-a"},"sprint":{"id":"sprint-a"},"verifier":{"id":"user-b"},"participants":[{"id":"user-c"}],"trackers":[{"id":"user-d"}],"customFieldValues":[{"fieldId":"priority","values":[{"identifier":"p1"}]},{"fieldId":"severity","values":[{"identifier":"s2"}]}],"subject":"图片恢复闪退","description":"## 问题类型\n\n崩溃\n\n"});
        let mut filters = HistoryFilters {
            kind: vec!["崩溃".into()],
            workitem_type_id: vec!["type-a".into()],
            assigned_to: vec!["user-a".into()],
            sprint: vec!["sprint-a".into()],
            verifier: vec!["user-b".into()],
            priority: vec!["p1".into()],
            severity: vec!["s1".into(), "s2".into()],
            participants: vec!["user-c".into()],
            trackers: vec!["user-d".into()],
            keyword: "图片".into(),
        };
        assert!(filters.matches(&row, None, &HashMap::new()));
        filters.trackers = vec!["other-user".into()];
        assert!(!filters.matches(&row, None, &HashMap::new()));
        assert!(HistoryFilters::default().matches(&json!({}), None, &HashMap::new()));
    }
    #[test]
    fn old_cloud_records_use_local_kind_or_explicit_unrecorded_value() {
        let row = json!({"id":"old-bug","description":"unstructured text mentioning UI"});
        let filter = HistoryFilters {
            kind: vec!["UI".into()],
            ..Default::default()
        };
        assert!(!filter.matches(&row, None, &HashMap::new()));
        assert!(filter.matches(
            &row,
            None,
            &HashMap::from([("old-bug".into(), "UI".into())])
        ));
        assert!(HistoryFilters {
            kind: vec!["未记录".into()],
            ..Default::default()
        }
        .matches(&row, None, &HashMap::new()));
    }
    #[test]
    fn conditions_keep_creator_and_only_use_documented_query_shapes() {
        let filters = HistoryFilters {
            assigned_to: vec!["member-a".into()],
            keyword: "  图片  ".into(),
            severity: vec!["s1".into()],
            ..Default::default()
        };
        let c = filters.conditions("user-a");
        assert_eq!(c["conditionGroups"][0].as_array().unwrap().len(), 3);
        assert_eq!(c["conditionGroups"][0][0]["fieldIdentifier"], "creator");
        assert_eq!(c["conditionGroups"][0][1]["value"], json!(["member-a"]));
        assert_eq!(c["conditionGroups"][0][2]["value"], json!(["图片"]));
        assert!(HistoryFilters {
            sprint: vec!["../project".into()],
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(HistoryFilters {
            kind: vec!["invented".into()],
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(HistoryFilters {
            keyword: "a".repeat(251),
            ..Default::default()
        }
        .validate()
        .is_err());
    }
    #[tokio::test]
    async fn sparse_results_use_bounded_server_cursor_without_crawling_all_pages() {
        let server = MockServer::start().await;
        for page in 1..=4 {
            Mock::given(method("POST"))
                .and(body_partial_json(
                    json!({"page":page,"perPage":50,"category":"Bug","spaceId":"project-a"}),
                ))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("x-total-pages", "10")
                        .set_body_json(json!([row(
                            &format!("bug-{page}"),
                            if page == 4 { "UI" } else { "功能" }
                        )])),
                )
                .expect(1)
                .mount(&server)
                .await;
        }
        let filters = HistoryFilters {
            kind: vec!["UI".into()],
            ..Default::default()
        };
        let api = api(&server);
        let first = api
            .history_filtered("user-a", 1, &filters, None, &HashMap::new())
            .await
            .unwrap();
        assert!(first.items.is_empty());
        assert!(first.has_more);
        assert_eq!(first.page, 3);
        assert_eq!(server.received_requests().await.unwrap().len(), 3);
        let next = api
            .history_filtered("user-a", first.page + 1, &filters, None, &HashMap::new())
            .await
            .unwrap();
        assert_eq!(next.items[0].id, "bug-4");
        assert_eq!(next.page, 4);
        assert!(next.has_more);
    }
    #[tokio::test]
    async fn unrelated_rows_are_rejected_before_filters_can_hide_them() {
        let server = MockServer::start().await;
        let mut wrong = row("bug-wrong", "功能");
        wrong["space"]["id"] = json!("other-project");
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([wrong])))
            .expect(1)
            .mount(&server)
            .await;
        let filters = HistoryFilters {
            kind: vec!["UI".into()],
            ..Default::default()
        };
        assert!(api(&server)
            .history_filtered("user-a", 1, &filters, None, &HashMap::new())
            .await
            .unwrap_err()
            .contains("范围不一致"));
    }
    #[tokio::test]
    async fn invalid_filters_and_different_main_project_never_send_http() {
        let server = MockServer::start().await;
        let root = tempfile::tempdir().unwrap();
        let state = state(&server, root.path());
        assert!(
            list_for_state(&state, 1, HistoryFilters::default(), Some("other-project"))
                .await
                .unwrap_err()
                .contains("项目已变化")
        );
        assert!(list_for_state(
            &state,
            1,
            HistoryFilters {
                trackers: vec!["../bad".into()],
                ..Default::default()
            },
            None
        )
        .await
        .is_err());
        assert!(server.received_requests().await.unwrap().is_empty());
    }
    #[tokio::test]
    async fn late_error_does_not_clear_the_replacement_account() {
        let server = MockServer::start().await;
        let root = tempfile::tempdir().unwrap();
        let state = state(&server, root.path());
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(401).set_delay(std::time::Duration::from_millis(100)),
            )
            .expect(1)
            .mount(&server)
            .await;
        let request = list_for_state(&state, 1, HistoryFilters::default(), Some("project-a"));
        let switch = async {
            for _ in 0..50 {
                if !server.received_requests().await.unwrap().is_empty() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            let mut user = state.user.lock().unwrap();
            state.auth_epoch.fetch_add(1, Ordering::SeqCst);
            *user = Some(crate::workspace::UserIdentity {
                id: "user-b".into(),
                ..Default::default()
            });
        };
        let (result, ()) = tokio::join!(request, switch);
        assert!(result.unwrap_err().contains("账号或主页面项目已变化"));
        assert_eq!(state.user.lock().unwrap().as_ref().unwrap().id, "user-b");
    }
    #[tokio::test]
    async fn late_success_after_project_round_trip_is_discarded() {
        let server = MockServer::start().await;
        let root = tempfile::tempdir().unwrap();
        let state = state(&server, root.path());
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(std::time::Duration::from_millis(100))
                    .set_body_json(json!([row("bug-a", "UI")])),
            )
            .expect(1)
            .mount(&server)
            .await;
        let request = list_for_state(&state, 1, HistoryFilters::default(), Some("project-a"));
        let switch = async {
            for _ in 0..50 {
                if !server.received_requests().await.unwrap().is_empty() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            state.workspace_epoch.fetch_add(2, Ordering::SeqCst);
        };
        let (result, ()) = tokio::join!(request, switch);
        assert!(result.unwrap_err().contains("项目已变化"));
    }
    #[test]
    fn custom_ids_and_native_role_aliases_are_matched_using_metadata() {
        let mut metadata: Metadata =
            serde_json::from_str(include_str!("../../tests/fixtures/metadata.json")).unwrap();
        metadata
            .profile
            .field_mappings
            .insert("verify-alias".into(), "verifier".into());
        for (id, name) in [
            ("priority-alias", "优先级"),
            ("severity-alias", "严重程度"),
            ("verify-alias", "验证者"),
        ] {
            metadata.fields.push(crate::model::Field {
                id: id.into(),
                name: name.into(),
                format: "list".into(),
                field_type: "CustomField".into(),
                required: false,
                show_when_create: true,
                option_error: None,
                options: vec![],
            });
        }
        let row = json!({"customFieldValues":[{"fieldId":"priority-alias","values":[{"identifier":"p1"}]},{"fieldId":"severity-alias","values":[{"identifier":"s2"}]},{"fieldId":"verify-alias","values":[{"identifier":"user-a"}]}]});
        let filters = HistoryFilters {
            priority: vec!["p1".into()],
            severity: vec!["s2".into()],
            verifier: vec!["user-a".into()],
            ..Default::default()
        };
        assert!(!filters.matches(&row, None, &HashMap::new()));
        assert!(filters.matches(&row, Some(&metadata), &HashMap::new()));
    }
    #[test]
    fn future_reports_store_kind_as_a_structured_section_for_cloud_history() {
        let draft = json!({"kind":"UI","title":"错位"});
        let description = crate::evidence::report(&draft, &crate::workspace::empty_profile());
        assert!(HistoryFilters {
            kind: vec!["UI".into()],
            ..Default::default()
        }
        .matches(&json!({"description":description}), None, &HashMap::new()));
    }
}
