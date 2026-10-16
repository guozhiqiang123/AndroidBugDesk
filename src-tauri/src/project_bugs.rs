use crate::{
    commands::AppState,
    model::{string, Metadata, OptionItem, Result},
    yunxiao::{WorkitemSearchPage, WorkitemSearchQuery},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    path::Path,
    time::{Duration, SystemTime},
};
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
pub struct ProjectBugInlineFile {
    pub file_id: String,
    pub name: String,
    pub suffix: String,
    pub size: u64,
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
    pub inline_files: Vec<ProjectBugInlineFile>,
    pub attachments: Vec<ProjectBugAttachment>,
    pub comments: Vec<ProjectBugComment>,
    pub activities: Vec<ProjectBugActivity>,
    pub section_warnings: Vec<ProjectBugSectionWarning>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBugAttachmentPreview {
    pub kind: &'static str,
    pub name: String,
    pub mime: String,
    pub data: String,
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

fn content_payload(
    value: &Value,
    content_path: &str,
    format_path: &str,
) -> (String, ProjectBugContentFormat) {
    let declared = content_format(&string(value, format_path));
    let source = &value[content_path];
    if let Some(payload) = parsed_content_payload(source) {
        if let Some(content) = payload["htmlValue"].as_str() {
            return (content.to_string(), ProjectBugContentFormat::Richtext);
        }
        if let Some(content) = payload["markdownValue"].as_str() {
            return (content.to_string(), ProjectBugContentFormat::Markdown);
        }
        if let Some(content) = payload["textValue"].as_str() {
            return (content.to_string(), ProjectBugContentFormat::Plain);
        }
    }
    (source.as_str().unwrap_or_default().to_string(), declared)
}

fn parsed_content_payload(source: &Value) -> Option<Value> {
    let mut current = source.clone();
    for _ in 0..3 {
        match current {
            Value::Object(_) => return Some(current),
            Value::String(ref raw) => current = serde_json::from_str(raw).ok()?,
            _ => return None,
        }
    }
    current.is_object().then_some(current)
}

fn value_at<'a>(value: &'a Value, path: &str) -> &'a Value {
    path.split('.').fold(value, |current, part| &current[part])
}

fn timestamp_value(value: &Value, paths: &[&str]) -> String {
    for path in paths {
        let candidate = value_at(value, path);
        if let Some(text) = candidate.as_str().filter(|text| !text.trim().is_empty()) {
            return text.to_string();
        }
        let raw = candidate.as_i64().or_else(|| {
            candidate
                .as_u64()
                .and_then(|value| i64::try_from(value).ok())
        });
        if let Some(raw) = raw {
            let millis = if raw.unsigned_abs() >= 10_000_000_000 {
                raw
            } else {
                raw.saturating_mul(1_000)
            };
            if let Some(timestamp) = DateTime::<Utc>::from_timestamp_millis(millis) {
                return timestamp.to_rfc3339();
            }
        }
    }
    String::new()
}

fn file_identifier(source: &str) -> Option<String> {
    let from_url = reqwest::Url::parse(source).ok().and_then(|url| {
        url.query_pairs()
            .find(|(key, _)| key == "fileIdentifier")
            .map(|(_, value)| value.into_owned())
    });
    let value = from_url.or_else(|| {
        let marker = "fileIdentifier=";
        let start = source.find(marker)? + marker.len();
        let value = source[start..]
            .chars()
            .take_while(|value| value.is_ascii_alphanumeric() || matches!(value, '-' | '_'))
            .collect::<String>();
        (!value.is_empty()).then_some(value)
    })?;
    if crate::yunxiao::encode_id(&value).is_ok() {
        Some(value)
    } else {
        None
    }
}

fn file_suffix(name: &str, explicit: &str) -> String {
    if !explicit.trim().is_empty() {
        return explicit.trim().to_string();
    }
    Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| format!(".{value}"))
        .unwrap_or_default()
}

fn upsert_inline_file(files: &mut Vec<ProjectBugInlineFile>, incoming: ProjectBugInlineFile) {
    if let Some(current) = files
        .iter_mut()
        .find(|current| current.file_id == incoming.file_id)
    {
        if current.name == "正文图片" && incoming.name != "正文图片" {
            current.name = incoming.name;
        }
        if current.suffix.is_empty() {
            current.suffix = incoming.suffix;
        }
        if current.size == 0 {
            current.size = incoming.size;
        }
    } else {
        files.push(incoming);
    }
}

