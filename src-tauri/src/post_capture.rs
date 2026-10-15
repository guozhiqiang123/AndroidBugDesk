//! After-the-fact diagnostics. Never mark a new incident or mutate app identity.
use crate::{adb, commands::AppState, evidence, model::*, workspace};
use serde_json::{json, Value};
use std::{collections::HashSet, io::Write, path::Path, sync::atomic::Ordering};
use tauri::{Manager, State};

#[cfg(test)]
#[path = "post_capture_tests.rs"]
mod tests;

pub const ITEMS: &[&str] = &[
    "environment",
    "logs",
    "screenshot",
    "ui",
    "app",
    "display",
    "locale",
    "crash",
    "exit",
    "memory",
    "frames",
    "cpu",
    "network",
    "storage",
    "bugreport",
];

fn item_name(item: &str) -> &'static str {
    match item {
        "environment" => "设备与应用环境",
        "logs" => "现存日志",
        "screenshot" => "当前截图",
        "ui" => "当前界面层级",
        "app" => "应用与进程状态",
        "display" => "窗口与显示状态",
        "locale" => "语言与字体配置",
        "crash" => "崩溃日志",
        "exit" => "历史进程退出记录",
        "memory" => "当前应用内存",
        "frames" => "现存帧统计",
        "cpu" => "CPU与进程状态",
        "network" => "网络配置与统计",
        "storage" => "存储空间与权限",
        "bugreport" => "系统报告",
        _ => "未知采集项",
    }
}

