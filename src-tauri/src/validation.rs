use crate::model::*;
use crate::submission_fields::{excluded_field, excluded_id, hidden_optional_field};
use serde_json::Value;
pub fn useful(s: &str) -> bool {
    !s.trim().is_empty()
        && ![
            "未知",
            "待定",
            "待补充",
            "稍后补",
            "无",
            "不清楚",
            "unknown",
            "todo",
            "tbd",
            "n/a",
            "-",
        ]
        .contains(&s.trim().to_lowercase().as_str())
}
pub fn zoned_time(s: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(s).is_ok()
}
pub fn fixed_build_url(s: &str) -> Option<(String, String)> {
    let u = reqwest::Url::parse(s).ok()?;
    if !["http", "https"].contains(&u.scheme())
        || !u.username().is_empty()
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
    {
        return None;
    }
    let re = regex::Regex::new(r"^(.*(?:/job/[^/]+)+)/([1-9][0-9]*)/?$").unwrap();
    let caps = re.captures(u.path())?;
    if [
        "lastBuild",
        "lastSuccessfulBuild",
        "lastStableBuild",
        "lastCompletedBuild",
    ]
    .iter()
    .any(|v| u.path().split('/').any(|x| x == *v))
    {
        return None;
    }
    Some((caps[1].into(), caps[2].into()))
}
pub fn check_shape(d: &Value) -> Result<()> {
    if serde_json::to_vec(d).map_err(|e| e.to_string())?.len() > 2_000_000 {
        return Err("草稿超过体积限制".into());
    }
    let schema: Value =
        serde_json::from_str(include_str!("../../schemas/draft.schema.json")).unwrap();
    if !jsonschema::is_valid(&schema, d) {
        return Err("草稿数据格式错误，请重新打开草稿".into());
    }
    Ok(())
}
pub fn native_target<'a>(f: &'a Field, p: &'a Profile) -> Option<&'a str> {
    p.field_mappings.get(&f.id).map(String::as_str).or_else(|| {
        if (f.field_type == "NativeField"
            || (f.field_type == "Role"
                && ["verifier", "participants", "trackers"].contains(&f.id.as_str()))
            || (f.field_type == "Application"
                && ["sprint", "versions", "labels"].contains(&f.id.as_str())))
            && [
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
            .contains(&f.id.as_str())
        {
            Some(f.id.as_str())
        } else {
            None
        }
    })
}
/// Derivatives and their source receipts form one indivisible evidence set.
pub fn receipt_error(files: &[Attachment]) -> Option<String> {
    for a in files {
        let paired: Vec<_> = files
            .iter()
            .filter(|r| r.role == "report" && r.derived_from.as_deref() == Some(&a.id))
            .collect();
        let window = a.source.starts_with("ADB 问题日志窗口派生副本");
        let redacted = a.source.starts_with("文本派生副本");
        if a.role == "log" && (window || redacted || !paired.is_empty()) {
            let range_required = window
                || a.source.contains("来源为问题日志窗口")
                || (redacted
                    && a.derived_from.as_ref().is_some_and(|id| {
                        files.iter().any(|parent| {
                            &parent.id == id
                                && parent.source.starts_with("ADB 问题日志窗口派生副本")
                        })
                    }));
            if (range_required && !paired.iter().any(|r| r.source == "日志窗口范围与缺口记录"))
                || (redacted
                    && !paired
                        .iter()
                        .any(|r| r.source == "文本替换记录（请核对副本）"))
            {
                return Some("派生副本必须携带全部来源回执".into());
            }
        }
        if a.role == "report"
            && a.derived_from
                .as_ref()
                .is_some_and(|owner| !files.iter().any(|parent| &parent.id == owner))
        {
            return Some("来源回执必须保留关联副本".into());
        }
    }
    None
}
pub fn validate(
    d: &Value,
    p: &Profile,
    m: Option<&Metadata>,
    authorized: bool,
    online: bool,
) -> Vec<Issue> {
    let mut errors = Vec::new();
    macro_rules! add {
        ($field:expr,$msg:expr) => {
            errors.push(Issue {
                field: $field.into(),
                message: $msg.into(),
            })
        };
    }
    macro_rules! req {
        ($field:expr,$label:expr) => {
            if !useful(&string(d, $field)) {
                add!($field, format!("请补充{}", $label));
            }
        };
    }
    if check_shape(d).is_err() {
        add!("draft", "草稿格式不正确");
        return errors;
    }
    if online && !authorized {
        add!("auth", "请填写个人 PAT 并读取项目配置");
    }
    if online
        && !m.is_some_and(|m| {
            m.profile.project_id == p.project_id
                && m.profile.organization_id == p.organization_id
                && m.type_id == string(d, "workitemTypeId")
        })
    {
        add!("project", "请重新读取当前项目和缺陷类型配置");
    }
    req!("title", "问题标题");
    req!("app.packageName", "当前项目的 Android 包名配置");
    req!("preconditions", "复现条件");
    req!("expected", "期望表现");
    req!("actual", "问题表现");
    let steps = string(d, "steps");
    if !steps.lines().any(|s| !s.trim().is_empty())
        || steps.lines().filter(|s| !s.trim().is_empty()).any(|s| {
            s.trim().chars().count() < 6
                || ["打开看一下", "打开看看", "查看一下"].contains(&s.trim())
        })
    {
        add!("steps", "请逐行填写可执行的复现步骤");
    }
    if !bool_at(d, "frequency.firstDiscovery") {
        let a = string(d, "frequency.attempts").parse::<u64>().unwrap_or(0);
        let b = string(d, "frequency.occurrences")
            .parse::<u64>()
            .unwrap_or(0);
        if a < 1 || b < 1 || b > a {
            add!("frequency", "发生次数应为 1 至尝试次数之间的整数");
        }
    }
    let files = attachments(d).unwrap_or_default();
    if let Some(error) = receipt_error(&files) {
        add!("attachments", error);
    }
    for a in files.iter().filter(|a| a.role == "log") {
        if a.size == 0 || !useful(&a.source) {
            add!(
                format!("attachment.{}.confirmed", a.id),
                "所选日志为空或来源缺失，请重新导入"
            );
        }
        if !zoned_time(&a.coverage_start) {
            add!(
                format!("attachment.{}.coverageStart", a.id),
                "请填写日志的真实覆盖起点（含时区）"
            );
        }
        if !zoned_time(&a.coverage_end)
            || chrono::DateTime::parse_from_rfc3339(&a.coverage_end).ok()
                < chrono::DateTime::parse_from_rfc3339(&a.coverage_start).ok()
        {
            add!(
                format!("attachment.{}.coverageEnd", a.id),
                "请填写日志的真实覆盖终点，且不得早于起点（含时区）"
            );
        }
    }
    let kind = string(d, "kind");
    if kind == "网络" {
        for k in ["operation", "type", "error", "collectionStatus"] {
            req!(
                &format!("network.{k}"),
                "网络操作、类型、可观察错误和采集状态"
            );
        }
    }
    if kind == "数据" {
        for k in ["type", "count", "source"] {
            req!(&format!("data.{k}"), "数据类型、数量和来源");
        }
    }
    if online {
        if let Some(m) = m {
            if !m.members.iter().any(|v| v.id == string(d, "assignedTo")) {
                add!("assignedTo", "请选择项目真实成员");
            }
            let versions = d["versions"].as_array().unwrap();
            let sprint = string(d, "sprint");
            if versions.is_empty() && sprint.is_empty() {
                add!("scope", "至少选择一个真实版本或迭代");
            }
            if !sprint.is_empty() && !m.sprints.iter().any(|o| o.id == sprint) {
                add!("sprint", "迭代 ID 已失效");
            }
            if versions
                .iter()
                .any(|v| !m.versions.iter().any(|o| v == o.id.as_str()))
            {
                add!("versions", "版本 ID 已失效");
            }
            for f in &m.fields {
                if excluded_field(f, p) {
                    if f.required || p.required_field_ids.contains(&f.id) {
                        add!(
                            format!("customFields.{}", f.id),
                            format!(
                                "云效项目将已移除字段「{}」设为必填，请调整项目配置后提交",
                                f.name
                            )
                        );
                    }
                    continue;
                }
                if hidden_optional_field(f, p) {
                    continue;
                }
                let target = native_target(f, p);
                let v = match target {
                    Some("subject") => d["title"].clone(),
                    Some("description") => d["actual"].clone(),
                    Some("assignedTo") => d["assignedTo"].clone(),
                    Some("sprint") => d["sprint"].clone(),
                    Some("versions") => d["versions"].clone(),
                    _ => d["customFields"][&f.id].clone(),
                };
                let vals: Vec<String> = v
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_else(|| vec![v.as_str().unwrap_or("").into()]);
                let present = !vals.is_empty() && vals.iter().all(|v| useful(v));
                let required = f.required || p.required_field_ids.contains(&f.id);
                if (required || present)
                    && (![
                        "list",
                        "multiList",
                        "tag",
                        "string",
                        "text",
                        "int",
                        "float",
                        "date",
                        "dateTime",
                        "user",
                        "multiUser",
                        "sprint",
                        "version",
                    ]
                    .contains(&f.format.as_str())
                        || (target.is_none()
                            && !["CustomField", "SystemCustomField"]
                                .contains(&f.field_type.as_str())))
                {
                    add!(
                        format!("customFields.{}", f.id),
                        format!("字段「{}」格式或映射未支持", f.name)
                    );
                    continue;
                }
                if required && !present {
                    add!(
                        format!("customFields.{}", f.id),
                        format!("请补充云效必填字段「{}」", f.name)
                    );
                }
                if !present
                    || target.is_some_and(|v| {
                        ["subject", "description", "assignedTo", "sprint", "versions"].contains(&v)
                    })
                {
                    continue;
                }
                let options = match f.format.as_str() {
                    "user" | "multiUser" => &m.members,
                    "sprint" => &m.sprints,
                    "version" => &m.versions,
                    _ => &f.options,
                };
                if [
                    "list",
                    "multiList",
                    "tag",
                    "user",
                    "multiUser",
                    "sprint",
                    "version",
                ]
                .contains(&f.format.as_str())
                    && vals.iter().any(|v| !options.iter().any(|o| o.id == *v))
                {
                    add!(format!("customFields.{}", f.id), "选项 ID 已失效");
                }
                if (f.format == "int" && vals.iter().any(|v| v.parse::<i64>().is_err()))
                    || (f.format == "float"
                        && vals
                            .iter()
                            .any(|v| v.parse::<f64>().ok().is_none_or(|n| !n.is_finite())))
                {
                    add!(format!("customFields.{}", f.id), "数值格式不正确");
                }
                if (f.format == "dateTime" && vals.iter().any(|v| !zoned_time(v)))
                    || (f.format == "date"
                        && vals
                            .iter()
                            .any(|v| chrono::NaiveDate::parse_from_str(v, "%Y-%m-%d").is_err()))
                {
                    add!(format!("customFields.{}", f.id), "日期或时间格式不正确");
                }
            }
            for (id, v) in d["customFields"].as_object().unwrap() {
                let present =
                    v.as_array().is_some_and(|a| !a.is_empty()) || v.as_str().is_some_and(useful);
                if excluded_id(id)
                    || p.field_mappings
                        .get(id)
                        .is_some_and(|target| excluded_id(target))
                {
                    continue;
                }
                if present && !m.fields.iter().any(|f| f.id == *id) {
                    add!(
                        format!("customFields.{id}"),
                        "字段不在最新项目配置中，请核对后移除"
                    );
                }
            }
            for id in &p.required_field_ids {
                if !m.fields.iter().any(|f| f.id == *id) {
                    if excluded_id(id) {
                        add!(
                            "project",
                            format!(
                                "云效项目将已移除字段「{}」设为必填，请调整项目配置后提交",
                                id
                            )
                        );
                    } else {
                        add!("project", format!("项目必填字段 {id} 未出现在云效元数据中"));
                    }
                }
            }
        }
    }
    if online && !bool_at(d, "previewConfirmed") {
        add!("previewConfirmed", "请预览并确认项目、描述及附件");
    }
    errors
}
