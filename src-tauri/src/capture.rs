//! Durable, local-only evidence jobs. Completion is separate from actual timestamp coverage.
use crate::{
    adb::{ProblemWindow, WindowWait},
    evidence,
    model::*,
    store::Store,
};
use serde_json::{json, Value};
use std::{io::Write, path::Path};

pub fn pending_kind(store: &Store, draft_id: &str, kind: &str) -> Result<bool> {
    Ok(store.list("capture")?.iter().any(|t| {
        string(t, "draftId") == draft_id
            && string(t, "kind") == kind
            && string(t, "status") == "pending"
    }))
}
pub fn pending(store: &Store, draft_id: &str) -> Result<bool> {
    Ok(store
        .list("capture")?
        .iter()
        .any(|t| string(t, "draftId") == draft_id && string(t, "status") == "pending"))
}
pub fn recover(store: &Store) -> Result<()> {
    for mut t in store.list("capture")? {
        if string(&t, "status") == "pending" {
            t["status"] = json!("interrupted");
            t["error"] = json!("客户端退出中断采集；不得按重新打开时间补足覆盖范围");
            t["recoveredAt"] = json!(now());
            store.put("capture", &string(&t, "id"), &t)?;
        }
    }
    Ok(())
}
pub fn paired_report(
    store: &Store,
    receipt: &Path,
    attachment_id: &str,
    source: &str,
) -> Result<Attachment> {
    let dir = evidence::derived_directory(&store.root, "attachments")?;
    let path = dir.join("evidence-receipt.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).map_err(|e| e.to_string())?);
    zip.start_file("receipt.json", zip::write::SimpleFileOptions::default())
        .map_err(|e| e.to_string())?;
    zip.write_all(&std::fs::read(receipt).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    zip.finish().map_err(|e| e.to_string())?;
    let mut a = evidence::inspect(&path, "report", source, &store.root)?;
    a.derived_from = Some(attachment_id.into());
    Ok(a)
}
pub fn schedule(
    store: &Store,
    draft: &Value,
    session_id: &str,
    marked: &str,
    offset: Option<i64>,
    clock_note: &str,
    cancelled: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let draft_id = string(draft, "id");
    let window = ProblemWindow::new(marked)?;
    let task = json!({"id":uuid(),"draftId":draft_id,"sessionId":session_id,"kind":"log-window","status":"pending","window":window,"offsetMicros":offset,"clockNote":clock_note,"observedUntil":marked});
    store.put_many(&[
        ("draft", &draft_id, draft),
        ("capture", &string(&task, "id"), &task),
    ])?;
    let store = store.clone();
    tauri::async_runtime::spawn(async move {
        let wait = window.wait_ready(cancelled).await;
        let interrupted = !matches!(wait, Ok(WindowWait::Ready));
        // A stopped collector may still be flushing. Wait only for its bounded local finish.
        if interrupted {
            for _ in 0..50 {
                if store
                    .get::<Value>("session", session_id_from(&task))
                    .ok()
                    .flatten()
                    .is_some_and(|s| !s["stoppedAt"].is_null())
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }
        let _write = store.writes.lock().unwrap();
        let mut task = task;
        task["observedUntil"] = json!(now());
        if let Err(e) = finish(&store, &mut task, interrupted) {
            task["status"] = json!("failed");
            task["error"] = json!(e);
            let _ = store.put("capture", &string(&task, "id"), &task);
        }
    });
    Ok(())
}
fn session_id_from(task: &Value) -> &str {
    task["sessionId"].as_str().unwrap_or("")
}
/// Caller holds Store.writes. Preserve the most recent editable draft rather than a timer's snapshot.
pub(crate) fn finish(store: &Store, task: &mut Value, interrupted: bool) -> Result<()> {
    let id = string(task, "draftId");
    store.editable(&id)?;
    let session: Value = store
        .get("session", session_id_from(task))?
        .ok_or("采集会话不存在")?;
    let window: ProblemWindow =
        serde_json::from_value(task["window"].clone()).map_err(|e| e.to_string())?;
    let interrupted = interrupted || !session["stoppedAt"].is_null();
    let last = string(task, "observedUntil");
    let stopped = string(&session, "stoppedAt");
    let observed = if !session["stoppedAt"].is_null()
        && chrono::DateTime::parse_from_rfc3339(&stopped).ok()
            < chrono::DateTime::parse_from_rfc3339(&last).ok()
    {
        stopped
    } else {
        last
    };
    let mut raw = Vec::new();
    let root = store.root.canonicalize().map_err(|e| e.to_string())?;
    for path in session["logFiles"].as_array().ok_or("会话日志列表无效")? {
        let path = Path::new(path.as_str().ok_or("日志路径无效")?);
        let real = path.canonicalize().map_err(|e| e.to_string())?;
        if !real.starts_with(&root) {
            return Err("会话日志不在本地证据目录".into());
        }
        if real.metadata().is_ok_and(|m| m.len() > 0) {
            let mut attachment = evidence::inspect(
                &real,
                "log",
                "ADB 原始日志固定副本（未自动选择上传）",
                &root,
            )?;
            if evidence::apply_recorded_log_coverage(&mut attachment)? {
                attachment.source =
                    "ADB 原始日志固定副本（已按日志内首条和末条记录自动填写覆盖时间；未自动选择上传）"
                        .into();
            } else {
                attachment.source =
                    "ADB 原始日志固定副本（未识别到可用时间边界，需人工确认；未自动选择上传）"
                        .into();
            }
            raw.push(attachment);
        }
    }
    let mut gaps: Vec<evidence::LogGap> = session["gaps"]
        .as_array()
        .ok_or("日志缺口记录无效")?
        .iter()
        .map(|v| evidence::LogGap {
            start: None,
            end: None,
            reason: v.as_str().unwrap_or("未知日志缺口").into(),
        })
        .collect();
    if interrupted {
        gaps.push(evidence::LogGap {
            start: Some(observed.clone()),
            end: Some(window.due_at.clone()),
            reason: "采集提前停止或客户端中断；未观察到完整问题后窗口".into(),
        });
    }
    let capture = evidence::summarize_log_window(
        &root,
        &raw,
        &window,
        &observed,
        task["offsetMicros"].as_i64(),
        &string(task, "clockNote"),
        &gaps,
    )?;
    let report = paired_report(
        store,
        &capture.summary_path,
        &capture.attachment.id,
        "日志窗口范围与缺口记录",
    )?;
    let mut d = store.draft(&id)?;
    let rows = d["attachments"].as_array_mut().ok_or("草稿附件无效")?;
    rows.extend(raw.into_iter().map(|a| serde_json::to_value(a).unwrap()));
    rows.push(serde_json::to_value(&capture.attachment).unwrap());
    rows.push(serde_json::to_value(report).unwrap());
    d["previewConfirmed"] = json!(false);
    d["updatedAt"] = json!(now());
    task["status"] = json!(if interrupted {
        "interrupted"
    } else {
        "complete"
    });
    task["finishedAt"] = json!(now());
    task["summary"] = serde_json::to_value(capture.summary).map_err(|e| e.to_string())?;
    task["windowAttachmentId"] = json!(capture.attachment.id);
    store.put_many(&[("draft", &id, &d), ("capture", &string(task, "id"), task)])
}
pub fn retry_interrupted(store: &Store, draft_id: &str) -> Result<()> {
    let mut found = false;
    for mut t in store.list("capture")? {
        if string(&t, "draftId") == draft_id && string(&t, "kind") == "log-window" {
            found = true;
            if string(&t, "status") == "interrupted" && t["summary"].is_null() {
                // The last durable observation is the problem mark, never the restart time.
                finish(store, &mut t, true)?;
            }
        }
    }
    if !found {
        return Err("请先记录问题，以绑定日志时间窗口".into());
    }
    Ok(())
}
