use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc, LazyLock, Mutex, OnceLock,
    },
    time::{Duration, Instant, SystemTime},
};

use chrono::Utc;
use regex::Regex;
use serde_json::{json, Map, Value};
use tauri::AppHandle;

const BASE_LOG_BYTES: u64 = 2 * 1024 * 1024;
const DEBUG_LOG_BYTES: u64 = 4 * 1024 * 1024;
const DEBUG_LOG_ROTATIONS: usize = 4;
const LOG_RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const QUEUE_CAPACITY: usize = 2_048;
const EVENT_STRING_LIMIT: usize = 2_000;

static SENSITIVE_PAIR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(authorization|access[_-]?token|refresh[_-]?token|password|cookie|secret|pat)\s*[:=]\s*[^\s,;]+")
        .expect("fixed diagnostic redaction regex")
});
static URL_CREDENTIALS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(https?://)[^/@\s]+@\S+").expect("fixed URL credential regex")
});
static UNIX_USER_HOME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(/Users|/home)/[^/\s]+").expect("fixed Unix user-directory redaction regex")
});
static WINDOWS_USER_HOME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b[A-Z]:\\Users\\[^\\\s]+")
        .expect("fixed Windows user-directory redaction regex")
});

struct LogLine {
    detailed: bool,
    line: String,
}

enum WriterMessage {
    Line(LogLine),
    Flush(mpsc::Sender<()>),
}

struct Logger {
    directory: PathBuf,
    sender: SyncSender<WriterMessage>,
    debug_enabled: AtomicBool,
    dropped: AtomicU64,
    started: Instant,
    run_id: String,
    active: Mutex<HashMap<String, Value>>,
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

pub fn init(root: &Path) -> std::io::Result<PathBuf> {
    let directory = root.join("diagnostics");
    fs::create_dir_all(&directory)?;
    prune_old_logs(&directory);
    let path = directory.join("runtime.log");
    let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
    let worker_directory = directory.clone();
    std::thread::Builder::new()
        .name("diagnostic-writer".into())
        .spawn(move || writer_loop(&worker_directory, receiver))?;
    let logger = Logger {
        directory,
        sender,
        debug_enabled: AtomicBool::new(false),
        dropped: AtomicU64::new(0),
        started: Instant::now(),
        run_id: uuid::Uuid::new_v4().to_string(),
        active: Mutex::new(HashMap::new()),
    };
    let _ = LOGGER.set(logger);
    event(
        "app",
        &format!("startup version={}", env!("CARGO_PKG_VERSION")),
    );
    Ok(path)
}

pub fn set_debug_enabled(enabled: bool) {
    if let Some(logger) = LOGGER.get() {
        logger.debug_enabled.store(enabled, Ordering::SeqCst);
        record(
            false,
            "diagnostics",
            "debug_setting_changed",
            json!({"enabled":enabled}),
        );
    }
}

pub fn debug_enabled() -> bool {
    LOGGER
        .get()
        .is_some_and(|logger| logger.debug_enabled.load(Ordering::SeqCst))
}

pub fn event(area: &str, message: &str) {
    record(
        false,
        area,
        "message",
        json!({"message":single_line(message)}),
    );
}

pub fn base_event(area: &str, event_name: &str, fields: Value) {
    record(false, area, event_name, fields);
}

pub fn debug_event(area: &str, event_name: &str, fields: Value) {
    if debug_enabled() {
        record(true, area, event_name, fields);
    }
}

fn record(detailed: bool, area: &str, event_name: &str, fields: Value) {
    let Some(logger) = LOGGER.get() else {
        return;
    };
    if detailed && !logger.debug_enabled.load(Ordering::Relaxed) {
        return;
    }
    let dropped = logger.dropped.swap(0, Ordering::Relaxed);
    let line = json!({
        "timestamp": Utc::now().to_rfc3339(),
        "monotonicMs": logger.started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        "runId": logger.run_id,
        "level": if detailed { "debug" } else { "info" },
        "area": safe_identifier(area),
        "event": safe_identifier(event_name),
        "droppedSinceLast": dropped,
        "fields": sanitize_value(fields),
    });
    let message = WriterMessage::Line(LogLine {
        detailed,
        line: line.to_string(),
    });
    match logger.sender.try_send(message) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
            logger.dropped.fetch_add(dropped + 1, Ordering::Relaxed);
        }
    }
}

pub fn flush(timeout: Duration) -> bool {
    let Some(logger) = LOGGER.get() else {
        return false;
    };
    let (sender, receiver) = mpsc::channel();
    if logger
        .sender
        .try_send(WriterMessage::Flush(sender))
        .is_err()
    {
        return false;
    }
    receiver.recv_timeout(timeout).is_ok()
}

pub fn directory() -> Option<PathBuf> {
    LOGGER.get().map(|logger| logger.directory.clone())
}

pub fn run_id() -> String {
    LOGGER
        .get()
        .map(|logger| logger.run_id.clone())
        .unwrap_or_default()
}

