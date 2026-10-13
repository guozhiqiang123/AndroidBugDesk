use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Duration,
};

use chrono::Utc;
use tauri::AppHandle;

const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;
static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
static LOG_WRITE: Mutex<()> = Mutex::new(());

pub fn init(root: &Path) -> std::io::Result<PathBuf> {
    let directory = root.join("diagnostics");
    fs::create_dir_all(&directory)?;
    let path = directory.join("runtime.log");
    let _ = LOG_PATH.set(path.clone());
    event(
        "app",
        &format!("startup version={}", env!("CARGO_PKG_VERSION")),
    );
    Ok(path)
}

pub fn event(area: &str, message: &str) {
    let Some(path) = LOG_PATH.get() else {
        return;
    };
    let Ok(_guard) = LOG_WRITE.lock() else {
        return;
    };
    if path
        .metadata()
        .is_ok_and(|metadata| metadata.len() >= MAX_LOG_BYTES)
    {
        let previous = path.with_extension("log.1");
        let _ = fs::remove_file(&previous);
        let _ = fs::rename(path, previous);
    }
    let area = single_line(area);
    let message = single_line(message);
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{} [{area}] {message}", Utc::now().to_rfc3339());
    }
}

pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(location) = info.location() {
            event(
                "panic",
                &format!("captured at {}:{}", location.file(), location.line()),
            );
        } else {
            event("panic", "captured without source location");
        }
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
                event("watchdog", "main thread did not answer within 10 seconds");
            }
            std::thread::sleep(Duration::from_secs(20));
        }
    });
}

fn single_line(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            '\r' | '\n' | '\t' => ' ',
            _ => character,
        })
        .take(500)
        .collect()
}