fn collect_inline_files(value: &Value, files: &mut Vec<ProjectBugInlineFile>) {
    match value {
        Value::Object(object) => {
            if let Some(source) = object.get("src").and_then(Value::as_str) {
                if let Some(file_id) = file_identifier(source) {
                    let name = object
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("正文图片")
                        .to_string();
                    let suffix = file_suffix(
                        &name,
                        object
                            .get("suffix")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                    );
                    let size = object
                        .get("size")
                        .and_then(Value::as_u64)
                        .or_else(|| {
                            object
                                .get("size")
                                .and_then(Value::as_str)
                                .and_then(|value| value.parse().ok())
                        })
                        .unwrap_or_default();
                    upsert_inline_file(
                        files,
                        ProjectBugInlineFile {
                            file_id,
                            name,
                            suffix,
                            size,
                        },
                    );
                }
            }
            for child in object.values() {
                collect_inline_files(child, files);
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_inline_files(child, files);
            }
        }
        Value::String(source) => {
            if let Some(file_id) = file_identifier(source) {
                upsert_inline_file(
                    files,
                    ProjectBugInlineFile {
                        file_id,
                        name: "正文图片".into(),
                        suffix: String::new(),
                        size: 0,
                    },
                );
            }
        }
        _ => {}
    }
}

fn inline_files_from_content(value: &Value, content_path: &str) -> Vec<ProjectBugInlineFile> {
    let source = &value[content_path];
    let mut files = Vec::new();
    if let Some(payload) = parsed_content_payload(source) {
        collect_inline_files(&payload, &mut files);
    } else {
        collect_inline_files(source, &mut files);
    }
    files
}

