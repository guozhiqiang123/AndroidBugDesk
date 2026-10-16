use crate::{
    commands::AppState,
    model::{string, Metadata, OptionItem, Result},
    yunxiao::{WorkitemSearchPage, WorkitemSearchQuery},
};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use tauri::State;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectBugSort {
    #[default]
    CreatedDesc,
    CreatedAsc,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct ProjectBugQuery {
    pub page: usize,
    pub per_page: usize,
    pub keyword: String,
    pub status_ids: Vec<String>,
    pub assigned_to_ids: Vec<String>,
    pub creator_ids: Vec<String>,
    pub tag_ids: Vec<String>,
    pub created_from: String,
    pub created_to: String,
    pub sort: ProjectBugSort,
}

impl Default for ProjectBugQuery {
    fn default() -> Self {
        Self {
            page: 1,
            per_page: 50,
            keyword: String::new(),
            status_ids: vec![],
            assigned_to_ids: vec![],
            creator_ids: vec![],
            tag_ids: vec![],
            created_from: String::new(),
            created_to: String::new(),
            sort: ProjectBugSort::CreatedDesc,
        }
    }
}

impl ProjectBugQuery {
    fn validate(&self) -> Result<()> {
        if !(1..=1000).contains(&self.page)
            || !(1..=50).contains(&self.per_page)
            || self.keyword.chars().count() > 250
        {
            return Err("项目 Bug 查询条件不正确".into());
        }
        for values in [
            &self.status_ids,
            &self.assigned_to_ids,
            &self.creator_ids,
            &self.tag_ids,
        ] {
            if values.len() > 100 {
                return Err("项目 Bug 筛选选项过多".into());
            }
            for value in values {
                crate::yunxiao::encode_id(value)?;
            }
        }
        if self.created_from.is_empty() != self.created_to.is_empty() {
            return Err("创建时间需要同时填写开始和结束日期".into());
        }
        if !self.created_from.is_empty() {
            let from = parse_date(&self.created_from)?;
            let to = parse_date(&self.created_to)?;
            if from > to {
                return Err("创建时间的开始日期不能晚于结束日期".into());
            }
        }
        Ok(())
    }

    fn conditions(&self) -> Value {
        let mut group = Vec::new();
        if !self.keyword.trim().is_empty() {
            group.push(json!({"fieldIdentifier":"subject","operator":"CONTAINS","value":[self.keyword.trim()],"toValue":null,"className":"string","format":"input"}));
        }
        if !self.status_ids.is_empty() {
            group.push(json!({"fieldIdentifier":"status","operator":"CONTAINS","value":self.status_ids,"toValue":null,"className":"status","format":"list"}));
        }
        if !self.assigned_to_ids.is_empty() {
            group.push(json!({"fieldIdentifier":"assignedTo","operator":"CONTAINS","value":self.assigned_to_ids,"toValue":null,"className":"user","format":"list"}));
        }
        if !self.creator_ids.is_empty() {
            group.push(json!({"fieldIdentifier":"creator","operator":"CONTAINS","value":self.creator_ids,"toValue":null,"className":"user","format":"list"}));
        }
        if !self.tag_ids.is_empty() {
            group.push(json!({"fieldIdentifier":"tag","operator":"CONTAINS","value":self.tag_ids,"toValue":null,"className":"tag","format":"multiList"}));
        }
        if !self.created_from.is_empty() {
            group.push(json!({
                "fieldIdentifier":"gmtCreate",
                "operator":"BETWEEN",
                "value":[format!("{} 00:00:00", self.created_from)],
                "toValue":format!("{} 23:59:59", self.created_to),
                "className":"dateTime",
                "format":"input"
            }));
        }
        json!({"conditionGroups":[group]})
    }

    fn order(&self) -> &'static str {
        match self.sort {
            ProjectBugSort::CreatedDesc => "desc",
            ProjectBugSort::CreatedAsc => "asc",
        }
    }
}

