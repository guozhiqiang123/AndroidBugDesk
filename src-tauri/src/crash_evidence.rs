//! Privacy-preserving parsers for whole-device crash candidates.
//!
//! ADB cannot consistently filter historical Native crashes by UID or PID,
//! especially on Android 10. These helpers only return complete records that
//! can be attributed to the configured package; callers must discard the raw
//! device output after invoking them.

#[derive(Debug, Default, PartialEq, Eq)]
pub struct FilteredEvidence {
    pub bytes: Vec<u8>,
    pub matched: usize,
    pub discarded: usize,
    pub process_ids: Vec<u32>,
}

fn package_markers(package: &str) -> [String; 5] {
    [
        format!(">>> {package} <<<"),
        format!("Cmdline: {package}"),
        format!("Process: {package},"),
        format!("Process: {package}:"),
        format!("processName={package}"),
    ]
}

fn contains_numeric_token(text: &str, value: u32) -> bool {
    let needle = value.to_string();
    text.match_indices(&needle).any(|(index, _)| {
        let before = text[..index].chars().next_back();
        let after = text[index + needle.len()..].chars().next();
        before.is_none_or(|value| !value.is_ascii_digit())
            && after.is_none_or(|value| !value.is_ascii_digit())
    })
}

fn belongs_to_target(text: &str, package: &str, uid: Option<u32>) -> bool {
    package_markers(package)
        .iter()
        .any(|marker| text.contains(marker))
        || text.lines().any(|line| {
            let line = line.trim();
            line == package
                || line.contains(&format!(" {package} "))
                || line.contains(&format!(" {package}:"))
                || line.ends_with(&format!(" {package}"))
                || line.ends_with(&format!(" {package}:"))
        })
        || uid.is_some_and(|uid| {
            ["uid=", "uid: ", "UID ", "u0_a"]
                .iter()
                .any(|prefix| text.contains(&format!("{prefix}{uid}")))
        })
}

fn extract_process_ids(text: &str) -> Vec<u32> {
    let mut ids = Vec::new();
    for marker in ["pid: ", "PID: ", "pid="] {
        let mut remainder = text;
        while let Some(index) = remainder.find(marker) {
            remainder = &remainder[index + marker.len()..];
            let digits = remainder
                .chars()
                .take_while(|value| value.is_ascii_digit())
                .collect::<String>();
            if let Ok(pid) = digits.parse::<u32>() {
                if pid > 0 {
                    ids.push(pid);
                }
            }
        }
    }
    ids.sort_unstable();
    ids.dedup();
    ids.truncate(64);
    ids
}