pub struct OperationGuard {
    id: String,
    area: String,
    name: String,
    started: Instant,
    finished: bool,
}

impl OperationGuard {
    pub fn begin(area: &str, name: &str, timeout_seconds: u64, fields: Value) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        if let Some(logger) = LOGGER.get() {
            let mut value = json!({
                "id": id,
                "area": safe_identifier(area),
                "operation": safe_identifier(name),
                "startedAt": Utc::now().to_rfc3339(),
                "timeoutSeconds": timeout_seconds,
                "fields": sanitize_value(fields),
            });
            value["elapsedMs"] = json!(0);
            if let Ok(mut active) = logger.active.lock() {
                active.insert(id.clone(), value);
            }
        }
        base_event(
            area,
            "operation_started",
            json!({"operationId":id,"operation":name,"timeoutSeconds":timeout_seconds}),
        );
        Self {
            id,
            area: area.to_owned(),
            name: name.to_owned(),
            started: Instant::now(),
            finished: false,
        }
    }

    pub fn progress(&self, fields: Value) {
        if let Some(logger) = LOGGER.get() {
            if let Ok(mut active) = logger.active.lock() {
                if let Some(value) = active.get_mut(&self.id) {
                    value["elapsedMs"] = json!(elapsed_ms(self.started));
                    value["progress"] = sanitize_value(fields.clone());
                }
            }
        }
        debug_event(
            &self.area,
            "operation_progress",
            json!({"operationId":self.id,"operation":self.name,"progress":fields}),
        );
    }

    pub fn finish(mut self, outcome: &str, fields: Value) {
        self.finished = true;
        if let Some(logger) = LOGGER.get() {
            if let Ok(mut active) = logger.active.lock() {
                active.remove(&self.id);
            }
        }
        base_event(
            &self.area,
            "operation_finished",
            json!({
                "operationId":self.id,
                "operation":self.name,
                "outcome":safe_identifier(outcome),
                "durationMs":elapsed_ms(self.started),
                "result":fields,
            }),
        );
    }
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        if let Some(logger) = LOGGER.get() {
            if let Ok(mut active) = logger.active.lock() {
                active.remove(&self.id);
            }
        }
        base_event(
            &self.area,
            "operation_abandoned",
            json!({
                "operationId":self.id,
                "operation":self.name,
                "durationMs":elapsed_ms(self.started),
            }),
        );
    }
}

pub fn active_operations() -> Vec<Value> {
    let Some(logger) = LOGGER.get() else {
        return Vec::new();
    };
    logger
        .active
        .lock()
        .map(|active| active.values().cloned().collect())
        .unwrap_or_default()
}

pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(location) = info.location() {
            base_event(
                "panic",
                "captured",
                json!({"file":location.file(),"line":location.line()}),
            );
        } else {
            event("panic", "captured without source location");
        }
        let _ = flush(Duration::from_millis(200));
        previous(info);
    }));
}

pub fn start_main_thread_watchdog(app: AppHandle) {
    let acknowledged = Arc::new(AtomicU64::new(0));
    std::thread::spawn(move || {
        let mut sequence = 0_u64;
        loop {
            sequence = sequence.wrapping_add(1);
            let current = sequence;
            let queued = Instant::now();
            let main_acknowledged = acknowledged.clone();
            if app
                .run_on_main_thread(move || {
                    main_acknowledged.store(current, Ordering::SeqCst);
                })
                .is_err()
            {
                event("watchdog", "main thread probe could not be queued");
                return;
            }
            std::thread::sleep(Duration::from_secs(10));
            if acknowledged.load(Ordering::SeqCst) != current {
                base_event(
                    "watchdog",
                    "main_thread_unresponsive",
                    json!({"waitedMs":elapsed_ms(queued)}),
                );
            }
            std::thread::sleep(Duration::from_secs(20));
        }
    });
}

fn writer_loop(directory: &Path, receiver: Receiver<WriterMessage>) {
    while let Ok(message) = receiver.recv() {
        match message {
            WriterMessage::Line(line) => {
                let (name, limit, rotations) = if line.detailed {
                    ("debug.jsonl", DEBUG_LOG_BYTES, DEBUG_LOG_ROTATIONS)
                } else {
                    ("runtime.log", BASE_LOG_BYTES, 1)
                };
                write_log_line(directory, name, limit, rotations, &line.line);
            }
            WriterMessage::Flush(reply) => {
                let _ = reply.send(());
            }
        }
    }
}

fn write_log_line(directory: &Path, name: &str, limit: u64, rotations: usize, line: &str) {
    let fallback;
    let line = if line.len() as u64 + 1 > limit {
        fallback = json!({
            "timestamp":Utc::now().to_rfc3339(),
            "area":"diagnostics",
            "event":"oversize_event_dropped",
            "originalBytes":line.len(),
        })
        .to_string();
        fallback.as_str()
    } else {
        line
    };
    let path = directory.join(name);
    if path
        .metadata()
        .is_ok_and(|metadata| metadata.len() + line.len() as u64 + 1 > limit)
    {
        rotate(&path, rotations);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
    }
}

