use std::hash::{BuildHasher, Hasher};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

fn file() -> Option<PathBuf> {
    eframe::storage_dir("Tror").map(|dir| dir.join("instance"))
}

static OWN: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub fn release() {
    if let (Some(path), Some(own)) = (file(), OWN.get())
        && std::fs::read_to_string(&path).is_ok_and(|text| text == *own)
    {
        let _ = std::fs::remove_file(path);
    }
}

pub fn restarted() -> bool {
    std::env::args_os().any(|a| a == RESTART)
}

pub const RESTART: &str = "--restart";

pub fn args() -> Vec<PathBuf> {
    std::env::args_os()
        .skip(1)
        .filter(|a| a != RESTART)
        .map(PathBuf::from)
        .map(|p| std::path::absolute(&p).unwrap_or(p))
        .collect()
}

pub fn forward(paths: &[PathBuf]) -> bool {
    let Some(text) = file().and_then(|f| std::fs::read_to_string(f).ok()) else {
        return false;
    };
    let mut parts = text.split_whitespace();
    let (Some(port), Some(token)) = (parts.next(), parts.next()) else {
        return false;
    };
    let Ok(port) = port.parse::<u16>() else {
        return false;
    };
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(300)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    allow_focus();
    let mut message = format!("{token}\n");
    for path in paths {
        message.push_str(&path.to_string_lossy());
        message.push('\n');
    }
    if stream.write_all(message.as_bytes()).is_err() {
        return false;
    }
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut reply = String::new();
    stream.read_to_string(&mut reply).is_ok() && reply == "ok"
}

pub fn listen(ctx: eframe::egui::Context) -> Option<Receiver<PathBuf>> {
    let path = file()?;
    let listener = TcpListener::bind(("127.0.0.1", 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u32(std::process::id());
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default(),
    );
    let token = format!("{:016x}", hasher.finish());
    let _ = std::fs::create_dir_all(path.parent()?);
    let own = format!("{port} {token}");
    std::fs::write(&path, &own).ok()?;
    let _ = OWN.set(own);
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut reader = BufReader::new(&stream);
            let mut first = String::new();
            if reader.read_line(&mut first).is_err() || first.trim_end() != token {
                continue;
            }
            let mut paths = Vec::new();
            for line in reader.lines() {
                let Ok(line) = line else {
                    break;
                };
                if !line.is_empty() {
                    paths.push(PathBuf::from(line));
                }
            }
            let _ = (&stream).write_all(b"ok");
            if paths.is_empty() {
                paths.push(PathBuf::new());
            }
            for path in paths {
                if tx.send(path).is_err() {
                    return;
                }
            }
            ctx.request_repaint();
        }
    });
    Some(rx)
}

#[cfg(windows)]
fn allow_focus() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
    unsafe { AllowSetForegroundWindow(ASFW_ANY) };
}

#[cfg(not(windows))]
fn allow_focus() {}