fn crash_record_start(line: &str) -> bool {
    line.contains("*** *** *** ***")
        || line.contains("FATAL EXCEPTION:")
        || line.contains("ANR in ")
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

fn allowed_system_event(line: &str) -> bool {
    [
        "am_crash",
        "am_anr",
        "am_proc_died",
        "am_kill",
        "am_low_memory",
        "ActivityManager",
        "lowmemorykiller",
        "lmkd",
    ]
    .iter()
    .any(|tag| line.contains(tag))
}

fn keep_log_line(line: &str, package: &str, uid: Option<u32>, process_ids: &[u32]) -> Option<bool> {
    log_line_pid(line).map(|pid| {
        process_ids.contains(&pid)
            || (allowed_system_event(line)
                && (line.contains(package)
                    || uid.is_some_and(|uid| contains_numeric_token(line, uid))
                    || process_ids
                        .iter()
                        .any(|pid| contains_numeric_token(line, *pid))))
    })
}

fn join_records(records: Vec<&str>, package: &str, uid: Option<u32>) -> FilteredEvidence {
    let mut result = FilteredEvidence::default();
    for record in records {
        if belongs_to_target(record, package, uid) {
            if !result.bytes.is_empty() && !result.bytes.ends_with(b"\n") {
                result.bytes.push(b'\n');
            }
            result.bytes.extend_from_slice(record.as_bytes());
            if !result.bytes.ends_with(b"\n") {
                result.bytes.push(b'\n');
            }
            result.matched += 1;
            result.process_ids.extend(extract_process_ids(record));
        } else {
            result.discarded += 1;
        }
    }
    result.process_ids.sort_unstable();
    result.process_ids.dedup();
    result.process_ids.truncate(64);
    result
}

pub fn filter_crash_records(output: &[u8], package: &str, uid: Option<u32>) -> FilteredEvidence {
    let text = String::from_utf8_lossy(output);
    let mut starts = text
        .match_indices('\n')
        .filter_map(|(index, _)| {
            let start = index + 1;
            crash_record_start(text[start..].lines().next().unwrap_or_default()).then_some(start)
        })
        .collect::<Vec<_>>();
    if text.lines().next().is_some_and(crash_record_start) {
        starts.insert(0, 0);
    }
    starts.sort_unstable();
    starts.dedup();
    let records = starts
        .iter()
        .enumerate()
        .map(|(index, start)| {
            let end = starts.get(index + 1).copied().unwrap_or(text.len());
            &text[*start..end]
        })
        .collect::<Vec<_>>();
    join_records(records, package, uid)
}

pub fn filter_dropbox_entries(output: &[u8], package: &str, uid: Option<u32>) -> FilteredEvidence {
    let text = String::from_utf8_lossy(output);
    let separator = "========================================";
    let records = if text.contains(separator) {
        text.split(separator)
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .collect::<Vec<_>>()
    } else if text.trim().is_empty() {
        Vec::new()
    } else {
        vec![text.trim()]
    };
    join_records(records, package, uid)
}

pub fn filter_system_events(
    output: &[u8],
    package: &str,
    uid: Option<u32>,
    process_ids: &[u32],
) -> FilteredEvidence {
    let text = String::from_utf8_lossy(output);
    let mut result = FilteredEvidence::default();
    for line in text.lines() {
        let allowed = allowed_system_event(line);
        let identity = line.contains(package)
            || uid.is_some_and(|uid| contains_numeric_token(line, uid))
            || process_ids
                .iter()
                .any(|pid| contains_numeric_token(line, *pid));
        if allowed && identity {
            result.bytes.extend_from_slice(line.as_bytes());
            result.bytes.push(b'\n');
            result.matched += 1;
        } else if allowed {
            result.discarded += 1;
        }
    }
    result
}

pub fn filter_log_lines(
    output: &[u8],
    package: &str,
    uid: Option<u32>,
    process_ids: &[u32],
) -> Vec<u8> {
    let text = String::from_utf8_lossy(output);
    let mut result = Vec::new();
    let mut keep = false;
    for line in text.lines() {
        if let Some(decision) = keep_log_line(line, package, uid, process_ids) {
            keep = decision;
        }
        if keep {
            result.extend_from_slice(line.as_bytes());
            result.push(b'\n');
        }
    }
    result
}

pub struct LiveLineFilter {
    pending: Vec<u8>,
    keep: bool,
}

impl Default for LiveLineFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveLineFilter {
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
            keep: false,
        }
    }

    pub fn push(
        &mut self,
        bytes: &[u8],
        package: &str,
        uid: Option<u32>,
        process_ids: &[u32],
    ) -> Vec<u8> {
        self.pending.extend_from_slice(bytes);
        let mut result = Vec::new();
        while let Some(end) = self.pending.iter().position(|value| *value == b'\n') {
            let line = self.pending.drain(..=end).collect::<Vec<_>>();
            let text = String::from_utf8_lossy(&line);
            if let Some(decision) = keep_log_line(&text, package, uid, process_ids) {
                self.keep = decision;
            }
            if self.keep {
                result.extend_from_slice(&line);
            }
        }
        if self.pending.len() > 256_000 {
            self.pending.clear();
            self.keep = false;
        }
        result
    }

    pub fn finish(&mut self) -> Vec<u8> {
        let line = std::mem::take(&mut self.pending);
        if self.keep {
            line
        } else {
            Vec::new()
        }
    }
}

pub struct LiveCrashFilter {
    package: String,
    uid: Option<u32>,
    pending: Vec<u8>,
    record: Vec<u8>,
    active: bool,
    emitting: bool,
}

