use crate::model::*;
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
        if f.field_type == "NativeField"
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
    if !p.business_profiles.contains(&string(d, "business")) {
        add!("business", "请选择配置中的业务方向");
    }
    req!("title", "具体问题标题");
    if string(d, "title").trim().chars().count() < 5 {
        add!("title", "标题至少 5 个字符，并说明具体行为");
    }
    if fixed_build_url(&string(d, "buildUrl")).is_none() {
        add!("buildUrl", "请填写 Jenkins 固定构建号 URL");
    }
    if string(d, "buildStatus") == "conflict" {
        add!("buildStatus", "构建身份冲突，请重新选择实际构建");
    }
    if string(d, "buildStatus") == "unconfirmed" {
        add!("buildStatus", "请确认构建与当前安装包的关系");
    }
    if !d["buildManifest"].is_null()
        && ["packageName", "versionName", "versionCode"]
            .iter()
            .any(|k| d["buildManifest"][k] != d["app"][k])
    {
        add!("buildStatus", "构建清单与设备包名或版本不一致");
    }
    for key in [
        "packageName",
        "versionName",
        "channel",
        "environment",
        "obfuscation",
        "hardening",
    ] {
        req!(&format!("app.{key}"), key);
    }
    let pkg = regex::Regex::new(r"^[A-Za-z][\w]*(\.[A-Za-z][\w]*)+$").unwrap();
    if !pkg.is_match(&string(d, "app.packageName")) {
        add!("app.packageName", "包名格式不正确");
    }
    if string(d, "app.versionCode")
        .parse::<u64>()
        .ok()
        .is_none_or(|n| n > 9_007_199_254_740_991)
    {
        add!("app.versionCode", "versionCode 必须为非负整数（允许 0）");
    }
    for key in [
        "model",
        "androidVersion",
        "totalMemKb",
        "availableMemKb",
        "resolution",
        "density",
    ] {
        let path = format!("device.{key}");
        req!(&path, key);
        if useful(&string(d, &path))
            && d["sources"][&path]["status"] != "collected"
            && !useful(&string(d, "manualNote"))
        {
            add!("manualNote", "请注明人工设备信息来源");
        }
    }
    for key in ["totalMemKb", "availableMemKb", "density"] {
        if string(d, &format!("device.{key}"))
            .parse::<f64>()
            .ok()
            .is_none_or(|v| !v.is_finite() || v < 0.0 || (key != "availableMemKb" && v == 0.0))
        {
            add!(format!("device.{key}"), "请输入有效数值及指定单位");
        }
    }
    if string(d, "device.availableMemKb")
        .parse::<f64>()
        .unwrap_or(0.)
        > string(d, "device.totalMemKb").parse::<f64>().unwrap_or(0.)
    {
        add!("device.availableMemKb", "可用内存不能超过总内存");
    }
    if !regex::Regex::new(r"^[0-9]+[x×][0-9]+$")
        .unwrap()
        .is_match(&string(d, "device.resolution"))
    {
        add!("device.resolution", "分辨率格式为 1080x2400");
    }
    if !zoned_time(&string(d, "event.occurredAt")) {
        add!("event.occurredAt", "问题时间必须包含时区");
    }
    if string(d, "event.source") == "manual"
        || (useful(&string(d, "event.originalAt"))
            && d["event.originalAt"] != d["event.occurredAt"])
    {
        req!("event.sourceNote", "事件时间来源或修改依据");
    }
    req!("preconditions", "前置条件");
    req!("expected", "预期结果");
    req!("actual", "实际结果");
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
    if !files.iter().any(|a| {
        a.role == "log"
            && a.confirmed
            && a.size > 0
            && useful(&a.source)
            && zoned_time(&a.coverage_start)
            && zoned_time(&a.coverage_end)
            && chrono::DateTime::parse_from_rfc3339(&a.coverage_end).ok()
                >= chrono::DateTime::parse_from_rfc3339(&a.coverage_start).ok()
    }) {
        add!("attachments", "请确认非空日志及真实覆盖起止时间");
    }
    if bool_at(d, "event.lateCapture")
        && !files
            .iter()
            .any(|a| a.confirmed && a.size > 0 && a.source.starts_with("导入"))
    {
        add!("event.lateCapture", "事后补采需导入已有证据或重新复现");
    }
    let kind = string(d, "kind");
    if ["UI", "多语言", "动态过程"].contains(&kind.as_str())
        && !files
            .iter()
            .any(|a| ["image", "video"].contains(&a.role.as_str()) && a.confirmed && a.size > 0)
    {
        add!("attachments", "显示问题需要截图或视频");
    }
    if kind == "动态过程"
        && !files.iter().any(|a| a.role == "video" && a.confirmed)
        && files
            .iter()
            .filter(|a| a.role == "image" && a.confirmed)
            .count()
            < 2
    {
        add!("attachments", "动态过程需要视频或连续多张截图");
    }
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
    match string(d, "cloud.applicability").as_str() {
        "no" => {
            req!("cloud.reason", "该功能无云控的依据");
        }
        "yes" => {
            for k in ["name", "effectiveValue", "source", "override"] {
                req!(&format!("cloud.{k}"), "云控有效值、来源及覆盖声明");
            }
            if !["应用调试面板", "应用导出", "相关运行日志", "用户可观察行为"]
                .contains(&string(d, "cloud.source").as_str())
            {
                add!("cloud.source", "请选择设备生效证据来源");
            }
            if !zoned_time(&string(d, "cloud.observedAt")) {
                add!("cloud.observedAt", "云控观察时间必须包含时区");
            }
        }
        _ => add!("cloud.applicability", "请声明云控适用性"),
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
                if !present || target.is_some() {
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
                if present && !m.fields.iter().any(|f| f.id == *id) {
                    add!(
                        format!("customFields.{id}"),
                        "字段不在最新项目配置中，请核对后移除"
                    );
                }
            }
            for id in &p.required_field_ids {
                if !m.fields.iter().any(|f| f.id == *id) {
                    add!("project", format!("项目必填字段 {id} 未出现在云效元数据中"));
                }
            }
        }
    }
    if online && !bool_at(d, "previewConfirmed") {
        add!("previewConfirmed", "请预览并确认项目、构建、描述及附件");
    }
    errors
}