fn scope(state: &AppState, id: &str) -> Result<String> {
    let current = workspace::current_scope(state, "")?;
    if state.store.get::<String>("draft_scope", id)?.as_deref() != Some(&current) {
        return Err("草稿不属于当前账号或项目".into());
    }
    Ok(current)
}
fn binding(state: &AppState, job: &Value) -> Result<()> {
    let id = string(job, "draftId");
    state.editable(&id)?;
    if state.workspace_epoch.load(Ordering::SeqCst)
        != job["workspaceEpoch"].as_u64().unwrap_or(u64::MAX)
        || scope(state, &id)? != string(job, "scope")
    {
        return Err("账号或项目已变化，补采已停止；已保存的证据保留".into());
    }
    if crate::project_settings::current_package_for_state(state)? != string(job, "targetPackage")
        || string(&state.store.draft(&id)?, "app.packageName") != string(job, "targetPackage")
    {
        return Err("目标包名已变化，补采已停止；已保存的证据保留".into());
    }
    Ok(())
}
async fn connected(path: &Path, serial: &str) -> Result<()> {
    adb::valid_serial(serial)?;
    if serial.starts_with('-') {
        return Err("设备序列号不正确".into());
    }
    let devices = adb::devices(path).await?;
    match devices
        .as_array()
        .and_then(|rows| rows.iter().find(|row| string(row, "serial") == serial))
        .map(|row| string(row, "state"))
        .as_deref()
    {
        Some("device") => Ok(()),
        Some("unauthorized") => Err("请在手机上允许 USB 调试授权".into()),
        Some("offline") => Err("设备离线，请重新连接 USB 后刷新设备".into()),
        _ => Err("所选设备已断开，请刷新设备列表".into()),
    }
}
pub(crate) async fn prepare(
    state: &AppState,
    id: &str,
    serial: &str,
    items: Vec<String>,
    retry_of: Option<String>,
) -> Result<Value> {
    let epoch = state.workspace_epoch.load(Ordering::SeqCst);
    let original_scope = scope(state, id)?;
    state.editable(id)?;
    if items.is_empty()
        || items.len() > ITEMS.len()
        || items.iter().any(|item| !ITEMS.contains(&item.as_str()))
    {
        return Err("请选择有效的补采项目".into());
    }
    let unique: std::collections::HashSet<_> = items.iter().collect();
    if unique.len() != items.len() {
        return Err("补采项目不能重复".into());
    }
    let package = crate::project_settings::current_package_for_state(state)?;
    let path = state.adb_path()?;
    connected(&path, serial).await?;
    let target_uid = if items
        .iter()
        .any(|item| ["logs", "crash"].contains(&item.as_str()))
    {
        Some(adb::package_uid(&path, serial, &package).await?)
    } else {
        None
    };
    let _write = state.store.writes.lock().unwrap();
    state.editable(id)?;
    if epoch != state.workspace_epoch.load(Ordering::SeqCst) || scope(state, id)? != original_scope
    {
        return Err("账号或项目已变化，请重新开始补采".into());
    }
    if crate::capture::pending(&state.store, id)? {
        return Err("证据仍在采集中，请等待当前任务结束".into());
    }
    if state.collector.lock().unwrap().is_some() {
        return Err("请先停止本应用的实时采集会话，再补采证据".into());
    }
    if crate::project_settings::current_package_for_state(state)? != package {
        return Err("项目包名配置已变化，请重新开始补采".into());
    }
    let mut draft = state.store.draft(id)?;
    crate::project_settings::apply_current_package(state, &mut draft)?;
    state.store.put("draft", id, &draft)?;
    if let Some(ref previous_id) = retry_of {
        let previous: Value = state
            .store
            .get("capture", previous_id)?
            .ok_or("原补采任务不存在")?;
        if string(&previous, "kind") != "post-capture"
            || string(&previous, "draftId") != id
            || string(&previous, "scope") != original_scope
            || string(&previous, "serial") != serial
            || string(&previous, "targetPackage") != package
        {
            return Err("原补采任务的设备、应用或项目不一致".into());
        }
        for item in &items {
            let status = string(&previous["items"][item], "status");
            if !["failed", "unavailable", "partial", "interrupted"].contains(&status.as_str())
                && !(string(&previous, "status") == "interrupted"
                    && ["pending", "running"].contains(&status.as_str()))
            {
                return Err("只能重试未完整获取的采集项".into());
            }
        }
    }
    let mut selected = serde_json::Map::new();
    for item in items {
        selected.insert(
            item,
            json!({"status":"pending","attachments":[],"commands":[]}),
        );
    }
    let job = json!({"id":uuid(),"draftId":id,"kind":"post-capture","status":"pending","scope":original_scope,"workspaceEpoch":epoch,"serial":serial,"targetPackage":package,"targetUid":target_uid,"issueKind":draft["kind"],"bugOccurredAt":draft["event"]["occurredAt"],"startedAt":now(),"finishedAt":null,"retryOf":retry_of,"items":selected,"timeSemantics":"事后补采；当前状态不代表问题发生时状态；历史记录不保证连续覆盖"});
    state.store.put("capture", &string(&job, "id"), &job)?;
    Ok(job)
}
#[tauri::command]
pub async fn start_post_capture(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
    serial: String,
    items: Vec<String>,
    retry_of: Option<String>,
) -> Result<Value> {
    let job = prepare(&state, &id, &serial, items, retry_of).await?;
    let job_id = string(&job, "id");
    tauri::async_runtime::spawn(async move {
        let worker_app = app.clone();
        let worker_job_id = job_id.clone();
        let worker = tauri::async_runtime::spawn(async move {
            let state = worker_app.state::<AppState>();
            execute(&state, &worker_job_id).await
        });
        let failure = match worker.await {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(error),
            Err(error) => Some(format!("补采任务异常结束：{error}")),
        };
        if let Some(error) = failure {
            let state = app.state::<AppState>();
            let _ = interrupted(&state, &job_id, &error);
        }
    });
    Ok(job)
}
#[tauri::command]
pub fn post_capture_status(state: State<AppState>, id: String) -> Result<Value> {
    let current_scope = scope(&state, &id)?;
    let mut seen_items = HashSet::new();
    let jobs: Vec<Value> = state
        .store
        .list("capture")?
        .into_iter()
        .filter(|job| {
            string(job, "draftId") == id
                && string(job, "kind") == "post-capture"
                && string(job, "scope") == current_scope
        })
        .filter(|job| {
            let contributes = job["items"]
                .as_object()
                .is_some_and(|items| items.keys().any(|item| seen_items.insert(item.clone())));
            contributes || string(job, "status") == "pending"
        })
        .collect();
    Ok(json!({"draft":state.store.draft(&id)?,"jobs":jobs}))
}
fn interrupted(state: &AppState, job_id: &str, error: &str) -> Result<()> {
    let _write = state.store.writes.lock().unwrap();
    let Some(mut job) = state.store.get::<Value>("capture", job_id)? else {
        return Ok(());
    };
    for item in job["items"]
        .as_object_mut()
        .ok_or("补采任务格式错误")?
        .values_mut()
    {
        if ["pending", "running"].contains(&string(item, "status").as_str()) {
            item["status"] = json!("interrupted");
            item["error"] = json!(error);
        }
    }
    job["status"] = json!("interrupted");
    job["error"] = json!(error);
    job["finishedAt"] = json!(now());
    state.store.put("capture", job_id, &job)
}
fn zip_report(
    root: &Path,
    name: &str,
    files: &[(String, Vec<u8>)],
    receipt: &Value,
) -> Result<Attachment> {
    let path = root.join(format!(".post-capture-{}.zip", uuid()));
    let result = (|| -> Result<Attachment> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        let mut zip = zip::ZipWriter::new(file);
        for (name, bytes) in files {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .map_err(|e| e.to_string())?;
            zip.write_all(bytes).map_err(|e| e.to_string())?;
        }
        zip.start_file("receipt.json", zip::write::SimpleFileOptions::default())
            .map_err(|e| e.to_string())?;
        zip.write_all(&serde_json::to_vec_pretty(receipt).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        zip.finish().map_err(|e| e.to_string())?;
        let mut attachment = evidence::inspect(
            &path,
            "report",
            "事后补采诊断；时间及范围见 receipt.json",
            root,
        )?;
        attachment.name = name.into();
        let managed = Path::new(&attachment.path)
            .parent()
            .ok_or("补采附件目录无效")?
            .join(name);
        std::fs::rename(&attachment.path, &managed).map_err(|e| e.to_string())?;
        attachment.path = managed.to_string_lossy().into_owned();
        Ok(attachment)
    })();
    let _ = std::fs::remove_file(path);
    result
}
fn commands(item: &str, package: &str, target_uid: Option<u32>) -> Vec<Vec<String>> {
    let cmd = |args: &[&str]| args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let app = |args: &[&str]| {
        let mut c = cmd(args);
        c.push(package.into());
        c
    };
    match item {
        "environment" => vec![
            cmd(&["shell", "date", "+%Y-%m-%dT%H:%M:%S%z"]),
            cmd(&["shell", "getprop", "ro.product.manufacturer"]),
            cmd(&["shell", "getprop", "ro.product.model"]),
            cmd(&["shell", "getprop", "ro.build.version.release"]),
            cmd(&["shell", "getprop", "ro.build.version.sdk"]),
            cmd(&["shell", "getprop", "ro.build.fingerprint"]),
            cmd(&["shell", "getprop", "ro.product.cpu.abilist"]),
            cmd(&["shell", "getprop", "persist.sys.locale"]),
            cmd(&["shell", "getprop", "persist.sys.timezone"]),
            cmd(&["shell", "wm", "size"]),
            cmd(&["shell", "wm", "density"]),
            cmd(&["shell", "cat", "/proc/meminfo"]),
            cmd(&["shell", "df", "-k", "/data", "/sdcard"]),
            cmd(&["shell", "dumpsys", "battery"]),
            app(&["shell", "dumpsys", "package"]),
            cmd(&["shell", "settings", "get", "system", "font_scale"]),
            cmd(&[
                "shell",
                "cmd",
                "locale",
                "get-app-locales",
                package,
                "--user",
                "current",
            ]),
        ],
        "logs" if target_uid.is_some() => {
            let filter = format!("--uid={}", target_uid.unwrap());
            vec![logcat_command(item, &filter)]
        }
        "crash" if target_uid.is_some() => {
            let filter = format!("--uid={}", target_uid.unwrap());
            vec![logcat_command(item, &filter)]
        }
        "app" if !package.is_empty() => vec![
            app(&["shell", "dumpsys", "package"]),
            app(&["shell", "dumpsys", "activity", "processes"]),
        ],
        "exit" if !package.is_empty() => vec![app(&["shell", "dumpsys", "activity", "exit-info"])],
        "memory" if !package.is_empty() => vec![app(&["shell", "dumpsys", "meminfo"])],
        "frames" if !package.is_empty() => {
            vec![cmd(&["shell", "dumpsys", "gfxinfo", package, "framestats"])]
        }
        "cpu" => vec![
            cmd(&["shell", "dumpsys", "cpuinfo"]),
            cmd(&["shell", "ps", "-A"]),
        ],
        "display" => vec![
            cmd(&["shell", "dumpsys", "window"]),
            cmd(&["shell", "dumpsys", "input"]),
            cmd(&["shell", "wm", "size"]),
            cmd(&["shell", "wm", "density"]),
        ],
        "locale" => {
            let mut list = vec![
                cmd(&["shell", "getprop", "persist.sys.locale"]),
                cmd(&["shell", "settings", "get", "system", "font_scale"]),
            ];
            if !package.is_empty() {
                list.push(cmd(&[
                    "shell",
                    "cmd",
                    "locale",
                    "get-app-locales",
                    package,
                    "--user",
                    "current",
                ]));
            }
            list
        }
        "network" => vec![
            cmd(&["shell", "dumpsys", "connectivity"]),
            cmd(&["shell", "dumpsys", "netstats", "detail"]),
            cmd(&["shell", "settings", "get", "global", "http_proxy"]),
            cmd(&["shell", "settings", "get", "global", "private_dns_mode"]),
            cmd(&[
                "shell",
                "settings",
                "get",
                "global",
                "private_dns_specifier",
            ]),
        ],
        "storage" => {
            let mut list = vec![cmd(&["shell", "df", "-k", "/data", "/sdcard"])];
            if !package.is_empty() {
                list.push(app(&["shell", "dumpsys", "package"]));
            }
            list
        }
        _ => vec![],
    }
}