impl LiveCrashFilter {
    pub fn new(package: &str, uid: Option<u32>) -> Self {
        Self {
            package: package.into(),
            uid,
            pending: Vec::new(),
            record: Vec::new(),
            active: false,
            emitting: false,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.pending.extend_from_slice(bytes);
        let mut result = Vec::new();
        while let Some(end) = self.pending.iter().position(|value| *value == b'\n') {
            let line = self.pending.drain(..=end).collect::<Vec<_>>();
            let text = String::from_utf8_lossy(&line);
            if crash_record_start(&text) {
                self.record.clear();
                self.active = true;
                self.emitting = false;
            }
            if !self.active {
                continue;
            }
            if self.emitting {
                result.extend_from_slice(&line);
                continue;
            }
            self.record.extend_from_slice(&line);
            let record = String::from_utf8_lossy(&self.record);
            if belongs_to_target(&record, &self.package, self.uid) {
                result.append(&mut self.record);
                self.emitting = true;
            }
            if self.record.len() > 2_000_000 {
                self.record.clear();
                self.active = false;
            }
        }
        if self.pending.len() > 256_000 {
            self.pending.clear();
        }
        result
    }

    pub fn finish(&mut self) -> Vec<u8> {
        let pending = std::mem::take(&mut self.pending);
        if self.emitting {
            pending
        } else {
            self.record.extend_from_slice(&pending);
            let record = String::from_utf8_lossy(&self.record);
            if belongs_to_target(&record, &self.package, self.uid) {
                std::mem::take(&mut self.record)
            } else {
                self.record.clear();
                Vec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_record_keeps_debuggerd_continuations_and_drops_other_apps() {
        let input = br#"10-16 19:22:00.000  111  111 F DEBUG   : *** *** *** *** *** *** *** ***
10-16 19:22:00.001  111  111 F DEBUG   : Cmdline: com.other.private
10-16 19:22:00.002  111  111 F DEBUG   : backtrace: private
10-16 19:23:00.000  999  999 F DEBUG   : *** *** *** *** *** *** *** ***
10-16 19:23:00.001  999  999 F DEBUG   : pid: 7777, tid: 7788, name: DefaultDispatcher  >>> com.ludashi.superboost <<<
10-16 19:23:00.002  999  999 F DEBUG   : signal 6 (SIGABRT)
10-16 19:23:00.003  999  999 F DEBUG   : Abort message: 'Only GC-running thread should access the mark stack'
10-16 19:23:00.004  222  222 F DEBUG   : #00 pc findAdMobAdPrices
"#;
        let result = filter_crash_records(input, "com.ludashi.superboost", Some(10123));
        let text = String::from_utf8(result.bytes).unwrap();
        assert_eq!(result.matched, 1);
        assert_eq!(result.discarded, 1);
        assert_eq!(result.process_ids, vec![7777]);
        assert!(text.contains("SIGABRT"));
        assert!(text.contains("findAdMobAdPrices"));
        assert!(!text.contains("com.other.private"));
        assert!(!text.contains("private\n"));
    }

    #[test]
    fn java_record_keeps_complete_cause_chain_after_process_restart() {
        let input = br#"10-16 19:20:00.000 7777 7788 E AndroidRuntime: FATAL EXCEPTION: main
10-16 19:20:00.001 7777 7788 E AndroidRuntime: Process: com.example.test, PID: 7777
10-16 19:20:00.002 7777 7788 E AndroidRuntime: java.lang.RuntimeException: top
10-16 19:20:00.003 7777 7788 E AndroidRuntime: Caused by: java.lang.IllegalStateException: cause
10-16 19:21:00.000 8888 8889 E AndroidRuntime: FATAL EXCEPTION: main
10-16 19:21:00.001 8888 8889 E AndroidRuntime: Process: com.other.private, PID: 8888
10-16 19:21:00.002 8888 8889 E AndroidRuntime: secret
"#;
        let result = filter_crash_records(input, "com.example.test", Some(10123));
        let text = String::from_utf8(result.bytes).unwrap();
        assert_eq!(result.process_ids, vec![7777]);
        assert!(text.contains("Caused by"));
        assert!(!text.contains("com.other.private"));
    }

    #[test]
    fn dropbox_and_system_events_only_keep_target_entries() {
        let dropbox = br#"========================================
2026-10-16 SYSTEM_TOMBSTONE
Cmdline: com.example.test
signal 6 (SIGABRT)
========================================
2026-10-16 SYSTEM_TOMBSTONE
Cmdline: com.other.private
secret
"#;
        let filtered = filter_dropbox_entries(dropbox, "com.example.test", Some(10123));
        assert_eq!(filtered.matched, 1);
        assert!(!String::from_utf8(filtered.bytes)
            .unwrap()
            .contains("com.other.private"));

        let events = br#"10-16 I am_proc_died: [0,10123,com.example.test]
10-16 I am_proc_died: [0,10124,com.other.private]
10-16 I WifiService: com.example.test is unrelated
"#;
        let filtered = filter_system_events(events, "com.example.test", Some(10123), &[]);
        let text = String::from_utf8(filtered.bytes).unwrap();
        assert!(text.contains("com.example.test"));
        assert!(!text.contains("com.other.private"));
        assert!(!text.contains("WifiService"));
    }

    #[test]
    fn live_filters_never_emit_unrelated_lines_or_crash_records() {
        let mut lines = LiveLineFilter::new();
        let first = lines.push(
            b"1789102800.000 999 999 I OTHER: private\n1789102801.000 321 322 E APP: target\n  continuation\n",
            "com.example.test",
            Some(10123),
            &[321],
        );
        let text = String::from_utf8(first).unwrap();
        assert!(text.contains("target"));
        assert!(text.contains("continuation"));
        assert!(!text.contains("private"));

        let mut crashes = LiveCrashFilter::new("com.example.test", Some(10123));
        let first = crashes.push(
            b"10-16 19:00:00.000 999 999 F DEBUG: *** *** *** ***\n10-16 19:00:00.001 999 999 F DEBUG: Cmdline: com.other.private\n10-16 19:01:00.000 888 888 F DEBUG: *** *** *** ***\n",
        );
        assert!(first.is_empty());
        let second = crashes.push(
            b"10-16 19:01:00.001 888 888 F DEBUG: Cmdline: com.example.test\n10-16 19:01:00.002 777 777 F DEBUG: backtrace target\n",
        );
        let text = String::from_utf8(second).unwrap();
        assert!(text.contains("*** ***"));
        assert!(text.contains("com.example.test"));
        assert!(text.contains("backtrace target"));
        assert!(!text.contains("com.other.private"));
    }
}