fn parse_date(value: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| "创建时间日期格式不正确".into())
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBugItem {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub status: Option<OptionItem>,
    pub assigned_to: Option<OptionItem>,
    pub creator: Option<OptionItem>,
    pub labels: Vec<OptionItem>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBugPage {
    pub items: Vec<ProjectBugItem>,
    pub page: usize,
    pub per_page: usize,
    pub total: Option<usize>,
    pub total_pages: Option<usize>,
    pub has_more: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBugFacets {
    pub statuses: Vec<OptionItem>,
    pub members: Vec<OptionItem>,
    pub tags: Vec<OptionItem>,
    pub warnings: Vec<String>,
}

fn option_at(value: &Value, path: &str) -> Result<Option<OptionItem>> {
    let id = string(value, &format!("{path}.id"));
    if id.is_empty() {
        return Ok(None);
    }
    crate::yunxiao::encode_id(&id)?;
    let display = string(value, &format!("{path}.displayName"));
    let name = if display.is_empty() {
        string(value, &format!("{path}.name"))
    } else {
        display
    };
    Ok(Some(OptionItem {
        name: if name.is_empty() { id.clone() } else { name },
        id,
    }))
}

fn item(row: &Value, project_id: &str) -> Result<ProjectBugItem> {
    let id = string(row, "id");
    crate::yunxiao::encode_id(&id).map_err(|_| "项目 Bug 缺少有效工作项 ID")?;
    if string(row, "space.id") != project_id || string(row, "categoryId") != "Bug" {
        return Err("项目 Bug 结果与当前项目/Bug范围不一致，停止展示".into());
    }
    let mut labels = Vec::new();
    for label in row["labels"].as_array().into_iter().flatten() {
        let label_id = string(label, "id");
        crate::yunxiao::encode_id(&label_id).map_err(|_| "项目 Bug 标签 ID 不正确")?;
        labels.push(OptionItem {
            id: label_id.clone(),
            name: if string(label, "name").is_empty() {
                label_id
            } else {
                string(label, "name")
            },
        });
    }
    Ok(ProjectBugItem {
        id,
        identifier: string(row, "serialNumber"),
        title: string(row, "subject"),
        status: option_at(row, "status")?,
        assigned_to: option_at(row, "assignedTo")?,
        creator: option_at(row, "creator")?,
        labels,
        created_at: string(row, "gmtCreate"),
        updated_at: string(row, "gmtModified"),
    })
}

fn map_page(page: WorkitemSearchPage, project_id: &str) -> Result<ProjectBugPage> {
    let items = page
        .rows
        .iter()
        .map(|row| item(row, project_id))
        .collect::<Result<Vec<_>>>()?;
    Ok(ProjectBugPage {
        items,
        page: page.page,
        per_page: page.per_page,
        total: page.total,
        total_pages: page.total_pages,
        has_more: page.has_more,
    })
}

async fn list_for_state(
    state: &AppState,
    query: ProjectBugQuery,
    project_id: Option<&str>,
) -> Result<ProjectBugPage> {
    query.validate()?;
    let request = crate::workspace::read_scope(state, project_id)?;
    let result = request
        .api
        .search_workitems(WorkitemSearchQuery {
            page: query.page,
            per_page: query.per_page,
            order_by: "gmtCreate",
            sort: query.order(),
            conditions: query.conditions(),
        })
        .await
        .and_then(|page| map_page(page, &request.api.profile.project_id));
    crate::workspace::ensure_read_scope(state, &request)?;
    crate::workspace::observed_error(state, result)
}

fn unique(options: impl IntoIterator<Item = OptionItem>) -> Vec<OptionItem> {
    let mut ids = HashSet::new();
    options
        .into_iter()
        .filter(|option| ids.insert(option.id.clone()))
        .collect()
}

async fn facets_for_state(state: &AppState, project_id: Option<&str>) -> Result<ProjectBugFacets> {
    let request = crate::workspace::read_scope(state, project_id)?;
    let metadata = state.metadata.lock().unwrap().clone().filter(|metadata| {
        metadata.profile.organization_id == request.api.profile.organization_id
            && metadata.profile.project_id == request.api.profile.project_id
    });
    let result = facets(&request.api, metadata.as_ref()).await;
    crate::workspace::ensure_read_scope(state, &request)?;
    crate::workspace::observed_error(state, result)
}

async fn facets(
    api: &crate::yunxiao::Api,
    metadata: Option<&Metadata>,
) -> Result<ProjectBugFacets> {
    let mut warnings = Vec::new();
    let types = if let Some(metadata) = metadata {
        metadata.types.clone()
    } else {
        api.bug_types().await?
    };
    let members = if let Some(metadata) = metadata {
        metadata.members.clone()
    } else {
        api.project_members().await?
    };
    let mut tags = metadata
        .into_iter()
        .flat_map(|metadata| &metadata.fields)
        .filter(|field| {
            field.format == "tag"
                || crate::validation::native_target(field, &api.profile) == Some("labels")
        })
        .flat_map(|field| field.options.clone())
        .collect::<Vec<_>>();
    if tags.is_empty() {
        match api.project_labels().await {
            Ok(values) => tags = values,
            Err(error) if error.contains("[授权失效]") => return Err(error),
            Err(_) => warnings.push("标签筛选项未能读取，其他条件仍可使用".into()),
        }
    }
    let mut statuses = Vec::new();
    for workitem_type in types {
        match api.workitem_statuses(&workitem_type.id).await {
            Ok(values) => statuses.extend(values),
            Err(error) if error.contains("[授权失效]") => return Err(error),
            Err(_) => warnings.push(format!("{} 的状态筛选项未能读取", workitem_type.name)),
        }
    }
    Ok(ProjectBugFacets {
        statuses: unique(statuses),
        members: unique(members),
        tags: unique(tags),
        warnings,
    })
}

#[tauri::command]
pub async fn list_project_bugs(
    query: ProjectBugQuery,
    project_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<ProjectBugPage> {
    list_for_state(&state, query, project_id.as_deref()).await
}

#[tauri::command]
pub async fn project_bug_facets(
    project_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<ProjectBugFacets> {
    facets_for_state(&state, project_id.as_deref()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        matchers::{body_partial_json, method, path},
        Mock, MockServer, ResponseTemplate,
    };

    fn state(server: &MockServer, root: &std::path::Path) -> AppState {
        let mut state = AppState::new(root.into()).unwrap();
        state.api_base = server.uri();
        let mut profile = crate::workspace::empty_profile();
        profile.organization_id = "org-a".into();
        profile.project_id = "project-a".into();
        profile.name = "项目 A".into();
        *state.profile.lock().unwrap() = profile;
        *state.token.lock().unwrap() = Some("mock-token".into());
        *state.user.lock().unwrap() = Some(crate::workspace::UserIdentity {
            id: "user-a".into(),
            ..Default::default()
        });
        state
    }

    #[test]
    fn query_uses_documented_filter_shapes_without_personal_history_scope() {
        let query = ProjectBugQuery {
            keyword: " 广告 ".into(),
            status_ids: vec!["todo".into()],
            assigned_to_ids: vec!["owner-a".into()],
            creator_ids: vec!["creator-a".into()],
            tag_ids: vec!["tag-a".into()],
            created_from: "2026-10-01".into(),
            created_to: "2026-10-16".into(),
            ..Default::default()
        };
        query.validate().unwrap();
        let conditions = query.conditions();
        let group = conditions["conditionGroups"][0].as_array().unwrap();
        assert_eq!(group.len(), 6);
        assert_eq!(group[0]["fieldIdentifier"], "subject");
        assert_eq!(group[4]["fieldIdentifier"], "tag");
        assert_eq!(group[5]["toValue"], "2026-10-16 23:59:59");
        assert!(!group.iter().any(|value| {
            value["fieldIdentifier"] == "creator" && value["value"] == json!(["user-a"])
        }));
        assert!(ProjectBugQuery {
            created_from: "2026-10-17".into(),
            created_to: "2026-10-16".into(),
            ..Default::default()
        }
        .validate()
        .is_err());
    }

    #[tokio::test]
    async fn lists_all_project_bugs_with_server_pagination_and_total() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oapi/v1/projex/organizations/org-a/workitems:search"))
            .and(body_partial_json(json!({
                "category":"Bug","spaceId":"project-a","page":1,"perPage":50,
                "orderBy":"gmtCreate","sort":"desc"
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-page", "1")
                    .insert_header("x-per-page", "50")
                    .insert_header("x-total", "61")
                    .insert_header("x-total-pages", "2")
                    .set_body_json(json!([{
                        "id":"bug-a","serialNumber":"SKII-614","subject":"清理功能异常",
                        "categoryId":"Bug","space":{"id":"project-a"},
                        "status":{"id":"todo","displayName":"待处理"},
                        "assignedTo":{"id":"owner-b","name":"负责人"},
                        "creator":{"id":"creator-b","name":"其他创建者"},
                        "labels":[{"id":"tag-a","name":"Android"}],
                        "gmtCreate":"2026-10-16 10:00:00","gmtModified":"2026-10-16 11:00:00"
                    }])),
            )
            .expect(1)
            .mount(&server)
            .await;
        let root = tempfile::tempdir().unwrap();
        let state = state(&server, root.path());
        let result = list_for_state(&state, ProjectBugQuery::default(), Some("project-a"))
            .await
            .unwrap();
        assert_eq!(result.total, Some(61));
        assert_eq!(result.total_pages, Some(2));
        assert!(result.has_more);
        assert_eq!(result.items[0].creator.as_ref().unwrap().id, "creator-b");
        assert_eq!(result.items[0].identifier, "SKII-614");
        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert!(body.get("conditions").is_none());
    }

    #[tokio::test]
    async fn facets_merge_workflow_statuses_and_project_reference_data() {
        let server = MockServer::start().await;
        let base = "/oapi/v1/projex/organizations/org-a/projects/project-a";
        Mock::given(method("GET"))
            .and(path(format!("{base}/workitemTypes")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"id":"type-a","name":"缺陷"},{"id":"type-b","name":"线上缺陷"}
            ])))
            .expect(1)
            .mount(&server)
            .await;
        for (id, status_id, name) in [("type-a", "todo", "待处理"), ("type-b", "todo", "待处理")]
        {
            Mock::given(method("GET"))
                .and(path(format!("{base}/workitemTypes/{id}/workflows")))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "statuses":[{"id":status_id,"displayName":name}]
                })))
                .expect(1)
                .mount(&server)
                .await;
        }
        Mock::given(method("GET"))
            .and(path(format!("{base}/members")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"userId":"user-a","userName":"测试人员"}
            ])))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("{base}/labels")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"id":"tag-a","name":"Android"}
            ])))
            .expect(1)
            .mount(&server)
            .await;
        let root = tempfile::tempdir().unwrap();
        let state = state(&server, root.path());
        let result = facets_for_state(&state, Some("project-a")).await.unwrap();
        assert_eq!(result.statuses.len(), 1);
        assert_eq!(result.members[0].name, "测试人员");
        assert_eq!(result.tags[0].name, "Android");
        assert!(result.warnings.is_empty());
    }
}
