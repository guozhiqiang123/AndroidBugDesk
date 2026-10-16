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

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectBugContentFormat {
    Markdown,
    Richtext,
    Plain,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBugCustomField {
    pub id: String,
    pub name: String,
    pub format: String,
    pub values: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBugAttachment {
    pub id: String,
    pub file_id: String,
    pub name: String,
    pub suffix: String,
    pub size: u64,
    pub creator: Option<OptionItem>,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBugComment {
    pub id: String,
    pub parent_id: String,
    pub author: Option<OptionItem>,
    pub content: String,
    pub format: ProjectBugContentFormat,
    pub created_at: String,
    pub updated_at: String,
    pub pinned: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBugActivity {
    pub id: String,
    pub event_type: String,
    pub action_type: String,
    pub actor: Option<OptionItem>,
    pub property_name: String,
    pub old_values: Vec<String>,
    pub new_values: Vec<String>,
    pub related_resource_id: String,
    pub related_resource_type: String,
    pub occurred_at: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBugSectionWarning {
    pub section: &'static str,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBugDetail {
    #[serde(flatten)]
    pub item: ProjectBugItem,
    pub description: String,
    pub format: ProjectBugContentFormat,
    pub modifier: Option<OptionItem>,
    pub verifier: Option<OptionItem>,
    pub participants: Vec<OptionItem>,
    pub trackers: Vec<OptionItem>,
    pub sprint: Option<OptionItem>,
    pub status_updated_at: String,
    pub archived: bool,
    pub custom_fields: Vec<ProjectBugCustomField>,
    pub attachments: Vec<ProjectBugAttachment>,
    pub comments: Vec<ProjectBugComment>,
    pub activities: Vec<ProjectBugActivity>,
    pub section_warnings: Vec<ProjectBugSectionWarning>,
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

fn option_list(value: &Value, path: &str) -> Result<Vec<OptionItem>> {
    value[path]
        .as_array()
        .into_iter()
        .flatten()
        .map(|entry| {
            let id = string(entry, "id");
            crate::yunxiao::encode_id(&id).map_err(|_| "项目 Bug 人员 ID 不正确")?;
            let display = string(entry, "displayName");
            let name = if display.is_empty() {
                string(entry, "name")
            } else {
                display
            };
            Ok(OptionItem {
                name: if name.is_empty() { id.clone() } else { name },
                id,
            })
        })
        .collect()
}

fn content_format(value: &str) -> ProjectBugContentFormat {
    match value.trim().to_ascii_uppercase().as_str() {
        "MARKDOWN" => ProjectBugContentFormat::Markdown,
        "RICHTEXT" => ProjectBugContentFormat::Richtext,
        _ => ProjectBugContentFormat::Plain,
    }
}

fn display_value(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => (!value.trim().is_empty()).then(|| value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(if *value { "是" } else { "否" }.into()),
        Value::Object(_) => ["displayValue", "name", "value", "identifier"]
            .iter()
            .map(|key| string(value, key))
            .find(|value| !value.trim().is_empty()),
        _ => None,
    }
}

fn display_values(value: &Value, path: &str) -> Vec<String> {
    value[path]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(display_value)
        .collect()
}

fn custom_fields(value: &Value) -> Vec<ProjectBugCustomField> {
    value["customFieldValues"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|field| {
            let id = string(field, "fieldId");
            let name = string(field, "fieldName");
            if id.is_empty() && name.is_empty() {
                return None;
            }
            Some(ProjectBugCustomField {
                id,
                name,
                format: string(field, "fieldFormat"),
                values: display_values(field, "values"),
            })
        })
        .collect()
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

fn map_detail(value: &Value, project_id: &str) -> Result<ProjectBugDetail> {
    Ok(ProjectBugDetail {
        item: item(value, project_id)?,
        description: string(value, "description"),
        format: content_format(&string(value, "formatType")),
        modifier: option_at(value, "modifier")?,
        verifier: option_at(value, "verifier")?,
        participants: option_list(value, "participants")?,
        trackers: option_list(value, "trackers")?,
        sprint: option_at(value, "sprint")?,
        status_updated_at: string(value, "updateStatusAt"),
        archived: string(value, "logicalStatus").eq_ignore_ascii_case("archived"),
        custom_fields: custom_fields(value),
        attachments: vec![],
        comments: vec![],
        activities: vec![],
        section_warnings: vec![],
    })
}

fn map_attachments(values: Vec<Value>) -> Result<Vec<ProjectBugAttachment>> {
    values
        .into_iter()
        .map(|value| {
            let id = string(&value, "id");
            crate::yunxiao::encode_id(&id).map_err(|_| "项目 Bug 附件 ID 不正确")?;
            let file_id = string(&value, "fileId");
            if !file_id.is_empty() {
                crate::yunxiao::encode_id(&file_id).map_err(|_| "项目 Bug 附件文件 ID 不正确")?;
            }
            Ok(ProjectBugAttachment {
                id,
                file_id,
                name: {
                    let name = string(&value, "fileName");
                    if name.is_empty() {
                        string(&value, "name")
                    } else {
                        name
                    }
                },
                suffix: string(&value, "suffix"),
                size: value["size"]
                    .as_u64()
                    .or_else(|| string(&value, "size").parse().ok())
                    .unwrap_or_default(),
                creator: option_at(&value, "creator")?,
                created_at: string(&value, "gmtCreate"),
            })
        })
        .collect()
}

fn map_comments(values: Vec<Value>) -> Result<Vec<ProjectBugComment>> {
    let mut comments = values
        .into_iter()
        .map(|value| {
            let id = string(&value, "id");
            crate::yunxiao::encode_id(&id).map_err(|_| "项目 Bug 评论 ID 不正确")?;
            Ok(ProjectBugComment {
                id,
                parent_id: string(&value, "parentId"),
                author: option_at(&value, "user")?,
                content: string(&value, "content"),
                format: content_format(&string(&value, "contentFormat")),
                created_at: string(&value, "gmtCreate"),
                updated_at: string(&value, "gmtModified"),
                pinned: value["top"].as_bool().unwrap_or(false),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    comments.sort_by(|left, right| left.created_at.cmp(&right.created_at));
    Ok(comments)
}

fn event_id(value: &Value) -> String {
    value["eventId"]
        .as_i64()
        .map(|id| id.to_string())
        .unwrap_or_else(|| string(value, "eventId"))
}

fn map_activities(values: Vec<Value>) -> Result<Vec<ProjectBugActivity>> {
    let mut activities = values
        .into_iter()
        .map(|value| {
            let id = event_id(&value);
            if id.is_empty() {
                return Err("项目 Bug 动态缺少事件 ID".into());
            }
            Ok(ProjectBugActivity {
                id,
                event_type: string(&value, "eventType"),
                action_type: string(&value, "actionType"),
                actor: option_at(&value, "operator")?,
                property_name: string(&value, "property.propertyName"),
                old_values: display_values(&value, "oldValue"),
                new_values: display_values(&value, "newValue"),
                related_resource_id: string(&value, "relatedResource.resourceId"),
                related_resource_type: string(&value, "relatedResource.resourceType"),
                occurred_at: string(&value, "eventTime"),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    activities.sort_by(|left, right| right.occurred_at.cmp(&left.occurred_at));
    Ok(activities)
}

fn section_warning(
    error: String,
    section: &'static str,
    label: &str,
) -> Result<ProjectBugSectionWarning> {
    if error.contains("[授权失效]") {
        Err(error)
    } else {
        Ok(ProjectBugSectionWarning {
            section,
            message: format!("{label}读取失败：{error}"),
        })
    }
}

async fn detail_for_state(
    state: &AppState,
    id: &str,
    project_id: Option<&str>,
) -> Result<ProjectBugDetail> {
    crate::yunxiao::encode_id(id).map_err(|_| "项目 Bug ID 不正确")?;
    let request = crate::workspace::read_scope(state, project_id)?;
    let result = async {
        let value = request.api.get(id).await?;
        let mut detail = map_detail(&value, &request.api.profile.project_id)?;
        let (attachments, comments, activities) = tokio::join!(
            request.api.attachments(id),
            request.api.comments(id),
            request.api.activities(id)
        );
        match attachments.and_then(map_attachments) {
            Ok(values) => detail.attachments = values,
            Err(error) => {
                detail
                    .section_warnings
                    .push(section_warning(error, "attachments", "附件")?)
            }
        }
        match comments.and_then(map_comments) {
            Ok(values) => detail.comments = values,
            Err(error) => detail
                .section_warnings
                .push(section_warning(error, "comments", "评论")?),
        }
        match activities.and_then(map_activities) {
            Ok(values) => detail.activities = values,
            Err(error) => {
                detail
                    .section_warnings
                    .push(section_warning(error, "activities", "动态")?)
            }
        }
        Ok(detail)
    }
    .await;
    crate::workspace::ensure_read_scope(state, &request)?;
    crate::workspace::observed_error(state, result)
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

#[tauri::command]
pub async fn project_bug_detail(
    id: String,
    project_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<ProjectBugDetail> {
    detail_for_state(&state, &id, project_id.as_deref()).await
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

    #[tokio::test]
    async fn detail_aggregates_documented_fields_comments_attachments_and_activities() {
        let server = MockServer::start().await;
        let base = "/oapi/v1/projex/organizations/org-a/workitems/bug-a";
        Mock::given(method("GET"))
            .and(path(base))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id":"bug-a","serialNumber":"SKII-614","subject":"清理功能异常",
                "categoryId":"Bug","space":{"id":"project-a"},
                "description":"## 复现\n点击清理","formatType":"MARKDOWN",
                "status":{"id":"todo","displayName":"待处理"},
                "assignedTo":{"id":"owner-a","name":"负责人"},
                "creator":{"id":"creator-a","name":"创建者"},
                "modifier":{"id":"modifier-a","name":"修改者"},
                "verifier":{"id":"verifier-a","name":"验证者"},
                "participants":[{"id":"participant-a","name":"参与者"}],
                "trackers":[{"id":"tracker-a","name":"抄送者"}],
                "sprint":{"id":"sprint-a","name":"迭代 A"},
                "labels":[{"id":"tag-a","name":"Android"}],
                "customFieldValues":[{
                    "fieldId":"priority","fieldName":"优先级","fieldFormat":"list",
                    "values":[{"identifier":"high","displayValue":"高"}]
                }],
                "logicalStatus":"normal","updateStatusAt":"2026-10-16 12:00:00",
                "gmtCreate":"2026-10-16 10:00:00","gmtModified":"2026-10-16 11:00:00"
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("{base}/attachments")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "id":"attachment-a","fileId":"file-a","fileName":"evidence.png",
                "suffix":".png","size":1024,"creator":{"id":"creator-a","name":"创建者"},
                "gmtCreate":"2026-10-16 10:05:00","url":"https://example.test/evidence.png"
            }])))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("{base}/comments")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "id":"comment-a","parentId":"","content":"已复现","contentFormat":"RICHTEXT",
                "gmtCreate":"2026-10-16 10:10:00","gmtModified":"2026-10-16 10:11:00",
                "top":true,"user":{"id":"owner-a","name":"负责人"}
            }])))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("{base}/activities")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "eventId":42,"eventTime":"2026-10-16 10:20:00",
                "eventType":"workitem.updated","actionType":"updated",
                "operator":{"id":"owner-a","name":"负责人"},
                "property":{"propertyId":"priority","propertyName":"优先级","propertyType":"Field"},
                "oldValue":[{"identifier":"middle","displayValue":"中"}],
                "newValue":[{"identifier":"high","displayValue":"高"}],
                "relatedResource":{"resourceId":"","resourceType":""}
            }])))
            .expect(1)
            .mount(&server)
            .await;
        let root = tempfile::tempdir().unwrap();
        let state = state(&server, root.path());
        let result = detail_for_state(&state, "bug-a", Some("project-a"))
            .await
            .unwrap();
        assert_eq!(result.item.identifier, "SKII-614");
        assert!(matches!(result.format, ProjectBugContentFormat::Markdown));
        assert_eq!(result.custom_fields[0].values, ["高"]);
        assert_eq!(result.attachments[0].name, "evidence.png");
        assert!(matches!(
            result.comments[0].format,
            ProjectBugContentFormat::Richtext
        ));
        assert_eq!(result.activities[0].id, "42");
        assert_eq!(result.activities[0].old_values, ["中"]);
        assert!(result.section_warnings.is_empty());
    }

    #[tokio::test]
    async fn detail_keeps_primary_data_when_one_optional_section_fails() {
        let server = MockServer::start().await;
        let base = "/oapi/v1/projex/organizations/org-a/workitems/bug-a";
        Mock::given(method("GET"))
            .and(path(base))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id":"bug-a","serialNumber":"SKII-614","subject":"清理功能异常",
                "categoryId":"Bug","space":{"id":"project-a"},"labels":[]
            })))
            .expect(1)
            .mount(&server)
            .await;
        for endpoint in ["attachments", "activities"] {
            Mock::given(method("GET"))
                .and(path(format!("{base}/{endpoint}")))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
                .expect(1)
                .mount(&server)
                .await;
        }
        Mock::given(method("GET"))
            .and(path(format!("{base}/comments")))
            .respond_with(ResponseTemplate::new(403))
            .expect(1)
            .mount(&server)
            .await;
        let root = tempfile::tempdir().unwrap();
        let state = state(&server, root.path());
        let result = detail_for_state(&state, "bug-a", Some("project-a"))
            .await
            .unwrap();
        assert_eq!(result.item.title, "清理功能异常");
        assert!(result.comments.is_empty());
        assert_eq!(result.section_warnings.len(), 1);
        assert_eq!(result.section_warnings[0].section, "comments");
        assert!(state.user.lock().unwrap().is_some());
    }

    #[tokio::test]
    async fn detail_rejects_cross_project_workitem_before_reading_optional_sections() {
        let server = MockServer::start().await;
        let base = "/oapi/v1/projex/organizations/org-a/workitems/bug-a";
        Mock::given(method("GET"))
            .and(path(base))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id":"bug-a","serialNumber":"OTHER-1","subject":"其他项目",
                "categoryId":"Bug","space":{"id":"project-b"},"labels":[]
            })))
            .expect(1)
            .mount(&server)
            .await;
        let root = tempfile::tempdir().unwrap();
        let state = state(&server, root.path());
        assert!(detail_for_state(&state, "bug-a", Some("project-a"))
            .await
            .unwrap_err()
            .contains("范围不一致"));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}