fn merge_inline_files(target: &mut Vec<ProjectBugInlineFile>, incoming: Vec<ProjectBugInlineFile>) {
    for file in incoming {
        upsert_inline_file(target, file);
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
        created_at: timestamp_value(row, &["gmtCreate", "createTime", "createdAt"]),
        updated_at: timestamp_value(row, &["gmtModified", "modifiedTime", "updatedAt"]),
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
    let (description, format) = content_payload(value, "description", "formatType");
    Ok(ProjectBugDetail {
        item: item(value, project_id)?,
        description,
        format,
        modifier: option_at(value, "modifier")?,
        verifier: option_at(value, "verifier")?,
        participants: option_list(value, "participants")?,
        trackers: option_list(value, "trackers")?,
        sprint: option_at(value, "sprint")?,
        status_updated_at: timestamp_value(
            value,
            &["updateStatusAt", "statusUpdateTime", "statusUpdatedAt"],
        ),
        archived: string(value, "logicalStatus").eq_ignore_ascii_case("archived"),
        custom_fields: custom_fields(value),
        inline_files: inline_files_from_content(value, "description"),
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
                created_at: timestamp_value(&value, &["gmtCreate", "createTime", "createdAt"]),
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
            let (content, format) = content_payload(&value, "content", "contentFormat");
            Ok(ProjectBugComment {
                id,
                parent_id: string(&value, "parentId"),
                author: option_at(&value, "user")?,
                content,
                format,
                created_at: timestamp_value(&value, &["gmtCreate", "createTime", "createdAt"]),
                updated_at: timestamp_value(&value, &["gmtModified", "modifiedTime", "updatedAt"]),
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
                occurred_at: timestamp_value(
                    &value,
                    &["eventTime", "gmtCreate", "createTime", "createdAt"],
                ),
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
        match comments {
            Ok(values) => {
                for value in &values {
                    merge_inline_files(
                        &mut detail.inline_files,
                        inline_files_from_content(value, "content"),
                    );
                }
                match map_comments(values) {
                    Ok(values) => detail.comments = values,
                    Err(error) => detail
                        .section_warnings
                        .push(section_warning(error, "comments", "评论")?),
                }
            }
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

const REMOTE_ATTACHMENT_LIMIT: u64 = 100 * 1024 * 1024;
const INLINE_IMAGE_LIMIT: usize = 12 * 1024 * 1024;
const INLINE_TEXT_LIMIT: usize = 2 * 1024 * 1024;
const REMOTE_PREVIEW_CACHE_LIMIT: u64 = 100 * 1024 * 1024;
const REMOTE_PREVIEW_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

fn safe_attachment_name(name: &str, suffix: &str) -> String {
    let mut safe = name
        .chars()
        .map(|value| {
            if value.is_control()
                || matches!(value, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
            {
                '_'
            } else {
                value
            }
        })
        .take(120)
        .collect::<String>();
    if safe.trim_matches(['.', ' ']).is_empty() {
        safe = "yunxiao-attachment".into();
    }
    if Path::new(&safe).extension().is_none() {
        let suffix = suffix.trim().trim_start_matches('.');
        if !suffix.is_empty()
            && suffix.len() <= 12
            && suffix.chars().all(|value| value.is_ascii_alphanumeric())
        {
            safe.push('.');
            safe.push_str(suffix);
        }
    }
    safe
}

fn dangerous_preview(bytes: &[u8], name: &str) -> bool {
    let extension = Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    [
        "exe",
        "com",
        "bat",
        "cmd",
        "ps1",
        "command",
        "sh",
        "desktop",
        "app",
        "lnk",
        "url",
        "msi",
        "vbs",
        "js",
        "py",
        "pyw",
        "rb",
        "pl",
        "jar",
        "hta",
        "scpt",
        "applescript",
        "bash",
        "zsh",
        "fish",
    ]
    .contains(&extension.as_str())
        || bytes.starts_with(b"MZ")
        || bytes.starts_with(b"\x7fELF")
        || bytes.starts_with(b"#!")
        || [
            b"\xfe\xed\xfa\xce".as_slice(),
            b"\xce\xfa\xed\xfe".as_slice(),
            b"\xfe\xed\xfa\xcf".as_slice(),
            b"\xcf\xfa\xed\xfe".as_slice(),
            b"\xca\xfe\xba\xbe".as_slice(),
            b"\xbe\xba\xfe\xca".as_slice(),
        ]
        .iter()
        .any(|magic| bytes.starts_with(magic))
}

fn prune_preview_cache(root: &Path, incoming: u64) -> Result<()> {
    if incoming > REMOTE_PREVIEW_CACHE_LIMIT {
        return Err("附件超过 100 MB，不能在客户端预览".into());
    }
    fs::create_dir_all(root).map_err(|_| "无法建立云效附件临时目录")?;
    let now = SystemTime::now();
    let mut files = fs::read_dir(root)
        .map_err(|_| "无法读取云效附件临时目录")?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            metadata.is_file().then(|| {
                (
                    entry.path(),
                    metadata.len(),
                    metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                )
            })
        })
        .collect::<Vec<_>>();
    files.retain(|(path, _, modified)| {
        if now.duration_since(*modified).unwrap_or_default() > REMOTE_PREVIEW_MAX_AGE {
            let _ = fs::remove_file(path);
            false
        } else {
            true
        }
    });
    files.sort_by_key(|(_, _, modified)| *modified);
    let mut total = files.iter().map(|(_, size, _)| size).sum::<u64>();
    for (path, size, _) in files {
        if total.saturating_add(incoming) <= REMOTE_PREVIEW_CACHE_LIMIT {
            break;
        }
        if fs::remove_file(path).is_ok() {
            total = total.saturating_sub(size);
        }
    }
    if total.saturating_add(incoming) > REMOTE_PREVIEW_CACHE_LIMIT {
        return Err("云效附件临时目录空间不足，请稍后重试".into());
    }
    Ok(())
}

fn system_preview_attachment(
    state: &AppState,
    scope: &str,
    bug_id: &str,
    attachment_id: &str,
    metadata: &ProjectBugAttachment,
    bytes: &[u8],
) -> Result<crate::model::Attachment> {
    let root = state.store.root.join("remote-previews");
    prune_preview_cache(&root, bytes.len() as u64)?;
    let name = safe_attachment_name(&metadata.name, &metadata.suffix);
    let key = Sha256::digest(format!("{scope}:{bug_id}:{attachment_id}").as_bytes());
    let path = root.join(format!("{}-{name}", hex::encode(key)));
    fs::write(&path, bytes).map_err(|_| "无法保存云效附件临时副本")?;
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let (role, mime) = if extension == "zip" && bytes.starts_with(b"PK\x03\x04") {
        ("report", "application/zip")
    } else if ["mp4", "mov", "m4v"].contains(&extension.as_str()) {
        ("video", "video/mp4")
    } else {
        ("sample", "application/octet-stream")
    };
    Ok(crate::model::Attachment {
        id: attachment_id.into(),
        name,
        path: path.to_string_lossy().into_owned(),
        mime: mime.into(),
        size: bytes.len() as u64,
        sha256: hex::encode(Sha256::digest(bytes)),
        role: role.into(),
        source: "云效工作项附件临时预览".into(),
        coverage_start: String::new(),
        coverage_end: String::new(),
        confirmed: true,
        derived_from: None,
    })
}

async fn attachment_preview_for_state(
    state: &AppState,
    id: &str,
    attachment_id: &str,
    project_id: Option<&str>,
) -> Result<(
    ProjectBugAttachmentPreview,
    Option<crate::model::Attachment>,
)> {
    crate::yunxiao::encode_id(id).map_err(|_| "项目 Bug ID 不正确")?;
    crate::yunxiao::encode_id(attachment_id).map_err(|_| "项目 Bug 附件 ID 不正确")?;
    let request = crate::workspace::read_scope(state, project_id)?;
    let result = async {
        let detail = request.api.get(id).await?;
        item(&detail, &request.api.profile.project_id)?;
        let rows = request.api.attachments(id).await?;
        let row = rows
            .into_iter()
            .find(|row| string(row, "id") == attachment_id)
            .ok_or("云效附件不存在或已被移除")?;
        let metadata = map_attachments(vec![row.clone()])?
            .pop()
            .ok_or("云效附件信息不完整")?;
        if metadata.size > REMOTE_ATTACHMENT_LIMIT {
            return Err("附件超过 100 MB，不能在客户端预览".into());
        }
        let bytes = request
            .api
            .download_attachment_bytes(&row, REMOTE_ATTACHMENT_LIMIT)
            .await?;
        let name = safe_attachment_name(&metadata.name, &metadata.suffix);
        if dangerous_preview(&bytes, &name) {
            return Err("此附件是程序或脚本，不能作为预览启动".into());
        }
        if bytes.len() <= INLINE_IMAGE_LIMIT && bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Ok((
                ProjectBugAttachmentPreview {
                    kind: "image",
                    name,
                    mime: "image/png".into(),
                    data: format!("data:image/png;base64,{}", STANDARD.encode(bytes)),
                },
                None,
            ));
        }
        if bytes.len() <= INLINE_IMAGE_LIMIT && bytes.starts_with(b"\xff\xd8\xff") {
            return Ok((
                ProjectBugAttachmentPreview {
                    kind: "image",
                    name,
                    mime: "image/jpeg".into(),
                    data: format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes)),
                },
                None,
            ));
        }
        if bytes.len() <= INLINE_TEXT_LIMIT && !bytes.contains(&0) {
            if let Ok(text) = String::from_utf8(bytes.clone()) {
                return Ok((
                    ProjectBugAttachmentPreview {
                        kind: "text",
                        name,
                        mime: "text/plain".into(),
                        data: text,
                    },
                    None,
                ));
            }
        }
        let attachment =
            system_preview_attachment(state, &request.key, id, attachment_id, &metadata, &bytes)?;
        Ok((
            ProjectBugAttachmentPreview {
                kind: "system",
                name,
                mime: attachment.mime.clone(),
                data: String::new(),
            },
            Some(attachment),
        ))
    }
    .await;
    crate::workspace::ensure_read_scope(state, &request)?;
    crate::workspace::observed_error(state, result)
}

async fn inline_file_preview_for_state(
    state: &AppState,
    id: &str,
    file_id: &str,
    project_id: Option<&str>,
) -> Result<ProjectBugAttachmentPreview> {
    crate::yunxiao::encode_id(id).map_err(|_| "项目 Bug ID 不正确")?;
    crate::yunxiao::encode_id(file_id).map_err(|_| "正文图片文件 ID 不正确")?;
    let request = crate::workspace::read_scope(state, project_id)?;
    let result = async {
        let value = request.api.get(id).await?;
        item(&value, &request.api.profile.project_id)?;
        let mut files = inline_files_from_content(&value, "description");
        if !files.iter().any(|file| file.file_id == file_id) {
            for comment in request.api.comments(id).await? {
                merge_inline_files(&mut files, inline_files_from_content(&comment, "content"));
            }
        }
        let embedded = files
            .into_iter()
            .find(|file| file.file_id == file_id)
            .ok_or("正文图片不存在或已被移除")?;
        if embedded.size > INLINE_IMAGE_LIMIT as u64 {
            return Err("正文图片超过 12 MB，不能在详情中直接预览".into());
        }
        let row = request.api.workitem_file(id, file_id).await?;
        let remote_size = row["size"]
            .as_u64()
            .or_else(|| string(&row, "size").parse().ok())
            .unwrap_or_default();
        if remote_size > INLINE_IMAGE_LIMIT as u64 {
            return Err("正文图片超过 12 MB，不能在详情中直接预览".into());
        }
        let bytes = request
            .api
            .download_attachment_bytes(&row, INLINE_IMAGE_LIMIT as u64)
            .await?;
        let remote_name = {
            let name = string(&row, "name");
            if name.is_empty() {
                string(&row, "fileName")
            } else {
                name
            }
        };
        let remote_suffix = string(&row, "suffix");
        let name = safe_attachment_name(
            if remote_name.is_empty() {
                &embedded.name
            } else {
                &remote_name
            },
            if remote_suffix.is_empty() {
                &embedded.suffix
            } else {
                &remote_suffix
            },
        );
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Ok(ProjectBugAttachmentPreview {
                kind: "image",
                name,
                mime: "image/png".into(),
                data: format!("data:image/png;base64,{}", STANDARD.encode(bytes)),
            });
        }
        if bytes.starts_with(b"\xff\xd8\xff") {
            return Ok(ProjectBugAttachmentPreview {
                kind: "image",
                name,
                mime: "image/jpeg".into(),
                data: format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes)),
            });
        }
        Err("正文文件不是可预览的 PNG 或 JPEG 图片".into())
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

#[tauri::command]
pub async fn preview_project_bug_attachment(
    id: String,
    attachment_id: String,
    project_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<ProjectBugAttachmentPreview> {
    let (preview, attachment) =
        attachment_preview_for_state(&state, &id, &attachment_id, project_id.as_deref()).await?;
    if let Some(attachment) = attachment {
        tauri::async_runtime::spawn_blocking(move || {
            if crate::commands::uses_macos_quick_look(&attachment) {
                crate::evidence::verify_attachment(&attachment)?;
                return std::process::Command::new("/usr/bin/qlmanage")
                    .args(["-p", &attachment.path])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .map(|_| ())
                    .map_err(|_| "无法使用 macOS 快速查看预览云效附件".to_string());
            }
            crate::commands::open_attachment_with(
                &attachment,
                |path, application| match application {
                    Some(application) => open::with(path, application),
                    None => open::that(path),
                },
            )
        })
        .await
        .map_err(|_| "云效附件预览启动失败".to_string())??;
    }
    Ok(preview)
}

#[tauri::command]
pub async fn preview_project_bug_inline_file(
    id: String,
    file_id: String,
    project_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<ProjectBugAttachmentPreview> {
    inline_file_preview_for_state(&state, &id, &file_id, project_id.as_deref()).await
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

    #[test]
    fn content_payload_accepts_stringified_cloud_richtext_objects() {
        let detail = json!({
            "description": "{\"htmlValue\":\"<p>真实正文</p>\",\"jsonMLValue\":[\"root\",{}]}",
            "formatType": "RICHTEXT"
        });
        let (content, format) = content_payload(&detail, "description", "formatType");
        assert_eq!(content, "<p>真实正文</p>");
        assert!(matches!(format, ProjectBugContentFormat::Richtext));

        let comment = json!({
            "content": {"htmlValue":"<p>真实评论</p>","jsonMLValue":["root",{}]},
            "contentFormat": ""
        });
        let (content, format) = content_payload(&comment, "content", "contentFormat");
        assert_eq!(content, "<p>真实评论</p>");
        assert!(matches!(format, ProjectBugContentFormat::Richtext));

        let real_shape = json!({
            "description": serde_json::to_string(&json!({
                "htmlValue":"<p><img src=\"https://devops.aliyun.com/projex/api/workitem/file/url?fileIdentifier=6c0fb48b245b0e833822df43a9\"></p>",
                "jsonMLValue":["root",{},["p",{},["img",{
                    "id":"829sgi","name":"image.png","size":424784,
                    "src":"https://devops.aliyun.com/projex/api/workitem/file/url?fileIdentifier=6c0fb48b245b0e833822df43a9"
                }]]]
            })).unwrap(),
            "formatType":"RICHTEXT"
        });
        let files = inline_files_from_content(&real_shape, "description");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_id, "6c0fb48b245b0e833822df43a9");
        assert_eq!(files[0].name, "image.png");
        assert_eq!(files[0].size, 424784);
    }

    #[test]
    fn timestamps_accept_real_cloud_milliseconds_and_legacy_strings() {
        let value = json!({
            "gmtCreate": 1792057374000_i64,
            "gmtModified": 1792120296000_i64,
            "createTime":"2026-10-16 10:00:00"
        });
        assert_eq!(
            timestamp_value(&value, &["gmtCreate", "createTime"]),
            "2026-10-15T09:42:54+00:00"
        );
        assert_eq!(
            timestamp_value(&value, &["missing", "createTime"]),
            "2026-10-16 10:00:00"
        );
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

    #[tokio::test]
    async fn remote_image_preview_uses_a_separate_client_without_the_pat() {
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
        Mock::given(method("GET"))
            .and(path(format!("{base}/attachments")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "id":"attachment-a","fileId":"file-a","fileName":"screen.png",
                "suffix":".png","size":12,"url":format!("{}/signed/screen", server.uri())
            }])))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/signed/screen"))
            .respond_with(
                ResponseTemplate::new(200).set_body_bytes(b"\x89PNG\r\n\x1a\nmock".to_vec()),
            )
            .expect(1)
            .mount(&server)
            .await;
        let root = tempfile::tempdir().unwrap();
        let state = state(&server, root.path());
        let (preview, system_file) =
            attachment_preview_for_state(&state, "bug-a", "attachment-a", Some("project-a"))
                .await
                .unwrap();
        assert_eq!(preview.kind, "image");
        assert!(preview.data.starts_with("data:image/png;base64,"));
        assert!(system_file.is_none());
        let requests = server.received_requests().await.unwrap();
        let signed = requests
            .iter()
            .find(|request| request.url.path() == "/signed/screen")
            .unwrap();
        assert!(signed.headers.get("x-yunxiao-token").is_none());
        assert!(signed.headers.get("authorization").is_none());
    }

    #[tokio::test]
    async fn embedded_workitem_image_is_resolved_by_file_identifier_without_pat_leak() {
        let server = MockServer::start().await;
        let base = "/oapi/v1/projex/organizations/org-a/workitems/bug-a";
        Mock::given(method("GET"))
            .and(path(base))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id":"bug-a","serialNumber":"SKII-629","subject":"UI 异常",
                "categoryId":"Bug","space":{"id":"project-a"},"labels":[],
                "description":serde_json::to_string(&json!({
                    "htmlValue":"<p><img src=\"https://devops.aliyun.com/projex/api/workitem/file/url?fileIdentifier=inline-a\"></p>",
                    "jsonMLValue":["root",{},["img",{
                        "name":"image.png","size":12,
                        "src":"https://devops.aliyun.com/projex/api/workitem/file/url?fileIdentifier=inline-a"
                    }]]
                })).unwrap(),
                "formatType":"RICHTEXT"
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("{base}/files/inline-a")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id":"inline-a","name":"image.png","suffix":".png","size":12,
                "url":format!("{}/signed/inline-image", server.uri())
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/signed/inline-image"))
            .respond_with(
                ResponseTemplate::new(200).set_body_bytes(b"\x89PNG\r\n\x1a\nmock".to_vec()),
            )
            .expect(1)
            .mount(&server)
            .await;
        let root = tempfile::tempdir().unwrap();
        let state = state(&server, root.path());
        let preview = inline_file_preview_for_state(&state, "bug-a", "inline-a", Some("project-a"))
            .await
            .unwrap();
        assert_eq!(preview.kind, "image");
        assert_eq!(preview.name, "image.png");
        let requests = server.received_requests().await.unwrap();
        let signed = requests
            .iter()
            .find(|request| request.url.path() == "/signed/inline-image")
            .unwrap();
        assert!(signed.headers.get("x-yunxiao-token").is_none());
        assert!(signed.headers.get("authorization").is_none());
    }

    #[tokio::test]
    async fn remote_preview_rejects_programs_and_untrusted_plain_http_urls() {
        assert!(dangerous_preview(b"#!/bin/sh\necho unsafe", "capture.txt"));
        assert!(dangerous_preview(b"MZmock", "capture.log"));
        assert_eq!(safe_attachment_name("../bad:name", ".txt"), ".._bad_name");

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
        Mock::given(method("GET"))
            .and(path(format!("{base}/attachments")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "id":"attachment-a","fileId":"file-a","fileName":"capture.zip",
                "suffix":".zip","size":12,"url":"http://example.test/capture.zip"
            }])))
            .expect(1)
            .mount(&server)
            .await;
        let root = tempfile::tempdir().unwrap();
        let state = state(&server, root.path());
        assert!(
            attachment_preview_for_state(&state, "bug-a", "attachment-a", Some("project-a"))
                .await
                .unwrap_err()
                .contains("HTTPS")
        );
    }
}