fn rotate(path: &Path, rotations: usize) {
    if rotations == 0 {
        let _ = fs::remove_file(path);
        return;
    }
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("log");
    let oldest = path.with_file_name(format!("{file_name}.{rotations}"));
    let _ = fs::remove_file(oldest);
    for index in (1..rotations).rev() {
        let from = path.with_file_name(format!("{file_name}.{index}"));
        let to = path.with_file_name(format!("{file_name}.{}", index + 1));
        let _ = fs::rename(from, to);
    }
    let _ = fs::rename(path, path.with_file_name(format!("{file_name}.1")));
}

fn prune_old_logs(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !(name.starts_with("runtime.log") || name.starts_with("debug.jsonl")) {
            continue;
        }
        let expired = path
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > LOG_RETENTION);
        if expired {
            let _ = fs::remove_file(path);
        }
    }
}

fn sanitize_value(value: Value) -> Value {
    match value {
        Value::String(value) => Value::String(redact_text(&value)),
        Value::Array(values) => Value::Array(values.into_iter().map(sanitize_value).collect()),
        Value::Object(values) => {
            let mut clean = Map::new();
            for (key, value) in values {
                let normalized = safe_identifier(&key);
                if [
                    "authorization",
                    "token",
                    "accesstoken",
                    "refreshtoken",
                    "password",
                    "cookie",
                    "secret",
                    "pat",
                ]
                .contains(&normalized.to_ascii_lowercase().as_str())
                {
                    clean.insert(normalized, Value::String("<REDACTED>".into()));
                } else {
                    clean.insert(normalized, sanitize_value(value));
                }
            }
            Value::Object(clean)
        }
        value => value,
    }
}

pub fn redact_text(value: &str) -> String {
    redact_payload_text(&single_line(value))
        .chars()
        .take(EVENT_STRING_LIMIT)
        .collect()
}

pub fn redact_payload_text(value: &str) -> String {
    let mut value = value.to_owned();
    for home in [
        std::env::var("HOME").ok(),
        std::env::var("USERPROFILE").ok(),
    ]
    .into_iter()
    .flatten()
    .filter(|value| value.len() > 2)
    {
        value = value.replace(&home, "<HOME>");
    }
    value = SENSITIVE_PAIR
        .replace_all(&value, "$1=<REDACTED>")
        .into_owned();
    value = URL_CREDENTIALS
        .replace_all(&value, "$1<REDACTED>@<HOST>")
        .into_owned();
    value = UNIX_USER_HOME.replace_all(&value, "$1/<USER>").into_owned();
    WINDOWS_USER_HOME
        .replace_all(&value, "<USER_HOME>")
        .into_owned()
}

fn single_line(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            '\r' | '\n' | '\t' => ' ',
            _ => character,
        })
        .take(EVENT_STRING_LIMIT)
        .collect()
}

fn safe_identifier(value: &str) -> String {
    value
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || ['_', '-', '.'].contains(character)
        })
        .take(80)
        .collect()
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_removes_home_credentials_and_sensitive_pairs() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp/example-home".into());
        let text = redact_text(&format!(
            "path={home}/file authorization=Bearer-abc https://name:pass@example.test/a"
        ));
        assert!(!text.contains(&home));
        assert!(!text.contains("Bearer-abc"));
        assert!(!text.contains("name:pass"));
    }

    #[test]
    fn rotation_has_fixed_file_count() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("debug.jsonl");
        fs::write(&path, "current").unwrap();
        for index in 1..=DEBUG_LOG_ROTATIONS {
            fs::write(path.with_file_name(format!("debug.jsonl.{index}")), "old").unwrap();
        }
        rotate(&path, DEBUG_LOG_ROTATIONS);
        assert!(!path.exists());
        assert_eq!(
            fs::read_dir(temp.path()).unwrap().flatten().count(),
            DEBUG_LOG_ROTATIONS
        );
    }

    #[test]
    fn repeated_writes_keep_a_fixed_total_disk_bound() {
        let temp = tempfile::tempdir().unwrap();
        for index in 0..100 {
            write_log_line(
                temp.path(),
                "bounded.jsonl",
                128,
                2,
                &format!("{index:03}-{}", "x".repeat(24)),
            );
        }
        let files: Vec<_> = fs::read_dir(temp.path()).unwrap().flatten().collect();
        assert_eq!(files.len(), 3);
        let total: u64 = files
            .iter()
            .map(|entry| entry.metadata().unwrap().len())
            .sum();
        assert!(files
            .iter()
            .all(|entry| entry.metadata().unwrap().len() <= 128));
        assert!(total <= 3 * 128);
        write_log_line(temp.path(), "bounded.jsonl", 128, 2, &"y".repeat(1_024));
        assert!(temp.path().join("bounded.jsonl").metadata().unwrap().len() <= 128);
    }
}
