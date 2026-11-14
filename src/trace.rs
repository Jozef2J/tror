use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Info,
    Client,
    Server,
    Error,
}

static LINES: Mutex<Vec<(String, Kind)>> = Mutex::new(Vec::new());
static POPUPS: Mutex<Vec<(String, bool)>> = Mutex::new(Vec::new());
static REPEAT: AtomicBool = AtomicBool::new(false);

struct Sink;

fn ftp_line(text: &str) -> Option<String> {
    if let Some(command) = text.strip_prefix("CC OUT: ") {
        let upper = command.to_ascii_uppercase();
        return Some(if upper.starts_with("PASS ") {
            "-> PASS ********".to_owned()
        } else {
            format!("-> {}", command.trim_end())
        });
    }
    let list = text.strip_prefix("CC IN: [")?.strip_suffix(']')?;
    let bytes: Vec<u8> = list
        .split(',')
        .filter_map(|n| n.trim().parse().ok())
        .collect();
    let reply = String::from_utf8_lossy(&bytes);
    let reply = reply.trim_end();
    (!reply.is_empty()).then(|| reply.to_owned())
}

impl log::Log for Sink {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.target() == "trorpad" || metadata.target().starts_with("suppaftp")
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let text = record.args().to_string();
        if record.target() == "trorpad" {
            line(text, record.level() <= log::Level::Warn);
        } else if text.starts_with("Code parsed from response") {
            REPEAT.store(true, Ordering::Relaxed);
        } else if text.starts_with("CC IN") && REPEAT.swap(false, Ordering::Relaxed) {
        } else if let Some(text) = ftp_line(&text) {
            let kind = if text.starts_with("-> ") {
                Kind::Client
            } else if text.starts_with('4') || text.starts_with('5') {
                Kind::Error
            } else {
                Kind::Server
            };
            push(text, kind);
        }
    }

    fn flush(&self) {}
}

pub fn install() {
    if log::set_logger(&Sink).is_ok() {
        log::set_max_level(log::LevelFilter::Trace);
    }
}

pub fn push(text: impl Into<String>, kind: Kind) {
    let text = text.into();
    if text.is_empty() {
        return;
    }
    let stamp = chrono::Local::now().format("%H:%M:%S");
    if let Ok(mut lines) = LINES.lock() {
        lines.push((format!("{stamp}  {text}"), kind));
    }
}

pub fn line(text: impl Into<String>, error: bool) {
    push(text, if error { Kind::Error } else { Kind::Info });
}

pub fn client(text: impl Into<String>) {
    push(text, Kind::Client);
}

pub fn server(text: impl Into<String>) {
    push(text, Kind::Server);
}

pub fn take() -> Vec<(String, Kind)> {
    LINES
        .lock()
        .map(|mut lines| std::mem::take(&mut *lines))
        .unwrap_or_default()
}

pub fn popup(text: impl Into<String>, ok: bool) {
    if let Ok(mut list) = POPUPS.lock() {
        list.push((text.into(), ok));
    }
}

pub fn take_popups() -> Vec<(String, bool)> {
    POPUPS
        .lock()
        .map(|mut list| std::mem::take(&mut *list))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ftp_trace_becomes_raw_lines() {
        assert_eq!(ftp_line("CC OUT: CWD /www").as_deref(), Some("-> CWD /www"));
        assert_eq!(
            ftp_line("CC OUT: PASS secret").as_deref(),
            Some("-> PASS ********")
        );
        assert_eq!(
            ftp_line("CC IN: [50, 50, 54, 32, 79, 75, 13, 10]").as_deref(),
            Some("226 OK")
        );
        assert_eq!(ftp_line("Code parsed from response: 226 (x)"), None);
    }
}
