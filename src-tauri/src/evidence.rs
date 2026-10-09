use crate::{model::*, store::Store};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};
pub fn hash_file(path: &Path) -> Result<(u64, String)> {
    let mut file = File::open(path).map_err(|_| "附件文件不存在或无法读取")?;
    let mut hash = Sha256::new();
    let mut total = 0;
    let mut buf = [0u8; 65536];
    loop {
        let len = file.read(&mut buf).map_err(|_| "附件读取失败")?;
        if len == 0 {
            break;
        }
        total += len as u64;
        hash.update(&buf[..len]);
    }
    Ok((total, hex::encode(hash.finalize())))
}

/// Only fixed internal categories are accepted. Canonical containment rejects redirected directories.
pub(crate) fn derived_directory(root: &Path, category: &str) -> Result<std::path::PathBuf> {
    if !["attachments", "bugreports", "log-windows", "redactions"].contains(&category) {
        return Err("证据目录类型不支持".into());
    }
    let root = root.canonicalize().map_err(|_| "本地证据根目录不存在")?;
    let parent = root.join(category);
    fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
    let parent = parent.canonicalize().map_err(|e| e.to_string())?;
    if !parent.starts_with(&root) {
        return Err("证据目录不能离开本地根目录".into());
    }
    let directory = parent.join(uuid());
    fs::create_dir(&directory).map_err(|e| e.to_string())?;
    Ok(directory)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogGap {
    /// Host timestamps; an unknown boundary is deliberately treated as a relevant gap.
    pub start: Option<String>,
    pub end: Option<String>,
    pub reason: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogWindowSummary {
    pub marked_at: String,
    pub requested_start: String,
    pub requested_end: String,
    pub observed_until: String,
    pub device_minus_host_micros: Option<i64>,
    pub clock_note: String,
    pub sources: Vec<Value>,
    pub selected_records: u64,
    pub coverage_start: Option<String>,
    pub coverage_end: Option<String>,
    pub range_covered: bool,
    pub gaps: Vec<LogGap>,
    pub limitations: Vec<String>,
}

#[derive(Debug)]
pub struct LogWindowCapture {
    pub attachment: Attachment,
    pub summary: LogWindowSummary,
    pub summary_path: std::path::PathBuf,
}

/// Summarize immutable pinned logs, not the collector's changing files. Times in output are
/// host times after applying an explicitly supplied clock estimate; unknown clocks block coverage.
/// range_covered describes observed timestamp bounds, not proof that every device log was emitted.
pub fn summarize_log_window(
    root: &Path,
    raw: &[Attachment],
    window: &crate::adb::ProblemWindow,
    observed_until: &str,
    device_minus_host_micros: Option<i64>,
    clock_note: &str,
    gaps: &[LogGap],
) -> Result<LogWindowCapture> {
    use chrono::{DateTime, Duration, Utc};
    use std::io::BufRead;
    let mark = DateTime::parse_from_rfc3339(&window.marked_at)
        .map_err(|_| "问题标记时间无效")?
        .with_timezone(&Utc);
    let start = mark
        .checked_sub_signed(Duration::seconds(120))
        .ok_or("问题时间超出范围")?;
    let end = mark
        .checked_add_signed(Duration::seconds(30))
        .ok_or("问题时间超出范围")?;
    if DateTime::parse_from_rfc3339(&window.due_at)
        .map_err(|_| "截止时间无效")?
        .with_timezone(&Utc)
        != end
    {
        return Err("日志窗口截止时间不一致".into());
    }
    let observed = DateTime::parse_from_rfc3339(observed_until)
        .map_err(|_| "实际观察时间无效")?
        .with_timezone(&Utc);
    if clock_note.trim().is_empty() {
        return Err("必须说明设备日志时间与电脑标记的对应依据".into());
    }
    let mut summary = LogWindowSummary {
        marked_at: window.marked_at.clone(),
        requested_start: start.to_rfc3339(),
        requested_end: end.to_rfc3339(),
        observed_until: observed_until.into(),
        device_minus_host_micros,
        clock_note: clock_note.into(),
        sources: Vec::new(),
        selected_records: 0,
        coverage_start: None,
        coverage_end: None,
        range_covered: false,
        gaps: Vec::new(),
        limitations: Vec::new(),
    };
    for gap in gaps {
        let a = gap
            .start
            .as_deref()
            .map(DateTime::parse_from_rfc3339)
            .transpose()
            .map_err(|_| "缺口开始时间无效")?;
        let b = gap
            .end
            .as_deref()
            .map(DateTime::parse_from_rfc3339)
            .transpose()
            .map_err(|_| "缺口结束时间无效")?;
        if a.zip(b).is_some_and(|(a, b)| a > b) {
            return Err("日志缺口时间顺序错误".into());
        }
        if a.is_none_or(|v| v <= end) && b.is_none_or(|v| v >= start) {
            summary.gaps.push(gap.clone());
        }
    }
    if observed < end {
        summary
            .limitations
            .push("观察在标记后 30 秒之前结束；后窗口不足或客户端中断".into());
    }
    if device_minus_host_micros.is_none() {
        summary
            .limitations
            .push("设备与电脑时间对应未采集；不能推定日志覆盖问题窗口".into());
    }
    let directory = derived_directory(root, "log-windows")?;
    let output_path = directory.join("window.log");
    let mut output = File::create(&output_path).map_err(|e| e.to_string())?;
    writeln!(output, "# 问题日志派生窗口：前 120 秒 / 后 30 秒\n# 时间依据：{}\n# 原始日志保持不变；时间范围不证明缓冲区无丢失", clock_note.replace(['\r', '\n'], " ")).map_err(|e| e.to_string())?;
    let (mut first, mut last, mut previous) = (None::<i64>, None::<i64>, None::<i64>);
    let (mut selected_first, mut selected_last) = (None::<i64>, None::<i64>);
    let (mut unparsed, mut truncated, mut backwards) = (0u64, 0u64, false);
    for a in raw {
        if a.role != "log" || a.mime != "text/plain" {
            return Err("日志窗口只能使用文本日志固定副本".into());
        }
        verify_attachment(a)?;
        summary
            .sources
            .push(json!({"id":a.id,"sha256":a.sha256,"size":a.size,"source":a.source}));
        let mut reader = std::io::BufReader::new(File::open(&a.path).map_err(|e| e.to_string())?);
        let mut line = Vec::new();
        loop {
            line.clear();
            // Bound a single malformed record without loading the full session into memory.
            let n = reader
                .by_ref()
                .take(1_048_577)
                .read_until(b'\n', &mut line)
                .map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            if n > 1_048_576 {
                return Err("单条日志超过 1 MiB，需人工处理原始证据".into());
            }
            if !line.ends_with(b"\n") {
                truncated += 1;
            }
            let text = std::str::from_utf8(&line).map_err(|_| "日志不是完整 UTF-8 文本")?;
            if text.trim().is_empty() || text.starts_with("--------- beginning of ") {
                continue;
            }
            let Some(epoch) = epoch_micros(text) else {
                unparsed += 1;
                continue;
            };
            let Some(offset) = device_minus_host_micros else {
                continue;
            };
            let host = epoch.checked_sub(offset).ok_or("设备时间差超出范围")?;
            if DateTime::from_timestamp_micros(host).is_none() {
                return Err("日志时间超出范围".into());
            }
            if previous.is_some_and(|v| host < v) {
                backwards = true;
            }
            previous = Some(host);
            first = Some(first.map_or(host, |v| v.min(host)));
            last = Some(last.map_or(host, |v| v.max(host)));
            if host >= start.timestamp_micros()
                && host <= end.timestamp_micros()
                && host <= observed.timestamp_micros()
            {
                output.write_all(&line).map_err(|e| e.to_string())?;
                summary.selected_records += 1;
                selected_first = Some(selected_first.map_or(host, |v| v.min(host)));
                selected_last = Some(selected_last.map_or(host, |v| v.max(host)));
            }
        }
        verify_attachment(a)?;
    }
    summary.coverage_start =
        selected_first.map(|v| DateTime::from_timestamp_micros(v).unwrap().to_rfc3339());
    summary.coverage_end =
        selected_last.map(|v| DateTime::from_timestamp_micros(v).unwrap().to_rfc3339());
    if first.is_none_or(|v| v > start.timestamp_micros()) {
        summary
            .limitations
            .push("原始记录未证明前 120 秒范围已覆盖".into());
    }
    if last.is_none_or(|v| v < end.timestamp_micros()) {
        summary
            .limitations
            .push("原始记录未证明后 30 秒范围已覆盖".into());
    }
    if unparsed > 0 {
        summary.limitations.push(format!(
            "{unparsed} 行无法解析 epoch 时间；保留在原始日志中"
        ));
    }
    if truncated > 0 {
        summary
            .limitations
            .push(format!("{truncated} 条记录没有完整换行，可能被中断或分段"));
    }
    if backwards {
        summary
            .limitations
            .push("日志时间倒退或文件顺序重叠，覆盖需人工确认".into());
    }
    summary.range_covered =
        summary.limitations.is_empty() && summary.gaps.is_empty() && summary.selected_records > 0;
    output.sync_all().map_err(|e| e.to_string())?;
    let summary_path = directory.join("summary.json");
    fs::write(
        &summary_path,
        serde_json::to_vec_pretty(&summary).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let mut attachment = inspect(
        &output_path,
        "log",
        "ADB 问题日志窗口派生副本；未脱敏",
        root,
    )?;
    // Multiple sources are tracked by id/hash in summary.json; do not invent a single parent.
    if raw.len() == 1 {
        attachment.derived_from = Some(raw[0].id.clone());
    }
    attachment.coverage_start = summary.coverage_start.clone().unwrap_or_default();
    attachment.coverage_end = summary.coverage_end.clone().unwrap_or_default();
    Ok(LogWindowCapture {
        attachment,
        summary,
        summary_path,
    })
}

fn epoch_micros(line: &str) -> Option<i64> {
    let token = line.split_whitespace().next()?;
    let (seconds, fraction) = token.split_once('.')?;
    if !seconds.bytes().all(|v| v.is_ascii_digit())
        || seconds.is_empty()
        || fraction.is_empty()
        || fraction.len() > 9
        || !fraction.bytes().all(|v| v.is_ascii_digit())
    {
        return None;
    }
    let fraction = &fraction[..fraction.len().min(6)];
    seconds
        .parse::<i64>()
        .ok()?
        .checked_mul(1_000_000)?
        .checked_add(fraction.parse::<i64>().ok()? * 10i64.pow(6 - fraction.len() as u32))
}

pub struct TextRedactionRule {
    /// Non-sensitive display label. The literal itself is never stored in the receipt.
    pub label: String,
    pub literal: String,
}

#[derive(Debug)]
pub struct RedactedText {
    pub attachment: Attachment,
    pub receipt: Value,
    pub receipt_path: std::path::PathBuf,
}

/// Only exact, case-sensitive UTF-8 literal replacement. Does not claim automatic/complete sanitization.
pub fn redact_text(
    root: &Path,
    original: &Attachment,
    rules: &[TextRedactionRule],
) -> Result<RedactedText> {
    if original.role != "log" || original.mime != "text/plain" || original.size > 32_000_000 {
        return Err("文本脱敏仅支持不超过 32 MB 的 UTF-8 日志副本".into());
    }
    if rules.is_empty() || rules.len() > 100 {
        return Err("请提供 1 到 100 条明确的文本替换规则".into());
    }
    for rule in rules {
        if rule.literal.is_empty()
            || rule.literal.len() > 4096
            || rule.label.trim().is_empty()
            || rule.label.len() > 80
            || rule.label.chars().any(char::is_control)
        {
            return Err("脱敏规则必须有有效标签和非空文本（最长 4096 字节）".into());
        }
    }
    verify_attachment(original)?;
    let text = fs::read_to_string(&original.path).map_err(|_| "原始日志不是完整 UTF-8 文本")?;
    verify_attachment(original)?;
    // Longest literal wins for overlapping rules. Escaping forbids user-provided regex behavior.
    let mut ordered: Vec<usize> = (0..rules.len()).collect();
    ordered.sort_by_key(|i| std::cmp::Reverse(rules[*i].literal.len()));
    let pattern = ordered
        .iter()
        .map(|i| regex::escape(&rules[*i].literal))
        .collect::<Vec<_>>()
        .join("|");
    let regex = regex::Regex::new(&pattern).map_err(|_| "脱敏规则过大")?;
    let mut counts = vec![0u64; rules.len()];
    let redacted = regex.replace_all(&text, |matched: &regex::Captures<'_>| {
        let index = ordered
            .iter()
            .find(|i| rules[**i].literal == matched[0])
            .unwrap();
        counts[*index] += 1;
        "[REDACTED]"
    });
    let directory = derived_directory(root, "redactions")?;
    let output_path = directory.join("redacted.log");
    fs::write(&output_path, redacted.as_bytes()).map_err(|e| e.to_string())?;
    let mut attachment = inspect(
        &output_path,
        "log",
        "文本派生副本：仅按提供的精确文本替换；未保证完整脱敏",
        root,
    )?;
    attachment.derived_from = Some(original.id.clone());
    attachment.coverage_start = original.coverage_start.clone();
    attachment.coverage_end = original.coverage_end.clone();
    let receipt = json!({"derivedFrom":original.id,"originalSha256":original.sha256,
        "derivedId":attachment.id,"derivedSha256":attachment.sha256,
        "method":"exact_case_sensitive_literal","replacement":"[REDACTED]","completeRedaction":false,
        "rules":rules.iter().zip(counts).map(|(r,n)| json!({"label":r.label,"matches":n})).collect::<Vec<_>>()});
    let receipt_path = directory.join("receipt.json");
    fs::write(
        &receipt_path,
        serde_json::to_vec_pretty(&receipt).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(RedactedText {
        attachment,
        receipt,
        receipt_path,
    })
}
pub fn inspect(path: &Path, role: &str, source: &str, root: &Path) -> Result<Attachment> {
    if !["log", "image", "video", "report", "sample"].contains(&role) {
        return Err("附件角色不支持".into());
    }
    let metadata = fs::metadata(path).map_err(|_| "文件不存在")?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err("不能导入空文件或目录".into());
    }
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let mut header = [0u8; 4096];
    let len = f.read(&mut header).map_err(|e| e.to_string())?;
    let h = &header[..len];
    let png = h.starts_with(b"\x89PNG\r\n\x1a\n");
    let jpeg = h.starts_with(b"\xff\xd8\xff");
    let mp4 = len >= 12
        && &h[4..8] == b"ftyp"
        && h[8..len]
            .windows(3)
            .any(|w| w == b"mp4" || w == b"iso" || w == b"avc");
    let zip = h.starts_with(b"PK\x03\x04");
    let mime = match role {
        "image" if png => "image/png",
        "image" if jpeg => "image/jpeg",
        "video" if mp4 => "video/mp4",
        "report" if zip => "application/zip",
        "log" if !h.contains(&0) && std::str::from_utf8(h).is_ok() => "text/plain",
        "sample" => "application/octet-stream",
        _ => {
            return Err(
                "文件实际格式与角色不一致（支持 PNG/JPEG、MP4、UTF-8 日志、ZIP 报告）".into(),
            )
        }
    };
    if role == "image" {
        let mut reader = image::ImageReader::open(path)
            .map_err(|_| "图片读取失败")?
            .with_guessed_format()
            .map_err(|_| "图片格式无法识别")?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(256 * 1024 * 1024);
        reader.limits(limits);
        reader
            .decode()
            .map_err(|_| "图片损坏或解码体积过大，请重新导入")?;
    }
    if role == "video" {
        validate_mp4(path)?;
    }
    if role == "report" {
        zip::ZipArchive::new(File::open(path).map_err(|_| "报告读取失败")?)
            .map_err(|_| "ZIP 报告损坏")?;
    }
    let name = path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("attachment")
        .to_string();
    let directory = derived_directory(root, "attachments")?;
    let id = directory
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let destination = directory.join(safe_name(&name));
    fs::copy(path, &destination).map_err(|e| e.to_string())?;
    let (size, sha256) = hash_file(&destination)?;
    Ok(Attachment {
        id,
        name,
        path: destination.to_string_lossy().into(),
        mime: mime.into(),
        size,
        sha256,
        role: role.into(),
        source: source.into(),
        coverage_start: String::new(),
        coverage_end: String::new(),
        confirmed: false,
        derived_from: None,
    })
}
fn validate_mp4(path: &Path) -> Result<()> {
    let mut file = File::open(path).map_err(|_| "视频无法读取")?;
    let length = file.metadata().map_err(|_| "视频无法读取")?.len();
    let mut offset = 0;
    let (mut ftyp, mut moov, mut mdat) = (false, false, false);
    let mut boxes = 0;
    while offset < length {
        boxes += 1;
        if boxes > 10000 || length - offset < 8 {
            return Err("MP4 容器结构不完整".into());
        }
        let mut header = [0u8; 8];
        file.read_exact(&mut header).map_err(|_| "MP4 头损坏")?;
        let mut size = u32::from_be_bytes(header[..4].try_into().unwrap()) as u64;
        let mut header_size = 8;
        if size == 1 {
            let mut extended = [0u8; 8];
            file.read_exact(&mut extended)
                .map_err(|_| "MP4 大文件头损坏")?;
            size = u64::from_be_bytes(extended);
            header_size = 16;
        } else if size == 0 {
            size = length - offset;
        }
        if size < header_size || size > length - offset {
            return Err("MP4 文件被截断或块大小不正确".into());
        }
        match &header[4..8] {
            b"ftyp" => ftyp = true,
            b"moov" => moov = true,
            b"mdat" => mdat = true,
            _ => {}
        }
        offset += size;
        file.seek(SeekFrom::Start(offset))
            .map_err(|_| "MP4 读取失败")?;
    }
    if !(ftyp && moov && mdat) {
        return Err("MP4 缺少类型、媒体索引或数据块".into());
    }
    Ok(())
}
pub fn verify_attachment(a: &Attachment) -> Result<()> {
    let (size, hash) = hash_file(Path::new(&a.path))?;
    if size == 0 || size != a.size || hash != a.sha256 {
        return Err(format!("附件 {} 已修改或为空，请重新导入", a.name));
    }
    Ok(())
}
pub fn safe_name(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .take(120)
        .collect()
}
fn md(s: &str) -> String {
    s.replace('<', "&lt;").replace('>', "&gt;")
}
pub fn report(d: &Value, p: &Profile) -> String {
    let rows = [
        ("问题概述", "title"),
        ("前置条件", "preconditions"),
        ("预期结果", "expected"),
        ("实际结果", "actual"),
    ];
    let mut out = format!(
        "# {}\n\n项目：{}\n方向：{}\n\n",
        md(&string(d, "title")),
        md(&p.name),
        md(&string(d, "business"))
    );
    for (title, key) in rows {
        out.push_str(&format!("## {title}\n\n{}\n\n", md(&string(d, key))));
    }
    out.push_str("## 复现步骤\n\n");
    for (i, s) in string(d, "steps")
        .lines()
        .filter(|s| !s.trim().is_empty())
        .enumerate()
    {
        out.push_str(&format!("{}. {}\n", i + 1, md(s)));
    }
    out.push_str(&format!("\n## 复现频率\n\n{}\n\n## 问题时间\n\n事件：{}\n原始标记：{}\n来源：{}\n事后补采：{}\n依据：{}\n\n## 应用与设备\n\n应用：{}\n\n设备：{}\n\n人工补充来源：{}\n\n## Jenkins 固定构建\n\n{}\n身份状态：{}\n\n## 有效云控\n\n{}\n\n## 采集缺口\n\n{}\n",if bool_at(d,"frequency.firstDiscovery"){ "首次发现、尚未重复".into() }else{format!("{} 次尝试，{} 次发生",string(d,"frequency.attempts"),string(d,"frequency.occurrences"))},md(&string(d,"event.occurredAt")),md(&string(d,"event.originalAt")),md(&string(d,"event.source")),bool_at(d,"event.lateCapture"),md(&string(d,"event.sourceNote")),md(&d["app"].to_string()),md(&d["device"].to_string()),md(&string(d,"manualNote")),md(&string(d,"buildUrl")),md(&string(d,"buildStatus")),md(&d["cloud"].to_string()),md(&d["collectionErrors"].to_string())));
    out
}
pub fn marker(id: &str) -> String {
    format!("<!-- android-bug-desk:{id}:start -->")
}
pub fn managed(e: &Evidence, uploads: &[Upload], complete: bool) -> String {
    let mut text = format!(
        "{}\n## 客户端证据\n\nevidenceId: {}\nschemaVersion: 1.0\n证据包 SHA256: {}\n状态: {}\n",
        marker(&e.id),
        e.id,
        e.sha256,
        if complete {
            "complete"
        } else {
            "附件上传中 / 证据未齐"
        }
    );
    for a in uploads {
        text.push_str(&format!(
            "\n- {} · SHA256 {} · 附件 ID {} · {}",
            md(&a.name),
            a.sha256,
            a.remote_id.as_deref().unwrap_or("未确认"),
            a.state
        ));
    }
    text.push_str(&format!("\n<!-- android-bug-desk:{}:end -->", e.id));
    text
}
pub fn merge_managed(
    description: &str,
    e: &Evidence,
    uploads: &[Upload],
    complete: bool,
) -> Result<String> {
    let start = marker(&e.id);
    let end = format!("<!-- android-bug-desk:{}:end -->", e.id);
    let a = description
        .find(&start)
        .ok_or("远端证据段落被修改，停止覆盖，请人工处理")?;
    let b = description[a..]
        .find(&end)
        .ok_or("远端证据段落不完整，停止覆盖")?
        + a
        + end.len();
    if description.matches(&start).count() != 1 || description.matches(&end).count() != 1 {
        return Err("远端证据段落重复，停止覆盖".into());
    }
    Ok(format!(
        "{}{}{}",
        &description[..a],
        managed(e, uploads, complete),
        &description[b..]
    ))
}
pub fn build(store: &Store, d: &Value, p: &Profile, include_media: bool) -> Result<Evidence> {
    if let Some(error) = crate::validation::receipt_error(&attachments(d)?) {
        return Err(error);
    }
    let id = uuid();
    let dir = store.root.join("evidence").join(&id);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut files = Vec::new();
    for mut a in attachments(d)?.into_iter().filter(|a| a.confirmed) {
        verify_attachment(&a)?;
        let sub = if ["image", "video"].contains(&a.role.as_str()) {
            "media"
        } else {
            "logs"
        };
        let relative = format!("{sub}/{}-{}", a.id, safe_name(&a.name));
        let dest = dir.join(&relative);
        fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::copy(&a.path, &dest).map_err(|e| e.to_string())?;
        a.path = dest.to_string_lossy().into();
        files.push(a);
    }
    let snapshot:Value=store.get("snapshot",&string(d,"id"))?.unwrap_or_else(||json!({"device":d["device"],"app":d["app"],"details":null,"deviceTime":null,"capturedAt":null}));
    let manifest = json!({"schemaVersion":"1.0","evidenceId":id,"sessionId":d["sessionId"],"clientVersion":env!("CARGO_PKG_VERSION"),"createdAt":now(),
        "scope":{"organizationId":p.organization_id,"projectId":p.project_id,"businessProfile":d["business"],"versions":d["versions"],"sprint":d["sprint"],"multicaParentTaskId":null},
        "build":{"url":d["buildUrl"],"parsed":crate::validation::fixed_build_url(&string(d,"buildUrl")),"identityStatus":d["buildStatus"],"manifest":d["buildManifest"],"mappingStatus":"not_collected","parameters":null},
        "installedArtifact":{"identity":d["app"],"fingerprints":null,"status":"not_collected"},"ciArtifacts":{"items":null,"status":"not_collected"},
        "problem":{"kind":d["kind"],"title":d["title"],"preconditions":d["preconditions"],"steps":d["steps"],"expected":d["expected"],"actual":d["actual"],"frequency":d["frequency"],"event":d["event"],"network":d["network"],"data":d["data"]},"device":{"effective":d["device"],"snapshot":snapshot},"cloudConfig":d["cloud"],"collection":d["sources"],"gaps":d["collectionErrors"],
        "files":files.iter().map(|a|json!({"id":a.id,"relativePath":Path::new(&a.path).strip_prefix(&dir).unwrap().to_string_lossy(),"mime":a.mime,"size":a.size,"sha256":a.sha256,"role":a.role,"source":a.source,"derivedFrom":a.derived_from,"includedInZip":include_media || !["image","video"].contains(&a.role.as_str()),"coverage":{"start":a.coverage_start,"end":a.coverage_end}})).collect::<Vec<_>>()});
    for (name, value) in [
        ("manifest.json", manifest),
        (
            "device.json",
            json!({"effective":d["device"],"snapshot":snapshot}),
        ),
        ("app.json", d["app"].clone()),
        ("cloud-config.json", d["cloud"].clone()),
        ("collection-errors.json", d["collectionErrors"].clone()),
    ] {
        fs::write(dir.join(name), serde_json::to_vec_pretty(&value).unwrap())
            .map_err(|e| e.to_string())?;
    }
    fs::write(dir.join("report.md"), report(d, p)).map_err(|e| e.to_string())?;
    let zip_path = dir.with_extension("zip");
    let f = File::create(&zip_path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(f);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for name in [
        "manifest.json",
        "device.json",
        "app.json",
        "cloud-config.json",
        "collection-errors.json",
        "report.md",
    ] {
        zip.start_file(format!("{id}/{name}"), opts)
            .map_err(|e| e.to_string())?;
        zip.write_all(&fs::read(dir.join(name)).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    }
    for a in &files {
        if !include_media && ["image", "video"].contains(&a.role.as_str()) {
            continue;
        }
        let relative = Path::new(&a.path)
            .strip_prefix(&dir)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        zip.start_file(format!("{id}/{relative}"), opts)
            .map_err(|e| e.to_string())?;
        std::io::copy(
            &mut File::open(&a.path).map_err(|e| e.to_string())?,
            &mut zip,
        )
        .map_err(|e| e.to_string())?;
    }
    zip.finish()
        .map_err(|e| e.to_string())?
        .sync_all()
        .map_err(|e| e.to_string())?;
    let (_, sha256) = hash_file(&zip_path)?;
    Ok(Evidence {
        id,
        directory: dir.to_string_lossy().into(),
        zip_path: zip_path.to_string_lossy().into(),
        sha256,
        files,
    })
}

#[cfg(test)]
mod gap_tests {
    use super::*;
    use crate::adb::ProblemWindow;
    const MARK: &str = "2026-10-09T04:00:00Z";
    const END: &str = "2026-10-09T04:00:30Z";

    fn log(root: &Path, name: &str, lines: &str) -> Attachment {
        let path = root.join(name);
        fs::write(&path, lines).unwrap();
        inspect(&path, "log", "fake ADB epoch fixture", root).unwrap()
    }
    fn line(at: &str, offset: i64, message: &str) -> String {
        let epoch = chrono::DateTime::parse_from_rfc3339(at)
            .unwrap()
            .timestamp_micros()
            + offset;
        format!(
            "{}.{:06} 1000 123 456 I App: {message}\n",
            epoch / 1_000_000,
            epoch % 1_000_000
        )
    }
    fn full_log(root: &Path) -> Attachment {
        log(
            root,
            "raw.log",
            &[
                line("2026-10-09T03:57:59Z", 0, "before"),
                line("2026-10-09T03:58:00Z", 0, "start"),
                line(MARK, 0, "old PID"),
                line("2026-10-09T04:00:01Z", 0, "new PID").replace("123 456", "987 654"),
                line(END, 0, "end"),
                line("2026-10-09T04:00:31Z", 0, "after"),
            ]
            .concat(),
        )
    }
    fn capture(
        root: &Path,
        raw: &[Attachment],
        observed: &str,
        offset: Option<i64>,
        gaps: &[LogGap],
    ) -> LogWindowCapture {
        summarize_log_window(
            root,
            raw,
            &ProblemWindow::new(MARK).unwrap(),
            observed,
            offset,
            "fixture clock alignment",
            gaps,
        )
        .unwrap()
    }

    #[test]
    fn window_has_exact_requested_bounds_actual_coverage_and_no_pid_filter() {
        let temp = tempfile::tempdir().unwrap();
        let raw = full_log(temp.path());
        let original = fs::read(&raw.path).unwrap();
        let a = capture(temp.path(), std::slice::from_ref(&raw), END, Some(0), &[]);
        assert_eq!(a.summary.requested_start, "2026-10-09T03:58:00+00:00");
        assert_eq!(a.summary.requested_end, "2026-10-09T04:00:30+00:00");
        assert!(a.summary.range_covered);
        assert_eq!(a.summary.selected_records, 4);
        assert_eq!(a.attachment.coverage_start, a.summary.requested_start);
        assert_eq!(a.attachment.coverage_end, a.summary.requested_end);
        assert_eq!(a.attachment.derived_from, Some(raw.id.clone()));
        let text = fs::read_to_string(&a.attachment.path).unwrap();
        assert!(text.contains("old PID") && text.contains("new PID"));
        assert!(!text.contains("App: before") && !text.contains("App: after"));
        assert_eq!(fs::read(&raw.path).unwrap(), original);
        verify_attachment(&raw).unwrap();
        let b = capture(temp.path(), std::slice::from_ref(&raw), END, Some(0), &[]);
        assert_eq!(a.attachment.sha256, b.attachment.sha256);
        let receipt: Value = serde_json::from_slice(&fs::read(a.summary_path).unwrap()).unwrap();
        assert_eq!(receipt["sources"][0]["sha256"], raw.sha256);
    }

    #[test]
    fn window_never_fakes_short_pre_or_post_coverage_or_unknown_clock() {
        let temp = tempfile::tempdir().unwrap();
        let raw = log(
            temp.path(),
            "short.log",
            &line(MARK, 0, "only problem record"),
        );
        let a = capture(
            temp.path(),
            std::slice::from_ref(&raw),
            "2026-10-09T04:00:10Z",
            Some(0),
            &[],
        );
        assert!(!a.summary.range_covered);
        assert_eq!(a.attachment.coverage_start, "2026-10-09T04:00:00+00:00");
        assert_eq!(a.attachment.coverage_end, a.attachment.coverage_start);
        assert_eq!(a.summary.limitations.len(), 3);
        let raw = full_log(temp.path());
        let unknown = capture(temp.path(), &[raw], END, None, &[]);
        assert!(!unknown.summary.range_covered);
        assert_eq!(unknown.summary.selected_records, 0);
        assert!(unknown.attachment.coverage_start.is_empty());
        assert!(unknown.attachment.coverage_end.is_empty());
        assert!(unknown
            .summary
            .limitations
            .iter()
            .any(|v| v.contains("对应未采集")));
    }

    #[test]
    fn window_records_intersecting_disconnect_and_unknown_gaps() {
        let temp = tempfile::tempdir().unwrap();
        let raw = full_log(temp.path());
        let gaps = [
            LogGap {
                start: Some("2026-10-09T03:59:00Z".into()),
                end: Some("2026-10-09T03:59:10Z".into()),
                reason: "USB-A disconnected; no switch to USB-B".into(),
            },
            LogGap {
                start: Some("2026-10-09T03:50:00Z".into()),
                end: Some("2026-10-09T03:51:00Z".into()),
                reason: "outside".into(),
            },
            LogGap {
                start: None,
                end: None,
                reason: "legacy untimed collection failure".into(),
            },
        ];
        let a = capture(temp.path(), &[raw], END, Some(0), &gaps);
        assert!(!a.summary.range_covered);
        assert_eq!(a.summary.gaps.len(), 2);
        assert!(a.summary.gaps[0].reason.contains("USB-A"));
        assert!(a.summary.limitations.is_empty());
    }

    #[test]
    fn window_applies_explicit_device_clock_offset_and_tracks_multiple_sources() {
        let temp = tempfile::tempdir().unwrap();
        let offset = 5_000_000;
        let first = log(
            temp.path(),
            "existing.log",
            &line("2026-10-09T03:58:00Z", offset, "initial buffer"),
        );
        let second = log(temp.path(), "live.log", &line(END, offset, "live stream"));
        let a = capture(temp.path(), &[first, second], END, Some(offset), &[]);
        assert!(a.summary.range_covered);
        assert_eq!(a.summary.sources.len(), 2);
        assert!(a.attachment.derived_from.is_none());
        assert_eq!(a.attachment.coverage_end, "2026-10-09T04:00:30+00:00");
    }

    #[test]
    fn window_marks_malformed_truncated_and_backwards_records_as_uncertain() {
        let temp = tempfile::tempdir().unwrap();
        let raw = full_log(temp.path());
        let mut text = fs::read_to_string(&raw.path).unwrap();
        text.push_str("unexpected log format\n");
        text.push_str(line(MARK, 0, "backwards and truncated").trim_end());
        let uncertain = log(temp.path(), "uncertain.log", &text);
        let a = capture(temp.path(), &[uncertain], END, Some(0), &[]);
        assert!(!a.summary.range_covered);
        assert!(a.summary.limitations.iter().any(|v| v.contains("无法解析")));
        assert!(a.summary.limitations.iter().any(|v| v.contains("换行")));
        assert!(a.summary.limitations.iter().any(|v| v.contains("倒退")));
        fs::write(&raw.path, "changed").unwrap();
        assert!(summarize_log_window(
            temp.path(),
            &[raw],
            &ProblemWindow::new(MARK).unwrap(),
            END,
            Some(0),
            "fixture",
            &[]
        )
        .is_err());
    }

    #[test]
    fn epoch_parser_handles_fractional_precision_without_float_rounding() {
        assert_eq!(epoch_micros("123.1 1 I x"), Some(123_100_000));
        assert_eq!(epoch_micros("123.123456789 1 I x"), Some(123_123_456));
        for text in [
            "123 x",
            "-1.0 x",
            "1.nan x",
            ".1 x",
            "1. x",
            "999999999999999999999.0 x",
        ] {
            assert!(epoch_micros(text).is_none());
        }
    }

    #[test]
    fn redaction_preserves_original_lineage_hashes_and_precise_rule_counts() {
        let temp = tempfile::tempdir().unwrap();
        let mut original = log(
            temp.path(),
            "sensitive.log",
            "email=user@example.test token=a.b token=a.b\nuntargeted ip=10.0.0.1\n",
        );
        original.coverage_start = MARK.into();
        original.coverage_end = END.into();
        let bytes = fs::read(&original.path).unwrap();
        let rules = [
            TextRedactionRule {
                label: "test email".into(),
                literal: "user@example.test".into(),
            },
            TextRedactionRule {
                label: "test token".into(),
                literal: "a.b".into(),
            },
        ];
        let a = redact_text(temp.path(), &original, &rules).unwrap();
        let b = redact_text(temp.path(), &original, &rules).unwrap();
        assert_eq!(a.attachment.sha256, b.attachment.sha256);
        assert_eq!(a.attachment.derived_from, Some(original.id.clone()));
        assert_eq!(a.attachment.coverage_start, MARK);
        assert!(!a.attachment.confirmed);
        assert_eq!(a.receipt["rules"][0]["matches"], 1);
        assert_eq!(a.receipt["rules"][1]["matches"], 2);
        assert_eq!(a.receipt["completeRedaction"], false);
        assert_eq!(a.receipt["originalSha256"], original.sha256);
        assert!(!a.receipt.to_string().contains("user@example.test"));
        assert!(!a.receipt.to_string().contains("a.b"));
        let text = fs::read_to_string(a.attachment.path).unwrap();
        assert_eq!(text.matches("[REDACTED]").count(), 3);
        assert!(text.contains("ip=10.0.0.1"));
        assert_eq!(fs::read(&original.path).unwrap(), bytes);
        verify_attachment(&original).unwrap();
        let receipt: Value = serde_json::from_slice(&fs::read(a.receipt_path).unwrap()).unwrap();
        assert_eq!(receipt, a.receipt);
    }

    #[test]
    fn redaction_escapes_literals_and_prefers_longest_match_without_recursion() {
        let temp = tempfile::tempdir().unwrap();
        let original = log(temp.path(), "overlap.log", "abc ab axb a.b [REDACTED]\n");
        let rules = [
            TextRedactionRule {
                label: "short".into(),
                literal: "ab".into(),
            },
            TextRedactionRule {
                label: "long".into(),
                literal: "abc".into(),
            },
            TextRedactionRule {
                label: "literal dot".into(),
                literal: "a.b".into(),
            },
            TextRedactionRule {
                label: "existing marker".into(),
                literal: "[REDACTED]".into(),
            },
        ];
        let a = redact_text(temp.path(), &original, &rules).unwrap();
        assert_eq!(
            fs::read_to_string(a.attachment.path).unwrap(),
            "[REDACTED] [REDACTED] axb [REDACTED] [REDACTED]\n"
        );
        for rule in a.receipt["rules"].as_array().unwrap() {
            assert_eq!(rule["matches"], 1);
        }
    }

    #[test]
    fn redaction_blocks_invalid_text_rules_and_changed_originals() {
        let temp = tempfile::tempdir().unwrap();
        let original = log(temp.path(), "raw.log", "log\n");
        assert!(redact_text(temp.path(), &original, &[]).is_err());
        let rules = [TextRedactionRule {
            label: "empty literal".into(),
            literal: String::new(),
        }];
        assert!(redact_text(temp.path(), &original, &rules).is_err());
        fs::write(&original.path, "changed").unwrap();
        let rules = [TextRedactionRule {
            label: "test".into(),
            literal: "log".into(),
        }];
        assert!(redact_text(temp.path(), &original, &rules).is_err());
        let original = log(temp.path(), "invalid-utf8.log", "valid prefix\n");
        let mut bytes = vec![b'x'; 4096];
        bytes.push(0xff);
        fs::write(&original.path, bytes).unwrap();
        let mut original = original;
        (original.size, original.sha256) = hash_file(Path::new(&original.path)).unwrap();
        assert!(redact_text(temp.path(), &original, &rules).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn derived_outputs_reject_symlink_escape_for_every_category() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        for category in ["attachments", "bugreports", "log-windows", "redactions"] {
            std::os::unix::fs::symlink(outside.path(), temp.path().join(category)).unwrap();
            assert!(derived_directory(temp.path(), category).is_err());
        }
        assert!(derived_directory(temp.path(), "../escape").is_err());
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    }
}