fn logcat_command(item: &str, filter: &str) -> Vec<String> {
    let mut args = vec![
        "logcat".into(),
        "-d".into(),
        "-v".into(),
        "epoch".into(),
        filter.into(),
        "-b".into(),
    ];
    if item == "crash" {
        args.push("crash".into());
    } else {
        args.extend(
            ["main", "-b", "system", "-b", "events"]
                .into_iter()
                .map(str::to_string),
        );
    }
    args
}

fn unfiltered_logcat_command(item: &str) -> Vec<String> {
    let mut args = vec![
        "logcat".into(),
        "-d".into(),
        "-v".into(),
        "threadtime".into(),
        "-b".into(),
    ];
    if item == "crash" {
        args.push("crash".into());
    } else {
        args.extend(
            ["main", "-b", "system", "-b", "events"]
                .into_iter()
                .map(str::to_string),
        );
    }
    args
}

fn unsupported_log_filter(args: &[String], error: &str) -> bool {
    let has_filter = args
        .iter()
        .any(|argument| argument.starts_with("--uid=") || argument.starts_with("--pid="));
    let lower = error.to_ascii_lowercase();
    has_filter
        && (lower.contains("unrecognized option")
            || lower.contains("unknown option")
            || lower.contains("invalid option"))
}

fn package_pids(output: &[u8]) -> Vec<u32> {
    let mut pids = output
        .split(|byte| byte.is_ascii_whitespace())
        .filter_map(|part| std::str::from_utf8(part).ok()?.parse::<u32>().ok())
        .filter(|pid| *pid > 0)
        .collect::<Vec<_>>();
    pids.sort_unstable();
    pids.dedup();
    pids.truncate(64);
    pids
}

