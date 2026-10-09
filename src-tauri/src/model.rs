use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
pub type Result<T> = std::result::Result<T, String>;
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
pub fn string(v: &Value, path: &str) -> String {
    path.split('.')
        .fold(v, |v, p| &v[p])
        .as_str()
        .unwrap_or("")
        .to_string()
}
pub fn bool_at(v: &Value, path: &str) -> bool {
    path.split('.')
        .fold(v, |v, p| &v[p])
        .as_bool()
        .unwrap_or(false)
}
pub fn uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profile {
    pub name: String,
    pub edition: String,
    pub organization_id: String,
    pub project_id: String,
    pub business_profiles: Vec<String>,
    pub required_field_ids: Vec<String>,
    pub field_mappings: HashMap<String, String>,
    pub enable_versions: bool,
    pub enable_sprints: bool,
    pub attachment_warning_mb: f64,
}
impl Profile {
    pub fn validate(&self) -> Result<()> {
        let id = regex::Regex::new(r"^[A-Za-z0-9_-]{1,128}$").unwrap();
        if self.edition != "central"
            || !id.is_match(&self.organization_id)
            || !id.is_match(&self.project_id)
            || self.business_profiles.is_empty()
            || self.business_profiles.iter().any(|s| s.is_empty())
        {
            return Err("项目配置不正确；首版仅支持云效中心版".into());
        }
        if self.field_mappings.values().any(|s| {
            ![
                "subject",
                "description",
                "assignedTo",
                "sprint",
                "versions",
                "verifier",
                "participants",
                "labels",
                "trackers",
            ]
            .contains(&s.as_str())
        }) {
            return Err("存在未支持的原生字段映射".into());
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct OptionItem {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Field {
    pub id: String,
    pub name: String,
    pub format: String,
    #[serde(rename = "type")]
    pub field_type: String,
    pub required: bool,
    pub options: Vec<OptionItem>,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub profile: Profile,
    pub type_id: String,
    pub types: Vec<OptionItem>,
    pub fields: Vec<Field>,
    pub members: Vec<OptionItem>,
    pub versions: Vec<OptionItem>,
    pub sprints: Vec<OptionItem>,
    pub fetched_at: String,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub path: String,
    pub mime: String,
    pub size: u64,
    pub sha256: String,
    pub role: String,
    pub source: String,
    pub coverage_start: String,
    pub coverage_end: String,
    pub confirmed: bool,
    pub derived_from: Option<String>,
}
pub fn attachments(d: &Value) -> Result<Vec<Attachment>> {
    serde_json::from_value(d["attachments"].clone()).map_err(|_| "附件数据格式错误".into())
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Issue {
    pub field: String,
    pub message: String,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub id: String,
    pub directory: String,
    pub zip_path: String,
    pub sha256: String,
    pub files: Vec<Attachment>,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Upload {
    pub local_id: String,
    pub name: String,
    pub sha256: String,
    pub remote_id: Option<String>,
    pub state: String,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Submission {
    pub draft_id: String,
    pub profile: Profile,
    pub state: String,
    pub evidence: Evidence,
    pub workitem_id: Option<String>,
    pub identifier: Option<String>,
    pub url: Option<String>,
    pub uploads: Vec<Upload>,
    pub error: Option<String>,
    pub intended_at: String,
    pub updated_at: String,
}
