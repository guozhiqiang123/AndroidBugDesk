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
    let id = uuid();
    let name = path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("attachment")
        .to_string();
    let destination = root.join("attachments").join(&id).join(safe_name(&name));
    fs::create_dir_all(destination.parent().unwrap()).map_err(|e| e.to_string())?;
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