fn package_pids_from_ps(output: &[u8], package: &str) -> Vec<u32> {
    let text = String::from_utf8_lossy(output);
    let mut lines = text.lines();
    let Some(header) = lines.next() else {
        return Vec::new();
    };
    let pid_column = header
        .split_whitespace()
        .position(|column| column.eq_ignore_ascii_case("PID"))
        .unwrap_or(1);
    let mut pids = lines
        .filter_map(|line| {
            let columns = line.split_whitespace().collect::<Vec<_>>();
            let process = columns.last().copied()?;
            (process == package || process.starts_with(&format!("{package}:")))
                .then(|| columns.get(pid_column)?.parse::<u32>().ok())
                .flatten()
        })
        .filter(|pid| *pid > 0)
        .collect::<Vec<_>>();
    pids.sort_unstable();
    pids.dedup();
    pids.truncate(64);
    pids
}

fn log_line_pid(line: &str) -> Option<u32> {
    let columns = line.split_whitespace().collect::<Vec<_>>();
    if columns.first()?.parse::<f64>().is_ok() {
        return columns.get(1)?.parse().ok();
    }
    let date = columns.first()?;
    if date.len() == 5 && date.as_bytes().get(2) == Some(&b'-') {
        return columns.get(2)?.parse().ok();
    }
    None
}

