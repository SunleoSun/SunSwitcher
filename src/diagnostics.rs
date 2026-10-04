use std::fmt;

#[cfg(any(test, all(target_os = "windows", not(debug_assertions))))]
use std::fs::{self, OpenOptions};
#[cfg(any(test, all(target_os = "windows", not(debug_assertions))))]
use std::io::{self, Write};
#[cfg(any(test, all(target_os = "windows", not(debug_assertions))))]
use std::path::Path;
#[cfg(all(target_os = "windows", not(debug_assertions)))]
use std::path::PathBuf;
#[cfg(all(target_os = "windows", not(debug_assertions)))]
use std::sync::{
    OnceLock,
    mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
};
#[cfg(all(target_os = "windows", not(debug_assertions)))]
use std::thread;
#[cfg(all(target_os = "windows", not(debug_assertions)))]
use std::time::{Duration, Instant};

#[cfg(all(target_os = "windows", not(debug_assertions)))]
const LOG_MAX_BYTES: usize = 1_000_000;
#[cfg(all(target_os = "windows", not(debug_assertions)))]
const LOG_FLUSH_INTERVAL: Duration = Duration::from_secs(1);
#[cfg(all(target_os = "windows", not(debug_assertions)))]
const LOG_QUEUE_CAPACITY: usize = 1024;
#[cfg(all(target_os = "windows", not(debug_assertions)))]
static LOG_SENDER: OnceLock<SyncSender<String>> = OnceLock::new();

pub fn log(args: fmt::Arguments<'_>) {
    #[cfg(all(target_os = "windows", not(debug_assertions)))]
    production_log(args);

    #[cfg(not(all(target_os = "windows", not(debug_assertions))))]
    eprintln!("{args}");
}

#[macro_export]
macro_rules! runtime_log {
    ($($arg:tt)*) => {
        $crate::diagnostics::log(format_args!($($arg)*))
    };
}

#[cfg(all(target_os = "windows", not(debug_assertions)))]
fn production_log(args: fmt::Arguments<'_>) {
    let sender = LOG_SENDER.get_or_init(start_log_worker);
    let _ = sender.try_send(format!("{args}\n"));
}

#[cfg(all(target_os = "windows", not(debug_assertions)))]
fn start_log_worker() -> SyncSender<String> {
    let (sender, receiver) = mpsc::sync_channel(LOG_QUEUE_CAPACITY);
    if let Ok(executable_path) = std::env::current_exe() {
        let log_path = executable_path.with_file_name("sunswitcher.log");
        let _ = thread::Builder::new()
            .name("sunswitcher-log".to_owned())
            .spawn(move || run_log_worker(log_path, receiver));
    }
    sender
}

#[cfg(all(target_os = "windows", not(debug_assertions)))]
fn run_log_worker(log_path: PathBuf, receiver: Receiver<String>) {
    let mut pending = String::new();
    let mut next_write_at = Instant::now();

    loop {
        if pending.is_empty() {
            match receiver.recv() {
                Ok(line) => push_pending_log(&mut pending, line),
                Err(_) => return,
            }
        }

        let now = Instant::now();
        if now >= next_write_at {
            let _ = append_bounded_log(&log_path, &pending, LOG_MAX_BYTES);
            pending.clear();
            next_write_at = Instant::now() + LOG_FLUSH_INTERVAL;
            continue;
        }

        match receiver.recv_timeout(next_write_at.saturating_duration_since(now)) {
            Ok(line) => push_pending_log(&mut pending, line),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

#[cfg(all(target_os = "windows", not(debug_assertions)))]
fn push_pending_log(pending: &mut String, line: String) {
    pending.push_str(&line);
    let max_pending_bytes = LOG_MAX_BYTES / 2;
    if pending.len() > max_pending_bytes {
        let retained = tail_log_text(pending, max_pending_bytes).to_owned();
        *pending = retained;
    }
}

#[cfg(any(test, all(target_os = "windows", not(debug_assertions))))]
fn tail_log_text(text: &str, max_bytes: usize) -> &str {
    if max_bytes == 0 {
        return "";
    }
    if text.len() <= max_bytes {
        return text;
    }

    let mut start = text.len() - max_bytes;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    if let Some(newline_offset) = text[start..].find('\n') {
        start += newline_offset + 1;
    }
    &text[start..]
}

#[cfg(any(test, all(target_os = "windows", not(debug_assertions))))]
fn append_bounded_log(path: &Path, batch: &str, max_bytes: usize) -> io::Result<()> {
    if max_bytes == 0 {
        return Ok(());
    }

    let batch = tail_log_text(batch, max_bytes);
    if batch.is_empty() {
        return Ok(());
    }

    let current_len = fs::metadata(path)
        .map(|metadata| metadata.len() as usize)
        .unwrap_or(0);
    if current_len.saturating_add(batch.len()) <= max_bytes {
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        return file.write_all(batch.as_bytes());
    }

    let keep_budget = max_bytes.saturating_sub(batch.len());
    let retained = if keep_budget == 0 {
        String::new()
    } else {
        fs::read_to_string(path)
            .map(|existing| tail_log_text(&existing, keep_budget).to_owned())
            .unwrap_or_default()
    };
    let mut payload = String::with_capacity(retained.len() + batch.len());
    payload.push_str(&retained);
    payload.push_str(batch);

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    file.write_all(payload.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn bounded_log_keeps_latest_data_within_limit() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "sunswitcher-log-test-{}-{unique}.log",
            std::process::id()
        ));
        let max_bytes = 128;

        append_bounded_log(&path, &format!("{}\n", "old".repeat(30)), max_bytes).unwrap();
        append_bounded_log(&path, &format!("{}\nlatest\n", "new".repeat(30)), max_bytes).unwrap();

        let bytes = fs::read(&path).unwrap();
        assert!(bytes.len() <= max_bytes);
        assert!(String::from_utf8(bytes).unwrap().contains("latest"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn tail_log_text_preserves_utf8_boundary() {
        let text = "old\nабвгд\nlatest\n";
        let tail = tail_log_text(text, 13);
        assert!(tail.len() <= 13);
        assert!(tail.is_char_boundary(0));
        assert!(tail.ends_with("latest\n"));
    }
}