fn filter_logs_for_pids(output: &[u8], pids: &[u32]) -> Vec<u8> {
    let text = String::from_utf8_lossy(output);
    let mut filtered = String::new();
    let mut keep = false;
    for line in text.lines() {
        if let Some(pid) = log_line_pid(line) {
            keep = pids.contains(&pid);
        }
        if keep {
            filtered.push_str(line);
            filtered.push('\n');
        }
    }
    filtered.into_bytes()
}
fn unavailable(item: &str, output: &str) -> bool {
    let text = output.trim();
    if text.is_empty() {
        return true;
    }
    let lower = text.to_ascii_lowercase();
    if [
        "permission denial",
        "permission denied",
        "unknown command",
        "unknown option",
        "can't find service",
        "cannot find service",
        "bad activity command",
        "no process found",
        "unable to find package",
    ]
    .iter()
    .any(|s| lower.lines().take(4).any(|line| line.trim().starts_with(s)))
    {
        return true;
    }
    if ["logs", "crash"].contains(&item) {
        return log_bounds(text).is_none();
    }
    if item == "exit" {
        return !text.contains("ApplicationExitInfo #");
    }
    if item == "memory" {
        return !text.contains("MEMINFO") || text.contains("No process found");
    }
    false
}
fn log_bounds(text: &str) -> Option<Value> {
    let times: Vec<_> = text
        .lines()
        .filter_map(|line| line.split_whitespace().next()?.parse::<f64>().ok())
        .filter(|n| n.is_finite() && *n > 0.)
        .collect();
    let min = times.iter().copied().reduce(f64::min)?;
    let max = times.iter().copied().reduce(f64::max)?;
    let format = |n: f64| {
        chrono::DateTime::<chrono::Utc>::from_timestamp_micros((n * 1_000_000.) as i64)
            .map(|d| d.to_rfc3339())
    };
    Some(
        json!({"firstDeviceLogAt":format(min),"lastDeviceLogAt":format(max),"continuousCoverageKnown":false,"clock":"设备日志 epoch；未假定与电脑精确同步"}),
    )
}
async fn collect(
    state: &AppState,
    job: &Value,
    item: &str,
    path: &Path,
) -> Result<(Value, Option<Attachment>)> {
    let serial = string(job, "serial");
    let package = string(job, "targetPackage");
    let mut receipt = json!({"status":"running","startedAt":now(),"serial":serial,"targetPackage":package,"bugOccurredAt":job["bugOccurredAt"],"timeSemantics":job["timeSemantics"],"commands":[],"attachments":[]});
    if item == "bugreport" {
        let mut captured = adb::bugreport(
            path,
            &serial,
            &state.store.root,
            adb::BUGREPORT_TIMEOUT_SECONDS,
        )
        .await?;
        if let Some(attachment) = captured.attachment.as_mut() {
            attachment.source = format!(
                "事后补采系统报告；采集项：{}；目标应用 {package}；采集于 {}；不保证问题时刻的完整覆盖",
                item_name(item),
                captured.started_at
            );
        }
        receipt["finishedAt"] = json!(now());
        receipt["commands"] = json!([{ "argv":["-s",serial,"bugreport","<本机专用输出目录>"],"startedAt":captured.started_at,"finishedAt":captured.finished_at,"exitCode":captured.exit_code,"timedOut":captured.timed_out,"error":captured.failure }]);
        receipt["status"] = json!(if captured.attachment.is_some() {
            "complete"
        } else {
            "failed"
        });
        receipt["error"] = json!(captured.failure);
        let attachment = captured.attachment.take();
        if attachment.is_some() {
            // The validated attachment is already copied into its managed final
            // directory. The raw adb destination would otherwise keep a second
            // byte-for-byte copy of every successful whole-device report.
            let _ = std::fs::remove_dir_all(&captured.directory);
        }
        return Ok((receipt, attachment));
    }
    if item == "screenshot" {
        let destination = state
            .store
            .root
            .join(format!(".post-capture-screen-{}.png", uuid()));
        let compression = adb::screenshot(path, &serial, &destination).await?;
        let inspected = evidence::inspect(
            &destination,
            "image",
            &format!(
                "事后补采当前截图；保持原始分辨率并使用 PNG 无损压缩；采集项：{}；采集于 {}；不代表问题发生时画面",
                item_name(item),
                string(&receipt, "startedAt")
            ),
            &state.store.root,
        );
        let _ = std::fs::remove_file(&destination);
        let mut attachment = inspected?;
        attachment.name = "post-capture-current-screen.png".into();
        let managed = Path::new(&attachment.path)
            .parent()
            .ok_or("补采截图目录无效")?
            .join(&attachment.name);
        std::fs::rename(&attachment.path, &managed).map_err(|e| e.to_string())?;
        attachment.path = managed.to_string_lossy().into_owned();
        attachment.confirmed = false;
        receipt["status"] = json!("complete");
        receipt["finishedAt"] = json!(now());
        receipt["compression"] = json!({
            "format":"PNG",
            "mode":"lossless",
            "resolutionChanged":false,
            "width":compression.width,
            "height":compression.height,
            "originalBytes":compression.original_bytes,
            "storedBytes":compression.stored_bytes,
            "smallerEncodingUsed":compression.optimized
        });
        receipt["commands"] = json!([{"argv":["-s",serial,"exec-out","screencap","-p"],"startedAt":receipt["startedAt"],"finishedAt":receipt["finishedAt"]}]);
        return Ok((receipt, Some(attachment)));
    }
    let target_uid = job["targetUid"]
        .as_u64()
        .and_then(|uid| u32::try_from(uid).ok());
    if ["logs", "crash"].contains(&item) {
        let uid = target_uid.ok_or("目标应用 UID 缺失，已停止日志采集以避免保存整机日志")?;
        receipt["logFilter"] = json!({"mode":"androidUid","uid":uid,"packageName":package});
    }
    let mut plans = commands(item, &package, target_uid);
    let remote = format!("/sdcard/AndroidBugDesk-{}.xml", uuid());
    if item == "ui" {
        plans = vec![
            vec![
                "shell".into(),
                "uiautomator".into(),
                "dump".into(),
                remote.clone(),
            ],
            vec!["exec-out".into(), "cat".into(), remote.clone()],
        ];
    }
    let mut files = vec![];
    let mut observations = vec![];
    let mut missed = false;
    let mut interrupted = None;
    for (index, args) in plans.iter().enumerate() {
        if let Err(error) = binding(state, job) {
            interrupted = Some(error);
            break;
        }
        let started = now();
        let refs: Vec<_> = args.iter().map(String::as_str).collect();
        let output = adb::output(path, Some(&serial), &refs, 15).await;
        let mut argv = vec!["-s".to_string(), serial.clone()];
        argv.extend(args.iter().cloned());
        let mut observation = json!({"argv":argv,"startedAt":started,"finishedAt":now()});
        match output {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                let empty = unavailable(item, &text)
                    || (item == "ui" && index == 1 && !text.contains("<hierarchy"));
                observation["status"] = json!(if empty { "unavailable" } else { "complete" });
                if empty {
                    observation["error"] =
                        json!("未获取到有效诊断内容；可能无记录、权限不足或系统不支持");
                    missed = true;
                }
                if ["logs", "crash"].contains(&item) {
                    observation["logBounds"] = log_bounds(&text).unwrap_or(Value::Null);
                }
                if !bytes.is_empty() {
                    files.push((
                        format!(
                            "{index:02}-{}.{}",
                            item,
                            if item == "ui" && index == 1 {
                                "xml"
                            } else {
                                "txt"
                            }
                        ),
                        bytes,
                    ));
                }
            }
            Err(error) if unsupported_log_filter(args, &error) => {
                observation["status"] = json!("unavailable");
                observation["warning"] =
                    json!("设备 logcat 不支持 UID 过滤，已自动改用当前目标应用进程采集");
                observations.push(observation);

                let pid_args = vec!["shell".to_string(), "pidof".to_string(), package.clone()];
                let started = now();
                let pid_output = adb::output(
                    path,
                    Some(&serial),
                    &["shell", "pidof", package.as_str()],
                    5,
                )
                .await;
                let mut pid_argv = vec!["-s".to_string(), serial.clone()];
                pid_argv.extend(pid_args);
                let mut pid_observation = json!({"argv":pid_argv,"startedAt":started,"finishedAt":now(),"purpose":"resolveCurrentTargetProcesses"});
                let mut pids = match pid_output {
                    Ok(bytes) => {
                        let pids = package_pids(&bytes);
                        if pids.is_empty() {
                            pid_observation["status"] = json!("unavailable");
                            pid_observation["error"] =
                                json!("目标应用当前没有运行，无法在此设备上安全降级采集日志");
                        } else {
                            pid_observation["status"] = json!("complete");
                            pid_observation["processIds"] = json!(pids);
                        }
                        pids
                    }
                    Err(error) => {
                        pid_observation["status"] = json!("failed");
                        pid_observation["error"] = json!(error);
                        Vec::new()
                    }
                };
                observations.push(pid_observation);
                if pids.is_empty() {
                    let started = now();
                    let ps_output = adb::output(path, Some(&serial), &["shell", "ps"], 5).await;
                    let mut ps_observation = json!({
                        "argv":["-s",serial,"shell","ps"],
                        "startedAt":started,
                        "finishedAt":now(),
                        "purpose":"resolveCurrentTargetProcesses"
                    });
                    match ps_output {
                        Ok(bytes) => {
                            pids = package_pids_from_ps(&bytes, &package);
                            if pids.is_empty() {
                                ps_observation["status"] = json!("unavailable");
                                ps_observation["error"] =
                                    json!("目标应用当前没有运行，无法在此设备上安全降级采集日志");
                            } else {
                                ps_observation["status"] = json!("complete");
                                ps_observation["processIds"] = json!(pids);
                            }
                        }
                        Err(error) => {
                            ps_observation["status"] = json!("failed");
                            ps_observation["error"] = json!(error);
                        }
                    }
                    observations.push(ps_observation);
                }
                receipt["logFilter"]["fallbackMode"] = json!("currentProcessPid");
                receipt["logFilter"]["historicalCoverageLimited"] = json!(true);
                receipt["logFilter"]["processIds"] = json!(pids);

                let mut requires_local_filter = false;
                let mut compatibility_complete = !pids.is_empty();
                for pid in &pids {
                    if let Err(error) = binding(state, job) {
                        interrupted = Some(error);
                        break;
                    }
                    let fallback_args = logcat_command(item, &format!("--pid={pid}"));
                    let started = now();
                    let refs = fallback_args.iter().map(String::as_str).collect::<Vec<_>>();
                    let fallback_output = adb::output(path, Some(&serial), &refs, 15).await;
                    let mut fallback_argv = vec!["-s".to_string(), serial.clone()];
                    fallback_argv.extend(fallback_args.iter().cloned());
                    let mut fallback_observation = json!({
                        "argv":fallback_argv,
                        "startedAt":started,
                        "finishedAt":now(),
                        "compatibilityFallback":"currentProcessPid"
                    });
                    match fallback_output {
                        Ok(bytes) => {
                            let text = String::from_utf8_lossy(&bytes);
                            let empty = unavailable(item, &text);
                            fallback_observation["status"] =
                                json!(if empty { "unavailable" } else { "complete" });
                            fallback_observation["logBounds"] =
                                log_bounds(&text).unwrap_or(Value::Null);
                            if empty {
                                compatibility_complete = false;
                                fallback_observation["error"] = json!(
                                    "当前目标应用进程没有可用日志；历史日志未纳入本次兼容采集"
                                );
                            }
                            if !bytes.is_empty() {
                                let file_index = observations.len();
                                files.push((format!("{file_index:02}-{item}.txt"), bytes));
                            }
                        }
                        Err(error) => {
                            compatibility_complete = false;
                            fallback_observation["status"] = json!("failed");
                            if unsupported_log_filter(&fallback_args, &error) {
                                fallback_observation["warning"] = json!(
                                    "设备 logcat 不支持 PID 过滤，已改用本机内存过滤当前进程日志"
                                );
                                requires_local_filter = true;
                            } else {
                                fallback_observation["error"] = json!(error);
                            }
                        }
                    }
                    observations.push(fallback_observation);
                    if requires_local_filter {
                        break;
                    }
                }
                if requires_local_filter && !pids.is_empty() {
                    compatibility_complete = false;
                    let fallback_args = unfiltered_logcat_command(item);
                    let started = now();
                    let refs = fallback_args.iter().map(String::as_str).collect::<Vec<_>>();
                    let fallback_output = adb::output(path, Some(&serial), &refs, 15).await;
                    let mut fallback_argv = vec!["-s".to_string(), serial.clone()];
                    fallback_argv.extend(fallback_args.iter().cloned());
                    let mut fallback_observation = json!({
                        "argv":fallback_argv,
                        "startedAt":started,
                        "finishedAt":now(),
                        "compatibilityFallback":"localCurrentProcessPid",
                        "rawDeviceLogPersisted":false
                    });
                    match fallback_output {
                        Ok(bytes) => {
                            let filtered = filter_logs_for_pids(&bytes, &pids);
                            if filtered.is_empty() {
                                fallback_observation["status"] = json!("unavailable");
                                fallback_observation["error"] = json!(
                                    "当前目标应用进程没有可用日志；其他应用日志已丢弃且未保存"
                                );
                            } else {
                                fallback_observation["status"] = json!("complete");
                                compatibility_complete = true;
                                fallback_observation["logBounds"] = Value::Null;
                                let file_index = observations.len();
                                files.push((format!("{file_index:02}-{item}.txt"), filtered));
                            }
                            receipt["logFilter"]["fallbackMode"] = json!("localCurrentProcessPid");
                        }
                        Err(error) => {
                            fallback_observation["status"] = json!("failed");
                            fallback_observation["error"] = json!(error);
                        }
                    }
                    observations.push(fallback_observation);
                }
                if !compatibility_complete {
                    missed = true;
                }
                continue;
            }
            Err(error) => {
                missed = true;
                observation["status"] = json!("failed");
                observation["error"] = json!(error);
            }
        }
        observations.push(observation);
    }
    if item == "ui" {
        let _ = adb::output(path, Some(&serial), &["shell", "rm", "-f", &remote], 5).await;
    }
    if let Some(error) = interrupted {
        return Err(error);
    }
    let successes = observations
        .iter()
        .filter(|o| {
            string(o, "status") == "complete"
                && (!["logs", "crash"].contains(&item)
                    || o["argv"]
                        .as_array()
                        .is_some_and(|argv| argv.iter().any(|value| value == "logcat")))
        })
        .count();
    // The dump command's console message is not the UI tree.
    let useful = successes > 0
        && (item != "ui"
            || observations
                .get(1)
                .is_some_and(|o| string(o, "status") == "complete"));
    receipt["status"] = json!(if useful {
        if missed {
            "partial"
        } else {
            "complete"
        }
    } else {
        "unavailable"
    });
    receipt["commands"] = json!(observations);
    receipt["finishedAt"] = json!(now());
    if !useful {
        receipt["error"] = json!(if plans.is_empty() {
            "未填写目标包名，未运行应用专项诊断"
        } else {
            "未获取到有效诊断内容，见逐项命令结果"
        });
    }
    let attachment = if item == "environment" && !files.is_empty() {
        let mut document = format!(
            "Android 设备环境快照\n目标设备: {serial}\n项目配置包名: {package}\n开始时间: {}\n结束时间: {}\n说明: 当前设备状态，仅供事后诊断；不代表问题发生时状态。\n",
            string(&receipt, "startedAt"),
            string(&receipt, "finishedAt")
        );
        for (index, observation) in observations.iter().enumerate() {
            let argv = observation["argv"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ");
            document.push_str(&format!(
                "\n============================================================\n命令: adb {argv}\n状态: {}\n开始: {}\n结束: {}\n",
                string(observation, "status"),
                string(observation, "startedAt"),
                string(observation, "finishedAt")
            ));
            if let Some(error) = observation["error"].as_str() {
                document.push_str(&format!("错误: {error}\n"));
            }
            document.push_str("------------------------------------------------------------\n");
            let prefix = format!("{index:02}-");
            if let Some((_, output)) = files.iter().find(|(name, _)| name.starts_with(&prefix)) {
                document.push_str(&String::from_utf8_lossy(output));
                if !output.ends_with(b"\n") {
                    document.push('\n');
                }
            } else {
                document.push_str("<未返回可保存的输出>\n");
            }
        }
        let path = state
            .store
            .root
            .join(format!("device-environment-snapshot-{}.txt", uuid()));
        std::fs::write(&path, document).map_err(|e| e.to_string())?;
        let inspected = evidence::inspect(
            &path,
            "sample",
            &format!(
                "设备环境快照；采集项：{}；项目配置包名 {package}；采集时间见文档",
                item_name(item)
            ),
            &state.store.root,
        );
        let _ = std::fs::remove_file(&path);
        Some(inspected?)
    } else if !files.is_empty() {
        let mut attachment = zip_report(
            &state.store.root,
            &format!("post-capture-{item}.zip"),
            &files,
            &receipt,
        )?;
        attachment.source = format!(
            "事后补采诊断；采集项：{}；时间及范围见 receipt.json",
            item_name(item)
        );
        Some(attachment)
    } else {
        None
    };
    Ok((receipt, attachment))
}
fn persist_item(
    state: &AppState,
    job_id: &str,
    item: &str,
    mut receipt: Value,
    attachment: Option<Attachment>,
) -> Result<()> {
    let _write = state.store.writes.lock().unwrap();
    let mut job: Value = state
        .store
        .get("capture", job_id)?
        .ok_or("补采任务不存在")?;
    binding(state, &job)?;
    let mut draft = state.store.draft(&string(&job, "draftId"))?;
    let mut replaced_files = Vec::new();
    if let Some(attachment) = attachment {
        let replaced = previous_item_attachments(state, &job, item)?;
        receipt["attachments"] = json!([attachment.id]);
        let attachments = draft["attachments"]
            .as_array_mut()
            .ok_or("草稿附件格式错误")?;
        replaced_files.extend(
            attachments
                .iter()
                .filter(|row| replaced.contains(&string(row, "id")))
                .filter_map(|row| serde_json::from_value::<Attachment>(row.clone()).ok()),
        );
        attachments.retain(|row| !replaced.contains(&string(row, "id")));
        attachments.push(serde_json::to_value(attachment).map_err(|e| e.to_string())?);
        draft["previewConfirmed"] = json!(false);
        draft["updatedAt"] = json!(now());
        state.assert_no_token(&draft)?;
    }
    job["items"][item] = receipt;
    state.store.put_many(&[
        ("capture", job_id, &job),
        ("draft", &string(&job, "draftId"), &draft),
    ])?;
    for old in replaced_files {
        let _ = evidence::remove_managed_attachment(&state.store.root, &old);
    }
    Ok(())
}
fn previous_item_attachments(
    state: &AppState,
    current: &Value,
    item: &str,
) -> Result<HashSet<String>> {
    let mut ids = HashSet::new();
    for job in state.store.list("capture")? {
        if string(&job, "id") == string(current, "id")
            || string(&job, "kind") != "post-capture"
            || string(&job, "draftId") != string(current, "draftId")
            || string(&job, "scope") != string(current, "scope")
        {
            continue;
        }
        for id in job["items"][item]["attachments"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            ids.insert(id.to_string());
        }
    }
    Ok(ids)
}
fn previous_summary_attachments(
    state: &AppState,
    current: &Value,
    environment_only: bool,
) -> Result<HashSet<String>> {
    let mut ids = HashSet::new();
    for job in state.store.list("capture")? {
        let previous_environment_only = job["items"]
            .as_object()
            .is_some_and(|items| items.len() == 1 && items.contains_key("environment"));
        if string(&job, "id") != string(current, "id")
            && string(&job, "kind") == "post-capture"
            && string(&job, "draftId") == string(current, "draftId")
            && string(&job, "scope") == string(current, "scope")
            && previous_environment_only == environment_only
        {
            let id = string(&job, "summaryAttachmentId");
            if !id.is_empty() {
                ids.insert(id);
            }
        }
    }
    Ok(ids)
}
pub(crate) async fn execute(state: &AppState, job_id: &str) -> Result<()> {
    let path = state.adb_path()?;
    for item in ITEMS {
        let mut job: Value = state
            .store
            .get("capture", job_id)?
            .ok_or("补采任务不存在")?;
        if job["items"].get(*item).is_none() {
            continue;
        }
        binding(state, &job)?;
        connected(&path, &string(&job, "serial")).await?;
        {
            let _write = state.store.writes.lock().unwrap();
            binding(state, &job)?;
            job["items"][item]["status"] = json!("running");
            job["items"][item]["startedAt"] = json!(now());
            state.store.put("capture", job_id, &job)?;
        }
        match collect(state, &job, item, &path).await {
            Ok((receipt, attachment)) => persist_item(state, job_id, item, receipt, attachment)?,
            Err(error) => persist_item(
                state,
                job_id,
                item,
                json!({"status":"failed","error":error,"startedAt":job["items"][item]["startedAt"],"finishedAt":now(),"commands":[],"attachments":[]}),
                None,
            )?,
        }
    }
    let _write = state.store.writes.lock().unwrap();
    let mut job: Value = state
        .store
        .get("capture", job_id)?
        .ok_or("补采任务不存在")?;
    binding(state, &job)?;
    job["finishedAt"] = json!(now());
    let items = job["items"].as_object().ok_or("任务格式错误")?;
    let environment_only = items.len() == 1 && items.contains_key("environment");
    let collected_items = ITEMS
        .iter()
        .copied()
        .filter(|item| items.contains_key(*item))
        .map(item_name)
        .collect::<Vec<_>>()
        .join("、");
    job["status"] = json!(if items
        .values()
        .all(|item| string(item, "status") == "complete")
    {
        "complete"
    } else if items
        .values()
        .any(|item| ["complete", "partial"].contains(&string(item, "status").as_str()))
    {
        "partial"
    } else {
        "failed"
    });
    let mut draft = state.store.draft(&string(&job, "draftId"))?;
    let replaced_summaries = previous_summary_attachments(state, &job, environment_only)?;
    let attachments = draft["attachments"]
        .as_array_mut()
        .ok_or("草稿附件格式错误")?;
    let replaced_summary_files = attachments
        .iter()
        .filter(|row| replaced_summaries.contains(&string(row, "id")))
        .filter_map(|row| serde_json::from_value::<Attachment>(row.clone()).ok())
        .collect::<Vec<_>>();
    attachments.retain(|row| !replaced_summaries.contains(&string(row, "id")));
    if !environment_only {
        let mut summary = zip_report(&state.store.root,"post-capture-summary.zip",&[("README.txt".into(),"事后补采证据。当前状态是补采时状态；日志和历史记录不保证问题时刻的完整连续覆盖。目标应用取自草稿，未根据当前前台应用替换。详见 receipt.json。".as_bytes().to_vec())],&job)?;
        summary.source =
            format!("事后补采汇总；包含采集项：{collected_items}；时间及范围见 receipt.json");
        attachments.push(serde_json::to_value(&summary).map_err(|e| e.to_string())?);
        job["summaryAttachmentId"] = json!(summary.id);
    }
    draft["previewConfirmed"] = json!(false);
    draft["updatedAt"] = json!(now());
    state.assert_no_token(&draft)?;
    state.store.put_many(&[
        ("capture", job_id, &job),
        ("draft", &string(&job, "draftId"), &draft),
    ])?;
    for old in replaced_summary_files {
        let _ = evidence::remove_managed_attachment(&state.store.root, &old);
    }
    Ok(())
}
