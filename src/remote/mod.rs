pub mod panel;
pub mod sync;
pub mod archive;
pub mod import;
pub mod ppk;
pub mod vault;

mod backend;
mod search;
mod ssh;
mod store;

use backend::*;
pub use search::*;
pub use ssh::*;
pub use store::*;

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::trace;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(12);
const IO_TIMEOUT_MS: u32 = 20_000;
const KEEPALIVE: Duration = Duration::from_secs(25);
const CHECK_LIMIT: u64 = 8 * 1024 * 1024;
const KEEP_BACKUPS: usize = 10;

pub fn content_hash(data: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut hasher);
    hasher.finish()
}

pub fn backups_dir() -> Option<PathBuf> {
    eframe::storage_dir("Tror").map(|dir| dir.join("backups"))
}

pub fn backup(profile: &str, path: &str, data: &[u8]) -> Result<PathBuf, String> {
    let root = backups_dir().ok_or(tr!("нет папки данных программы"))?;
    let host: String = profile
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '.' { c } else { '_' })
        .collect();
    let file = crate::remote::sync::local_path(&root.join(host), path.trim_start_matches('/'))
        .ok_or(tr!("небезопасное имя файла"))?;
    let dir = file.parent().ok_or(tr!("нет каталога"))?.to_path_buf();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let target = dir.join(format!("{name}~{stamp}"));
    std::fs::write(&target, data).map_err(|e| e.to_string())?;
    let prefix = format!("{name}~");
    let mut old: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
        .map(|e| e.path())
        .collect();
    old.sort();
    let extra = old.len().saturating_sub(KEEP_BACKUPS);
    for path in &old[..extra] {
        let _ = std::fs::remove_file(path);
    }
    Ok(target)
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub link: bool,
    pub size: u64,
    pub mtime: Option<u64>,
    pub perm: Option<u32>,
    pub owner: String,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Attrs {
    pub perm: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub size: Option<u64>,
    pub mtime: Option<u64>,
    pub atime: Option<u64>,
    pub is_dir: bool,
}

pub fn perm_string(mode: u32, is_dir: bool) -> String {
    let mut out = String::with_capacity(10);
    out.push(if is_dir { 'd' } else { '-' });
    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 7;
        out.push(if bits & 4 != 0 { 'r' } else { '-' });
        out.push(if bits & 2 != 0 { 'w' } else { '-' });
        out.push(if bits & 1 != 0 { 'x' } else { '-' });
    }
    out
}

#[derive(Clone)]
pub enum Cmd {
    TrustHost(bool),
    List(String),
    Download(String),
    Upload {
        path: String,
        data: Vec<u8>,
        doc: Option<(u64, u64)>,
        expect: Option<u64>,
    },
    Mkdir(String),
    Rename(String, String),
    Copy(String, String),
    Archive {
        dir: String,
        names: Vec<String>,
        zip: bool,
    },
    Delete {
        path: String,
        dir: bool,
    },
    Walk(String),
    Stat(String),
    Chmod {
        path: String,
        mode: u32,
        recursive: bool,
        dirs: bool,
        files: bool,
    },
    Touch {
        path: String,
        mtime: u64,
    },
    Extract {
        path: String,
        dest: Option<String>,
    },
}

pub enum Event {
    HostKey {
        fingerprint: String,
        changed: bool,
    },
    Connected {
        home: String,
        fingerprint: Option<String>,
    },
    Listing {
        path: String,
        entries: Vec<Entry>,
    },
    Downloaded {
        path: String,
        data: Vec<u8>,
    },
    Uploaded {
        path: String,
        doc: Option<(u64, u64)>,
    },
    Conflict {
        path: String,
        doc: (u64, u64),
        server: Vec<u8>,
    },
    Changed(String),
    NoTimes(String),
    Progress {
        done: u64,
        total: u64,
    },
    Walked {
        root: String,
        files: Vec<(String, u64)>,
        times: Vec<Option<u64>>,
    },
    Stat {
        path: String,
        attrs: Attrs,
    },
    Log(String, bool),
    Error(String),
    Failed {
        path: String,
        error: String,
    },
    Closed(String),
}

pub struct Connection {
    pub profile: String,
    pub tx: Sender<Cmd>,
    pub rx: Receiver<Event>,
}

pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

pub fn parent(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) | None => "/".into(),
        Some(i) => trimmed[..i].into(),
    }
}

pub fn file_name(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
}

fn sort(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        (!a.is_dir, a.name.to_lowercase()).cmp(&(!b.is_dir, b.name.to_lowercase()))
    });
}

fn describe(cmd: &Cmd) -> String {
    match cmd {
        Cmd::TrustHost(_) => String::new(),
        Cmd::List(path) => format!("[SFTP] opendir/readdir {path}"),
        Cmd::Download(path) => format!("[SFTP] open (read) {path}"),
        Cmd::Upload { path, data, .. } => {
            format!(
                "[SFTP] open (write, create, trunc) {path}, {} bytes",
                data.len()
            )
        }
        Cmd::Mkdir(path) => format!("[SFTP] mkdir {path}"),
        Cmd::Rename(from, to) => format!("[SFTP] rename {from} -> {to}"),
        Cmd::Walk(path) => format!("[SFTP] readdir (recursive) {path}"),
        Cmd::Delete { path, dir: true } => format!("[SFTP] rmdir (recursive) {path}"),
        Cmd::Delete { path, .. } => format!("[SFTP] unlink {path}"),
        Cmd::Stat(path) => format!("[SFTP] stat {path}"),
        Cmd::Chmod {
            path,
            mode,
            recursive,
            ..
        } => format!(
            "[SFTP] setstat {path} mode {mode:o}{}",
            if *recursive { " (recursive)" } else { "" }
        ),
        Cmd::Touch { path, mtime } => format!("[SFTP] setstat {path} mtime {mtime}"),
        Cmd::Copy(from, to) => format!("[SSH] exec: cp -a {} {}", quote(from), quote(to)),
        Cmd::Archive { dir, names, zip } => format!(
            "[SSH] exec: {}",
            crate::remote::archive::remote_command(dir, names, *zip)
        ),
        Cmd::Extract { path, dest } => format!(
            "[SSH] exec: {}",
            extract_command(path, dest.as_deref()).unwrap_or_default()
        ),
    }
}

fn execute(
    backend: &mut Backend,
    cmd: &Cmd,
    progress: &dyn Fn(u64, u64),
    keep: &dyn Fn(&str, &[u8]),
) -> Result<(Event, String), String> {
    Ok(match cmd {
        Cmd::TrustHost(_) => (Event::Log(String::new(), false), String::new()),
        Cmd::List(path) => {
            let entries = backend.list(path)?;
            let summary = format!("{} entries", entries.len());
            (
                Event::Listing {
                    path: path.clone(),
                    entries,
                },
                summary,
            )
        }
        Cmd::Download(path) => {
            let data = backend.download(path, progress)?;
            let summary = format!("{} bytes read", data.len());
            (
                Event::Downloaded {
                    path: path.clone(),
                    data,
                },
                summary,
            )
        }
        Cmd::Upload {
            path,
            data,
            doc,
            expect,
        } => {
            if doc.is_some()
                && let Ok(attrs) = backend.stat(path)
                && !attrs.is_dir
                && attrs.size.is_none_or(|size| size <= CHECK_LIMIT)
            {
                trace::client(format!("[SFTP] read {path} (server copy before saving)"));
                let server = backend.download(path, &|_, _| {})?;
                if let (Some(expect), Some(doc)) = (expect, doc)
                    && content_hash(&server) != *expect
                {
                    return Ok((
                        Event::Conflict {
                            path: path.clone(),
                            doc: *doc,
                            server,
                        },
                        "changed on server, not written".into(),
                    ));
                }
                if server != *data {
                    keep(path, &server);
                }
            }
            if doc.is_some() {
                trace::client(format!("[SFTP] write {path}"));
            }
            backend.upload(path, data, progress)?;
            (
                Event::Uploaded {
                    path: path.clone(),
                    doc: *doc,
                },
                "written".into(),
            )
        }
        Cmd::Mkdir(path) => {
            backend.mkdir(path)?;
            (Event::Changed(parent(path)), "ok".into())
        }
        Cmd::Rename(from, to) => {
            backend.rename(from, to)?;
            (Event::Changed(parent(to)), "ok".into())
        }
        Cmd::Walk(root) => {
            let mut files = Vec::new();
            let mut times = Vec::new();
            backend.walk(root, &mut files, &mut times)?;
            let summary = format!("{} files", files.len());
            (
                Event::Walked {
                    root: root.clone(),
                    files,
                    times,
                },
                summary,
            )
        }
        Cmd::Delete { path, dir } => {
            let mut removed = 0;
            backend.delete_tree(path, *dir, &mut removed)?;
            (Event::Changed(parent(path)), format!("{removed} removed"))
        }
        Cmd::Stat(path) => {
            let attrs = backend.stat(path)?;
            (
                Event::Stat {
                    path: path.clone(),
                    attrs,
                },
                "ok".into(),
            )
        }
        Cmd::Chmod {
            path,
            mode,
            recursive,
            dirs,
            files,
        } => {
            let mut changed = 0;
            backend.chmod(path, *mode, *recursive, *dirs, *files, &mut changed)?;
            (Event::Changed(parent(path)), format!("{changed} changed"))
        }
        Cmd::Touch { path, mtime } => match backend.touch(path, *mtime) {
            Ok(()) => (Event::Changed(parent(path)), "ok".into()),
            Err(e) if matches!(backend, Backend::Ftp(_)) => (Event::NoTimes(path.clone()), format!("MFMT: {e}")),
            Err(e) => return Err(e),
        },
        Cmd::Archive { dir, names, zip } => {
            backend.exec(&crate::remote::archive::remote_command(dir, names, *zip), tr!("архивирование"))?;
            (Event::Changed(dir.clone()), "archived".into())
        }
        Cmd::Copy(from, to) => {
            backend.exec(&format!("cp -a -- {} {}", quote(from), quote(to)), tr!("копирование на сервере"))?;
            (Event::Changed(parent(to)), "copied".into())
        }
        Cmd::Extract { path, dest } => {
            backend.extract(path, dest.as_deref())?;
            (Event::Changed(parent(path)), "extracted".into())
        }
    })
}

const ARCHIVES: [(&str, &str); 13] = [
    (".tar.gz", "tar -xzf {f}"),
    (".tgz", "tar -xzf {f}"),
    (".tar.bz2", "tar -xjf {f}"),
    (".tbz2", "tar -xjf {f}"),
    (".tar.xz", "tar -xJf {f}"),
    (".txz", "tar -xJf {f}"),
    (".tar", "tar -xf {f}"),
    (
        ".zip",
        "if command -v unzip >/dev/null 2>&1; then unzip -oq {f}; elif command -v python3 >/dev/null 2>&1; then python3 -m zipfile -e {f} .; else bsdtar -xf {f}; fi",
    ),
    (".gz", "gzip -dc {f} > {o}"),
    (".bz2", "bzip2 -dc {f} > {o}"),
    (".xz", "xz -dc {f} > {o}"),
    (".7z", "7z x -y {f}"),
    (".rar", "unrar x -o+ {f}"),
];

pub fn is_archive(path: &str) -> bool {
    let lower = path.to_lowercase();
    ARCHIVES.iter().any(|(ext, _)| lower.ends_with(ext))
}

pub fn archive_stem(path: &str) -> Option<&str> {
    let lower = path.to_lowercase();
    let (ext, _) = ARCHIVES.iter().find(|(ext, _)| lower.ends_with(ext))?;
    let name = file_name(path);
    name.get(..name.len().checked_sub(ext.len())?)
}

pub fn extract_command(path: &str, dest: Option<&str>) -> Option<String> {
    let lower = path.to_lowercase();
    let (_, template) = ARCHIVES.iter().find(|(ext, _)| lower.ends_with(ext))?;
    let out = archive_stem(path)?;
    let Some(dest) = dest else {
        let run = template
            .replace("{f}", &quote(file_name(path)))
            .replace("{o}", &quote(out));
        return Some(format!("cd {} && {run}", quote(&parent(path))));
    };
    let run = template
        .replace("{f}", &quote(path))
        .replace("{o}", &quote(out));
    let dest = quote(dest);
    Some(format!("mkdir -p {dest} && cd {dest} && {run}"))
}

const STALL: Duration = Duration::from_secs(40);
const GRACE: Duration = Duration::from_secs(10);

type Escape = Box<dyn Fn() + Send>;

struct Watchdog {
    limit: Duration,
    grace: Duration,
    socket: Arc<std::sync::Mutex<Option<TcpStream>>>,
    since: Arc<std::sync::Mutex<Option<std::time::Instant>>>,
    fired: Arc<AtomicBool>,
    gone: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

impl Watchdog {
    fn new(limit: Duration, grace: Duration) -> Self {
        Watchdog {
            limit,
            grace,
            socket: Default::default(),
            since: Default::default(),
            fired: Default::default(),
            gone: Default::default(),
            stop: Default::default(),
        }
    }

    fn start(&self, socket: Option<TcpStream>, tag: &'static str, escape: Escape) {
        self.replace(socket);
        let (limit, grace) = (self.limit, self.grace);
        let (socket, since, fired, gone, stop) = (
            self.socket.clone(),
            self.since.clone(),
            self.fired.clone(),
            self.gone.clone(),
            self.stop.clone(),
        );
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) && !gone.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(250));
                let Some(waited) = since.lock().ok().and_then(|busy| busy.map(|t| t.elapsed())) else {
                    continue;
                };
                if waited > limit && !fired.swap(true, Ordering::Relaxed) {
                    if let Some(socket) = socket.lock().ok().as_deref().and_then(Option::as_ref) {
                        let _ = socket.shutdown(std::net::Shutdown::Both);
                    }
                    trace::line(
                        tf!(
                            "{tag} сервер не отвечает {} с — соединение закрыто",
                            "{tag} the server has not answered for {} s, connection closed",
                            limit.as_secs()
                        ),
                        true,
                    );
                } else if waited > limit + grace {
                    gone.store(true, Ordering::Relaxed);
                    escape();
                }
            }
        });
    }

    fn replace(&self, socket: Option<TcpStream>) {
        if let Ok(mut slot) = self.socket.lock() {
            *slot = socket;
        }
    }

    fn alive(&self) {
        if let Ok(mut busy) = self.since.lock()
            && busy.is_some()
            && !self.fired.load(Ordering::Relaxed)
        {
            *busy = Some(std::time::Instant::now());
        }
    }

    fn guard<T>(&self, work: impl FnOnce() -> T) -> T {
        self.fired.store(false, Ordering::Relaxed);
        if let Ok(mut busy) = self.since.lock() {
            *busy = Some(std::time::Instant::now());
        }
        let out = work();
        if let Ok(mut busy) = self.since.lock() {
            *busy = None;
        }
        out
    }

    fn fired(&self) -> bool {
        self.fired.load(Ordering::Relaxed)
    }

    fn gone(&self) -> bool {
        self.gone.load(Ordering::Relaxed)
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn timed_out(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    ["timed out", "session(-9)", "timeout", "os error 11)", "os error 35)", "os error 10060)"]
        .iter()
        .any(|needle| lower.contains(needle))
}

fn serve(
    profile: Profile,
    known: Option<String>,
    cmds: Receiver<Cmd>,
    events: Sender<Event>,
    wake: Arc<dyn Fn() + Send + Sync>,
) {
    let send = |event: Event| {
        let _ = events.send(event);
        wake();
    };
    let log = |text: String, error: bool| {
        trace::line(text, error);
        wake();
    };
    let watch = Watchdog::new(STALL, GRACE);
    let last = std::cell::Cell::new(std::time::Instant::now());
    let progress = |done: u64, total: u64| {
        watch.alive();
        if done >= total || last.get().elapsed() > Duration::from_millis(60) {
            last.set(std::time::Instant::now());
            send(Event::Progress { done, total });
        }
    };
    let keep = |path: &str, data: &[u8]| {
        if let Err(e) = backup(&profile.name, path, data) {
            trace::line(tf!("[Копии] не удалось сохранить копию {path}: {e}", "[Backups] could not save a copy of {path}: {e}"), true);
        }
    };
    let sftp = profile.protocol == Protocol::Sftp;
    let tag = match profile.protocol {
        Protocol::Sftp => "[SFTP]",
        Protocol::Ftp => "[FTP]",
        Protocol::Ftps | Protocol::FtpsImplicit => "[FTPS]",
    };
    let mut known = known;
    let open = |known: Option<&str>| match profile.protocol {
        Protocol::Sftp => connect_sftp(&profile, known, &events, &cmds, &*wake),
        _ => connect_ftp(&profile).map(|(b, home)| (b, home, None)),
    };
    log(
        format!(
            "{tag} connecting to {} as {}",
            profile.address(),
            profile.user.trim()
        ),
        false,
    );
    let (mut backend, home, fingerprint) = match open(known.as_deref()) {
        Ok(ok) => ok,
        Err(e) => {
            log(format!("{tag} connection failed: {e}"), true);
            send(Event::Closed(e));
            return;
        }
    };
    if fingerprint.is_some() {
        known = fingerprint.clone();
    }
    let escape = {
        let (events, wake) = (events.clone(), wake.clone());
        Box::new(move || {
            let _ = events.send(Event::Closed(tr!("сервер не отвечает").into()));
            wake();
        })
    };
    watch.start(backend.socket(), tag, escape);
    log(format!("{tag} connected, home directory {home}"), false);
    let start = profile.directory.trim();
    send(Event::Connected {
        home: if start.is_empty() {
            home
        } else {
            start.to_owned()
        },
        fingerprint,
    });
    let mut alive = true;
    loop {
        if watch.gone() {
            return;
        }
        let cmd = match cmds.recv_timeout(KEEPALIVE) {
            Ok(cmd) => cmd,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if alive && watch.guard(|| backend.ping()).is_err() {
                    alive = false;
                    backend.abandon();
                    log(format!("{tag} idle connection closed by server"), false);
                }
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if matches!(cmd, Cmd::TrustHost(_)) {
            continue;
        }
        let quiet = matches!(cmd, Cmd::Download(_) | Cmd::Upload { doc: None, .. } | Cmd::Touch { .. });
        if sftp && !quiet {
            trace::client(describe(&cmd));
        }
        let mut result = if alive {
            watch.guard(|| execute(&mut backend, &cmd, &progress, &keep))
        } else {
            Err("connection closed".into())
        };
        if watch.gone() {
            return;
        }
        let stalled = watch.fired() || result.as_ref().is_err_and(|e| timed_out(e));
        if result.is_err() && (!alive || stalled || watch.guard(|| backend.ping()).is_err()) {
            if let Err(e) = &result {
                log(format!("{tag} {e}"), true);
            }
            backend.abandon();
            log(format!("{tag} connection lost, reconnecting"), false);
            match open(known.as_deref()) {
                Ok((fresh, _, fingerprint)) => {
                    backend = fresh;
                    watch.replace(backend.socket());
                    alive = true;
                    if fingerprint.is_some() {
                        known = fingerprint;
                    }
                    log(format!("{tag} reconnected, retrying"), false);
                    if sftp && !quiet {
                        trace::client(describe(&cmd));
                    }
                    result = watch.guard(|| execute(&mut backend, &cmd, &progress, &keep));
                }
                Err(e) => {
                    log(format!("{tag} reconnect failed: {e}"), true);
                    send(Event::Closed(e));
                    return;
                }
            }
        }
        match result {
            Ok((event, summary)) => {
                if sftp && !quiet {
                    trace::server(format!("[SFTP] {summary}"));
                }
                send(event);
            }
            Err(e) => {
                log(format!("{tag} {e}"), true);
                match &cmd {
                    Cmd::Download(path) | Cmd::Upload { path, .. } | Cmd::Touch { path, .. } => {
                        send(Event::Failed {
                            path: path.clone(),
                            error: e,
                        })
                    }
                    _ => send(Event::Error(e)),
                }
            }
        }
    }
    log(format!("{tag} disconnected"), false);
}

pub fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn timed(started: std::time::Instant, result: Result<String, String>) -> Result<String, String> {
    let result = result.map(|text| tf!("{text} ({} мс)", "{text} ({} ms)", started.elapsed().as_millis()));
    match &result {
        Ok(text) => trace::server(format!("[TEST] ok: {text}")),
        Err(e) => trace::line(format!("[TEST] failed: {e}"), true),
    }
    result
}

pub fn test(profile: &Profile, known: Option<String>) -> Result<String, String> {
    let started = std::time::Instant::now();
    trace::client(format!(
        "[TEST] {} {} as {}",
        profile.protocol.label(),
        profile.address(),
        profile.user.trim()
    ));
    let result = match profile.protocol {
        Protocol::Sftp => ssh_probe(profile, known.as_deref()).and_then(|(session, note)| {
            let sftp = session.sftp().map_err(|e| tf!("SFTP недоступен: {e}", "SFTP is not available: {e}"))?;
            let home = sftp
                .realpath(Path::new("."))
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| "/".into());
            let start = profile.directory.trim();
            if !start.is_empty() && sftp.stat(Path::new(start)).is_err() {
                return Err(tf!("вход выполнен, но начальной папки {start} нет на сервере", "logged in, but the start folder {start} does not exist on the server"
                ));
            }
            Ok(tf!("вход выполнен, домашняя папка {home}{note}", "logged in, home folder {home}{note}"))
        }),
        _ => connect_ftp(profile).and_then(|(mut backend, home)| {
            let start = profile.directory.trim();
            if !start.is_empty() && backend.list(start).is_err() {
                return Err(tf!("вход выполнен, но начальной папки {start} нет на сервере", "logged in, but the start folder {start} does not exist on the server"
                ));
            }
            if let Backend::Ftp(ftp) = &mut backend {
                let _ = ftp.quit();
            }
            Ok(tf!("вход выполнен, папка {home}", "logged in, folder {home}"))
        }),
    };
    timed(started, result)
}

pub fn test_terminal(profile: &TermProfile, known: Option<String>) -> Result<String, String> {
    let started = std::time::Instant::now();
    trace::client(format!(
        "[TEST] {} {} as {}",
        profile.protocol.label(),
        profile.address(),
        profile.user.trim()
    ));
    let result = match profile.protocol {
        TermProtocol::Local => Ok(crate::console::local::program()),
        TermProtocol::Telnet => (|| {
            let address = profile
                .address()
                .to_socket_addrs()
                .map_err(|e| tf!("не удалось найти хост: {e}", "could not resolve the host: {e}"))?
                .next()
                .ok_or(tr!("не удалось найти хост"))?;
            let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)
                .map_err(|e| tf!("нет соединения: {e}", "no connection: {e}"))?;
            stream
                .set_read_timeout(Some(Duration::from_millis(1500)))
                .ok();
            let mut buf = [0u8; 2048];
            let n = stream.read(&mut buf).unwrap_or(0);
            let text: String = String::from_utf8_lossy(&buf[..n])
                .chars()
                .filter(|c| !c.is_control() || *c == '\n')
                .collect();
            let line = text
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty() && l.chars().all(|c| (c as u32) < 0xfffd))
                .unwrap_or("")
                .chars()
                .take(80)
                .collect::<String>();
            Ok(if line.is_empty() {
                tr!("порт открыт, сервер принял соединение").to_owned()
            } else {
                tf!("порт открыт, сервер ответил: «{line}»", "port open, the server replied: \"{line}\"")
            })
        })(),
        TermProtocol::Ssh => ssh_probe(&profile.ssh(), known.as_deref())
            .map(|(_, note)| tf!("вход выполнен{note}", "logged in{note}")),
        TermProtocol::Mosh => ssh_probe(&profile.ssh(), known.as_deref()).and_then(|(session, note)| {
            let server = if profile.mosh_server.trim().is_empty() {
                "mosh-server"
            } else {
                profile.mosh_server.trim()
            };
            let mut channel = session.channel_session().map_err(|e| e.to_string())?;
            channel
                .exec(&format!("command -v {server} || which {server}"))
                .map_err(|e| e.to_string())?;
            let mut out = String::new();
            let _ = channel.read_to_string(&mut out);
            let _ = channel.wait_close();
            let path = out.lines().next().unwrap_or("").trim().to_owned();
            if path.is_empty() {
                Err(tf!("вход по SSH выполнен, но {server} на сервере не найден — установите пакет mosh", "logged in over SSH, but {server} was not found on the server — install the mosh package"
                ))
            } else {
                Ok(tf!("вход выполнен, найден {path}{note}", "logged in, found {path}{note}"))
            }
        }),
    };
    timed(started, result)
}

pub enum OneShot {
    List(String),
    Read(String),
    Write(String, Vec<u8>),
}

pub fn one_shot(profile: &Profile, known: Option<&str>, op: OneShot) -> Result<serde_json::Value, String> {
    let mut backend = match profile.protocol {
        Protocol::Sftp => {
            let (session, _) = open_session(profile, known, &|_, _| false)?;
            let sftp = session.sftp().map_err(|e| e.to_string())?;
            Backend::Sftp {
                session,
                sftp,
                socket: None,
            }
        }
        _ => connect_ftp(profile)?.0,
    };
    match op {
        OneShot::List(path) => {
            let entries = backend.list(&path)?;
            Ok(serde_json::Value::Array(
                entries
                    .into_iter()
                    .map(|e| {
                        serde_json::json!({
                            "name": e.name,
                            "dir": e.is_dir,
                            "size": e.size,
                            "mtime": e.mtime,
                        })
                    })
                    .collect(),
            ))
        }
        OneShot::Read(path) => {
            let bytes = backend.download(&path, &|_, _| {})?;
            Ok(serde_json::Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        }
        OneShot::Write(path, data) => {
            backend.upload(&path, &data, &|_, _| {})?;
            Ok(serde_json::Value::Null)
        }
    }
}

pub fn connect(
    profile: &Profile,
    known: Option<String>,
    wake: impl Fn() + Send + 'static,
) -> Connection {
    let (tx, cmds) = channel();
    let (events, rx) = channel();
    let name = profile.name.clone();
    let profile = profile.clone();
    let wake = std::sync::Mutex::new(wake);
    let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        if let Ok(wake) = wake.lock() {
            wake();
        }
    });
    std::thread::spawn(move || serve(profile, known, cmds, events, wake));
    Connection {
        profile: name,
        tx,
        rx,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(join("/var/www", "index.php"), "/var/www/index.php");
        assert_eq!(join("/", "etc"), "/etc");
        assert_eq!(parent("/var/www/site"), "/var/www");
        assert_eq!(parent("/var"), "/");
        assert_eq!(parent("/"), "/");
        assert_eq!(file_name("/var/www/index.php"), "index.php");
    }

    #[test]
    fn store_roundtrip_masks_password() {
        let _guard = crate::remote::vault::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::remote::vault::reset_for_test();
        crate::remote::vault::set_in_file(true);
        let dir = std::env::temp_dir().join(format!("tror-test-{}", std::process::id()));
        let path = dir.join("connections.json");
        let mut store = Store::load(Some(path.clone()));
        assert!(store.profiles.is_empty());
        store.profiles.push(Profile {
            name: "Клиент".into(),
            protocol: Protocol::Ftp,
            host: "example.org".into(),
            port: 21,
            user: "admin".into(),
            password: "s3cret пароль".into(),
            key_file: String::new(),
            passphrase: String::new(),
            directory: "/www".into(),
            url: "https://example.org".into(),
            insecure_tls: false,
            ..Default::default()
        });
        store
            .known_hosts
            .insert("example.org:22".into(), "ab12".into());
        store.save().unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("s3cret"));
        assert!(raw.contains("example.org"));
        #[cfg(unix)]
        assert_eq!(
            std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&path).unwrap().permissions()) & 0o777,
            0o600
        );
        let loaded = Store::load(Some(path));
        assert_eq!(loaded.profiles, store.profiles);
        assert_eq!(loaded.known_hosts, store.known_hosts);
        std::fs::remove_dir_all(dir).ok();
    }

    fn vault_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tror-vault-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("connections.json")
    }

    fn sample() -> Profile {
        Profile {
            name: "Клиент".into(),
            host: "example.org".into(),
            user: "admin".into(),
            password: "s3cret".into(),
            ..Profile::new("x".into())
        }
    }

    #[test]
    fn old_masked_passwords_move_to_the_vault() {
        let _guard = crate::remote::vault::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::remote::vault::reset_for_test();
        MIGRATED.store(false, Ordering::Relaxed);
        let path = vault_dir("migrate");
        let hidden = masked::hide("s3cret");
        std::fs::write(
            &path,
            format!(r#"{{"profiles":[{{"name":"Клиент","host":"example.org","port":22,"user":"admin","password":"{hidden}"}}],"databases":[{{"name":"b","password":"{}"}}]}}"#, masked::hide("dbpw")),
        )
        .unwrap();
        let store = Store::load(Some(path.clone()));
        assert_eq!(store.profiles[0].password, "s3cret");
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains(&hidden), "{raw}");
        assert!(raw.contains("example.org"));
        crate::remote::vault::reset_cache_for_test();
        let again = Store::load(Some(path.clone()));
        assert_eq!(again.profiles[0].password, "s3cret");
        assert_eq!(again.databases[0].password, "dbpw");
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn switching_modes_moves_passwords_both_ways() {
        let _guard = crate::remote::vault::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::remote::vault::reset_for_test();
        let path = vault_dir("switch");
        let mut store = Store::load(Some(path.clone()));
        store.profiles.push(sample());
        store.save().unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains(&masked::hide("s3cret")));
        Store::switch_vault(Some(path.clone()), true).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains(&masked::hide("s3cret")));
        assert!(crate::remote::vault::backend::ENTRIES.lock().unwrap().is_empty());
        Store::switch_vault(Some(path.clone()), false).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains(&masked::hide("s3cret")));
        crate::remote::vault::reset_cache_for_test();
        assert_eq!(Store::load(Some(path.clone())).profiles[0].password, "s3cret");
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn broken_vault_keeps_passwords_in_file_and_loses_nothing() {
        let _guard = crate::remote::vault::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::remote::vault::reset_for_test();
        let path = vault_dir("broken");
        let mut store = Store::load(Some(path.clone()));
        store.profiles.push(sample());
        store.save().unwrap();
        crate::remote::vault::reset_cache_for_test();
        crate::remote::vault::backend::BROKEN.store(true, Ordering::Relaxed);
        let mut locked = Store::load(Some(path.clone()));
        assert!(locked.locked);
        let mut other = sample();
        other.name = "Новый".into();
        other.password = "fresh".into();
        locked.profiles.push(other);
        locked.save().unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains(&masked::hide("fresh")));
        assert!(Store::switch_vault(Some(path.clone()), false).is_err());
        crate::remote::vault::backend::BROKEN.store(false, Ordering::Relaxed);
        crate::remote::vault::reset_cache_for_test();
        let fine = Store::load(Some(path.clone()));
        assert_eq!(fine.profiles[0].password, "s3cret");
        assert_eq!(fine.profiles[1].password, "fresh");
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn entries_sorted_dirs_first() {
        let mut entries = vec![
            Entry {
                name: "b.php".into(),
                is_dir: false,
                size: 1,
                mtime: None,
                ..Default::default()
            },
            Entry {
                name: "Zeta".into(),
                is_dir: true,
                size: 0,
                mtime: None,
                ..Default::default()
            },
            Entry {
                name: "a.php".into(),
                is_dir: false,
                size: 1,
                mtime: None,
                ..Default::default()
            },
            Entry {
                name: "alpha".into(),
                is_dir: true,
                size: 0,
                mtime: None,
                ..Default::default()
            },
        ];
        sort(&mut entries);
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "Zeta", "a.php", "b.php"]);
    }

    fn exercise(protocol: Protocol, port: u16) {
        let profile = Profile {
            name: "test".into(),
            protocol,
            host: "127.0.0.1".into(),
            port,
            user: "jozef".into(),
            password: "secret".into(),
            key_file: String::new(),
            passphrase: String::new(),
            directory: String::new(),
            url: String::new(),
            insecure_tls: protocol == Protocol::Ftps,
            ..Default::default()
        };
        let conn = connect(&profile, None, || {});
        let next = || loop {
            match conn
                .rx
                .recv_timeout(Duration::from_secs(20))
                .expect("ответ сервера")
            {
                Event::Log(..) | Event::Progress { .. } => {}
                event => break event,
            }
        };
        let home = loop {
            match next() {
                Event::HostKey {
                    fingerprint,
                    changed,
                } => {
                    assert_eq!(fingerprint.len(), 64);
                    assert!(!changed);
                    conn.tx.send(Cmd::TrustHost(true)).unwrap();
                }
                Event::Connected { home, .. } => break home,
                Event::Closed(e) | Event::Error(e) => panic!("{e}"),
                _ => panic!("неожиданное событие"),
            }
        };
        let www = join(&home, "www");
        conn.tx.send(Cmd::List(www.clone())).unwrap();
        let Event::Listing { entries, .. } = next() else {
            panic!("нет списка")
        };
        let names: Vec<_> = entries
            .iter()
            .map(|e| (e.name.as_str(), e.is_dir))
            .collect();
        assert_eq!(names[0], ("css", true));
        assert!(names.contains(&("index.php", false)));

        let file = join(&www, &format!("tror-{}.txt", protocol.label()));
        let body = "привет\nсервер".as_bytes().to_vec();
        conn.tx
            .send(Cmd::Upload {
                path: file.clone(),
                data: body.clone(),
                doc: Some((7, 9)),
                expect: None,
            })
            .unwrap();
        let Event::Uploaded { doc, .. } = next() else {
            panic!("нет загрузки")
        };
        assert_eq!(doc, Some((7, 9)));
        conn.tx.send(Cmd::Download(file.clone())).unwrap();
        let Event::Downloaded { data, .. } = next() else {
            panic!("нет файла")
        };
        assert_eq!(data, body);

        let dir = join(&www, &format!("dir-{}", protocol.label()));
        conn.tx.send(Cmd::Mkdir(dir.clone())).unwrap();
        assert!(matches!(next(), Event::Changed(p) if p == www));
        let moved = join(&dir, "moved.txt");
        conn.tx.send(Cmd::Rename(file, moved.clone())).unwrap();
        assert!(matches!(next(), Event::Changed(p) if p == dir));
        let deep = join(&dir, "a/b/deep.bin");
        let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        conn.tx
            .send(Cmd::Upload {
                path: deep.clone(),
                data: big.clone(),
                doc: None,
                expect: None,
            })
            .unwrap();
        assert!(matches!(next(), Event::Uploaded { .. }));
        conn.tx.send(Cmd::Walk(dir.clone())).unwrap();
        let Event::Walked { files, .. } = next() else {
            panic!("нет обхода")
        };
        assert_eq!(
            files,
            vec![(deep.clone(), 300_000), (moved.clone(), body.len() as u64)]
        );
        conn.tx.send(Cmd::Download(deep)).unwrap();
        let Event::Downloaded { data, .. } = next() else {
            panic!("нет файла")
        };
        assert!(data == big);
        conn.tx
            .send(Cmd::Delete {
                path: dir.clone(),
                dir: true,
            })
            .unwrap();
        assert!(matches!(next(), Event::Changed(p) if p == www));
        conn.tx.send(Cmd::List(www.clone())).unwrap();
        let Event::Listing { entries, .. } = next() else {
            panic!("нет списка")
        };
        assert!(entries.iter().all(|e| !e.name.starts_with("dir-")));
        conn.tx
            .send(Cmd::Download(join(&www, "missing.txt")))
            .unwrap();
        assert!(matches!(next(), Event::Failed { .. }));
    }

    #[test]
    fn watchdog_closes_a_stuck_connection() {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let peer = Arc::new(std::sync::Mutex::new(Some(listener.accept().unwrap().0)));
        let escaped = Arc::new(AtomicBool::new(false));
        let watch = Watchdog::new(Duration::from_millis(600), Duration::from_millis(600));
        let escape = {
            let (peer, escaped) = (peer.clone(), escaped.clone());
            Box::new(move || {
                escaped.store(true, Ordering::Relaxed);
                peer.lock().unwrap().take();
            })
        };
        watch.start(stream.try_clone().ok(), "[SFTP]", escape);
        let started = std::time::Instant::now();
        let read = watch.guard(|| stream.read(&mut [0u8; 8]));
        assert!(matches!(read, Ok(0) | Err(_)));
        assert!(watch.fired());
        assert!(started.elapsed() < Duration::from_secs(5));
        if cfg!(unix) {
            assert!(!escaped.load(Ordering::Relaxed));
            let quick = watch.guard(|| 1);
            assert_eq!(quick, 1);
            assert!(!watch.fired());
        }
    }

    #[test]
    fn watchdog_releases_the_panel_when_the_call_never_returns() {
        let escaped = Arc::new(AtomicBool::new(false));
        let watch = Watchdog::new(Duration::from_millis(300), Duration::from_millis(300));
        let flag = escaped.clone();
        watch.start(None, "[SFTP]", Box::new(move || flag.store(true, Ordering::Relaxed)));
        let started = std::time::Instant::now();
        watch.guard(|| {
            while !escaped.load(Ordering::Relaxed) && started.elapsed() < Duration::from_secs(5) {
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        assert!(escaped.load(Ordering::Relaxed));
        assert!(watch.gone());
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn stalls_are_recognised() {
        assert!(timed_out("[Session(-9)] Timed out waiting on socket"));
        assert!(timed_out("Resource temporarily unavailable (os error 11)"));
        assert!(timed_out("Попытка установить соединение была безуспешной (os error 10060)"));
        assert!(!timed_out("[SFTP(2)] no such file"));
    }

    #[cfg(unix)]
    #[test]
    #[ignore]
    fn live_stalled_server_reconnects_quickly() {
        let profile = Profile {
            name: "p".into(),
            host: "127.0.0.1".into(),
            port: 2222,
            user: "sftpu".into(),
            password: "test123".into(),
            ..Default::default()
        };
        let conn = connect(&profile, None, || {});
        loop {
            match conn.rx.recv_timeout(Duration::from_secs(20)).unwrap() {
                Event::HostKey { .. } => conn.tx.send(Cmd::TrustHost(true)).unwrap(),
                Event::Connected { .. } => break,
                _ => {}
            }
        }
        let signal = |sig: &str| {
            let found = std::process::Command::new("pgrep").args(["-u", "sftpu"]).output().unwrap();
            for pid in String::from_utf8_lossy(&found.stdout).split_whitespace() {
                std::process::Command::new("kill").args([sig, pid]).status().unwrap();
            }
        };
        signal("-STOP");
        let started = std::time::Instant::now();
        conn.tx.send(Cmd::List("/home/sftpu/www".into())).unwrap();
        let event = loop {
            match conn.rx.recv_timeout(Duration::from_secs(120)).unwrap() {
                Event::Log(..) | Event::Progress { .. } => {}
                event => break event,
            }
        };
        signal("-CONT");
        assert!(matches!(event, Event::Listing { .. }));
        assert!(started.elapsed() < Duration::from_secs(30), "{:?}", started.elapsed());
    }

    #[cfg(unix)]
    #[test]
    #[ignore]
    fn live_forced_sftp_account_saves_without_hanging() {
        let target = "/home/sftponly/www/a.txt";
        std::fs::write(target, "old\n").unwrap();
        let profile = Profile {
            name: "only".into(),
            host: "127.0.0.1".into(),
            port: 2223,
            user: "sftponly".into(),
            password: "test123".into(),
            ..Default::default()
        };
        let conn = connect(&profile, None, || {});
        let next = || loop {
            match conn.rx.recv_timeout(Duration::from_secs(20)).expect("ответ сервера") {
                Event::Log(..) | Event::Progress { .. } => {}
                event => break event,
            }
        };
        loop {
            match next() {
                Event::HostKey { .. } => conn.tx.send(Cmd::TrustHost(true)).unwrap(),
                Event::Connected { .. } => break,
                Event::Closed(e) | Event::Error(e) => panic!("{e}"),
                _ => {}
            }
        }
        let started = std::time::Instant::now();
        conn.tx
            .send(Cmd::Upload {
                path: target.into(),
                data: b"new content".to_vec(),
                doc: None,
                expect: None,
            })
            .unwrap();
        let event = next();
        assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
        assert!(matches!(event, Event::Uploaded { .. }));
        assert_eq!(std::fs::read_to_string(target).unwrap(), "new content");
        let left: Vec<_> = std::fs::read_dir("/home/sftponly/www").unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left.len(), 1, "{left:?}");
    }

    #[cfg(unix)]
    #[test]
    #[ignore]
    fn live_sftp_upload_overwrites_in_place() {
        use std::os::unix::fs::PermissionsExt;
        let base = std::env::temp_dir().join("tror-upload-test");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o777)).unwrap();
        let target = base.join("index.php");
        std::fs::write(&target, "old").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o666)).unwrap();
        let profile = Profile {
            name: "test".into(),
            host: "127.0.0.1".into(),
            port: 2222,
            user: "jozef".into(),
            password: "secret".into(),
            ..Default::default()
        };
        let conn = connect(&profile, None, || {});
        let next = || loop {
            match conn.rx.recv_timeout(Duration::from_secs(20)).expect("ответ сервера") {
                Event::Log(..) | Event::Progress { .. } => {}
                event => break event,
            }
        };
        loop {
            match next() {
                Event::HostKey { .. } => conn.tx.send(Cmd::TrustHost(true)).unwrap(),
                Event::Connected { .. } => break,
                Event::Closed(e) | Event::Error(e) => panic!("{e}"),
                _ => {}
            }
        }
        conn.tx
            .send(Cmd::Upload {
                path: target.to_string_lossy().into_owned(),
                data: b"new content".to_vec(),
                doc: None,
                expect: None,
            })
            .unwrap();
        let _ = next();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new content");
        assert_eq!(std::fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o666);
        let left: Vec<_> = std::fs::read_dir(&base).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left.len(), 1, "{left:?}");
    }

    #[cfg(unix)]
    #[test]
    #[ignore]
    fn live_sftp_delete_keeps_symlink_targets() {
        let base = std::env::temp_dir().join("tror-link-test");
        let _ = std::fs::remove_dir_all(&base);
        let doomed = base.join("doomed");
        let keep = base.join("keep");
        std::fs::create_dir_all(&doomed).unwrap();
        std::fs::create_dir_all(&keep).unwrap();
        std::fs::write(keep.join("data.txt"), "important").unwrap();
        std::fs::write(doomed.join("a.txt"), "x").unwrap();
        std::os::unix::fs::symlink(&keep, doomed.join("link")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        for dir in [&base, &doomed, &keep] {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        }
        let profile = Profile {
            name: "test".into(),
            host: "127.0.0.1".into(),
            port: 2222,
            user: "jozef".into(),
            password: "secret".into(),
            ..Default::default()
        };
        let conn = connect(&profile, None, || {});
        let next = || loop {
            match conn.rx.recv_timeout(Duration::from_secs(20)).expect("ответ сервера") {
                Event::Log(..) | Event::Progress { .. } => {}
                event => break event,
            }
        };
        loop {
            match next() {
                Event::HostKey { .. } => conn.tx.send(Cmd::TrustHost(true)).unwrap(),
                Event::Connected { .. } => break,
                Event::Closed(e) | Event::Error(e) => panic!("{e}"),
                _ => {}
            }
        }
        conn.tx
            .send(Cmd::Delete {
                path: doomed.to_string_lossy().into_owned(),
                dir: true,
            })
            .unwrap();
        let _ = next();
        assert!(!doomed.exists(), "папка не удалена");
        assert!(keep.join("data.txt").exists(), "удалено содержимое по ссылке");
    }

    #[test]
    #[ignore]
    fn live_sftp_server() {
        exercise(Protocol::Sftp, 2222);
    }

    #[test]
    #[ignore]
    fn live_ftp_server() {
        exercise(Protocol::Ftp, 2121);
    }

    #[test]
    #[ignore]
    fn live_ftps_server() {
        exercise(Protocol::Ftps, 2122);
        let mut profile = Profile::new("x".into());
        profile.protocol = Protocol::Ftps;
        profile.host = "127.0.0.1".into();
        profile.port = 2122;
        profile.user = "jozef".into();
        profile.password = "secret".into();
        let error = connect_ftp(&profile)
            .err()
            .expect("самоподписанный сертификат принят");
        assert!(error.contains("защищённое соединение"), "{error}");
    }

    #[test]
    #[ignore]
    fn live_sftp_rejects_wrong_password_and_changed_key() {
        let mut profile = Profile::new("x".into());
        profile.host = "127.0.0.1".into();
        profile.port = 2222;
        profile.user = "jozef".into();
        profile.password = "wrong".into();
        let conn = connect(&profile, Some("00".repeat(32)), || {});
        let next = || loop {
            match conn.rx.recv_timeout(Duration::from_secs(20)) {
                Ok(Event::Log(..)) | Ok(Event::Progress { .. }) => {}
                other => break other,
            }
        };
        let Ok(Event::HostKey { changed, .. }) = next() else {
            panic!("нет запроса ключа")
        };
        assert!(changed);
        conn.tx.send(Cmd::TrustHost(true)).unwrap();
        assert!(matches!(next(), Ok(Event::Closed(_))));
    }

    fn names_params() -> SearchParams {
        SearchParams {
            root: "/w".into(),
            query: String::new(),
            content: false,
            excludes: Vec::new(),
            case: false,
            word: false,
            regex: false,
        }
    }

    #[test]
    fn extract_commands() {
        assert_eq!(
            extract_command("/w/it's.tar.gz", None).as_deref(),
            Some("cd '/w' && tar -xzf 'it'\\''s.tar.gz'")
        );
        assert_eq!(
            extract_command("/w/a.sql.GZ", None).as_deref(),
            Some("cd '/w' && gzip -dc 'a.sql.GZ' > 'a.sql'")
        );
        assert!(
            extract_command("/w/a.zip", None)
                .unwrap()
                .contains("unzip -oq 'a.zip'")
        );
        assert_eq!(
            extract_command("/w/a.tar.gz", Some("/w/a")).as_deref(),
            Some("mkdir -p '/w/a' && cd '/w/a' && tar -xzf '/w/a.tar.gz'")
        );
        assert_eq!(archive_stem("/w/site.backup.tar.xz"), Some("site.backup"));
        assert!(is_archive("/w/b.TGZ"));
        assert!(!is_archive("/w/b.php"));
    }

    #[test]
    fn search_command_and_grep_parsing() {
        let params = SearchParams {
            root: "/var/www/site".into(),
            query: "it's $x".into(),
            content: true,
            excludes: vec![
                "bitrix/cache".into(),
                " node_modules ".into(),
                String::new(),
            ],
            case: false,
            word: false,
            regex: false,
        };
        let cmd = search_command(&params);
        assert!(cmd.contains("find -H '/var/www/site' \\( -path '*/bitrix/cache' -o -name 'node_modules' \\) -prune -o -type f"));
        assert!(cmd.contains("grep -InH -i -F -e 'it'\\''s $x' --"));
        let names = search_command(&SearchParams {
            content: false,
            query: "a*b".into(),
            excludes: Vec::new(),
            ..params
        });
        assert!(names.contains("find -H '/var/www/site' -iname '*a\\*b*' -print"));
        let strict = search_command(&SearchParams {
            content: true,
            query: "x".into(),
            case: true,
            word: true,
            regex: true,
            ..names_params()
        });
        assert!(strict.contains("grep -InH -w -E -e 'x' --"));
        let named = search_command(&SearchParams {
            content: false,
            query: "Index.PHP".into(),
            word: true,
            ..names_params()
        });
        assert!(named.contains("RE='(^|[^[:alnum:]_])(index\\.php)([^[:alnum:]_]|$)' awk -F/ 'tolower($NF) ~ ENVIRON[\"RE\"]'"));

        assert_eq!(
            parse_grep_line("/www/a:b.php:12:  echo $x; // 3:4"),
            Some(("/www/a:b.php".into(), 12, "echo $x; // 3:4".into()))
        );
        assert_eq!(parse_grep_line("no line number here"), None);
        assert!(excluded(
            "/www/bitrix/cache",
            "cache",
            &["bitrix/cache".into()]
        ));
        assert!(!excluded(
            "/www/local/cache",
            "cache",
            &["bitrix/cache".into()]
        ));
        assert!(excluded("/www/x/.git", ".git", &[".git".into()]));
    }

    fn collect(job: SearchJob) -> (Vec<(String, Option<usize>, String)>, String) {
        let mut hits = Vec::new();
        loop {
            match job.rx.recv_timeout(Duration::from_secs(30)).expect("поиск") {
                SearchEvent::Hit { path, line, text } => hits.push((path, line, text)),
                SearchEvent::Status(_) => {}
                SearchEvent::Done(note) => return (hits, note),
                SearchEvent::Failed(e) => panic!("{e}"),
            }
        }
    }

    fn live_search(protocol: Protocol, port: u16) -> String {
        let mut profile = Profile::new("s".into());
        profile.protocol = protocol;
        profile.host = "127.0.0.1".into();
        profile.port = port;
        profile.user = "jozef".into();
        profile.password = "secret".into();
        let known = if protocol == Protocol::Sftp {
            let conn = connect(&profile, None, || {});
            loop {
                match conn.rx.recv_timeout(Duration::from_secs(20)).unwrap() {
                    Event::HostKey { .. } => conn.tx.send(Cmd::TrustHost(true)).unwrap(),
                    Event::Connected { fingerprint, .. } => break fingerprint,
                    _ => {}
                }
            }
        } else {
            None
        };
        let root = "/search".to_owned();
        let text = |excludes: Vec<String>| SearchParams {
            root: root.clone(),
            query: "needle_42".into(),
            content: true,
            excludes,
            case: false,
            word: false,
            regex: false,
        };
        let (hits, note) = collect(search(&profile, known.clone(), text(Vec::new()), || {}));
        let mut found: Vec<_> = hits.iter().map(|h| (h.0.as_str(), h.1)).collect();
        found.sort();
        assert_eq!(
            found,
            vec![
                ("/search/a.php", Some(2)),
                ("/search/bitrix/cache/c.php", Some(1)),
                ("/search/sub/b.js", Some(1)),
            ],
            "{note}"
        );
        assert!(hits.iter().any(|h| h.2 == "echo 'NEEDLE_42';"));
        let (hits, _) = collect(search(
            &profile,
            known.clone(),
            text(vec!["bitrix/cache".into()]),
            || {},
        ));
        assert_eq!(hits.len(), 2);
        let (hits, _) = collect(search(
            &profile,
            known,
            SearchParams {
                root,
                query: "B.J".into(),
                content: false,
                excludes: Vec::new(),
                case: false,
                word: false,
                regex: false,
            },
            || {},
        ));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, "/search/sub/b.js");
        note
    }

    #[test]
    #[ignore]
    fn live_search_sftp_uses_server_commands() {
        assert!(live_search(Protocol::Sftp, 2222).contains("командами сервера"));
    }

    #[test]
    #[ignore]
    fn live_search_ftp_walks_directories() {
        assert!(live_search(Protocol::Ftp, 2121).contains("обход каталогов"));
    }
}

#[cfg(test)]
mod live_keys {
    use super::*;

    fn shell_with(key: &str, passphrase: &str) -> Result<String, String> {
        let profile = TermProfile {
            name: "keys".into(),
            host: "127.0.0.1".into(),
            port: 2222,
            user: "jozef".into(),
            key_file: format!("/tmp/keys/{key}"),
            passphrase: passphrase.into(),
            startup: "echo key-$((40+2))".into(),
            ..Default::default()
        };
        let shell = open_shell(&profile.ssh(), None, (80, 24), profile.startup.clone(), || {});
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let mut output = Vec::new();
        while std::time::Instant::now() < deadline {
            match shell.rx.recv_timeout(Duration::from_millis(200)) {
                Ok(ShellEvent::HostKey { .. }) => shell.tx.send(ShellCmd::TrustHost(true)).unwrap(),
                Ok(ShellEvent::Output(bytes)) => output.extend(bytes),
                Ok(ShellEvent::Closed(e)) => return Err(e),
                _ => {}
            }
            let text = String::from_utf8_lossy(&output).into_owned();
            if text.contains("key-42") {
                return Ok(key_kind(&profile.key_file).unwrap_or_default());
            }
        }
        Err("timeout".into())
    }

    #[test]
    #[ignore]
    fn live_ssh_key_types() {
        assert_eq!(shell_with("k_ed25519", "").unwrap(), "Ed25519");
        assert_eq!(shell_with("k_ecdsa", "").unwrap(), "ECDSA nistp256");
        assert_eq!(shell_with("k_rsa", "").unwrap(), "RSA");
        assert_eq!(shell_with("k_rsa_pem", "").unwrap(), "RSA");
        assert_eq!(
            shell_with("k_ed25519_pass", "фраза-pass").unwrap(),
            "Ed25519, защищён парольной фразой"
        );
        assert_eq!(
            shell_with("k_ecdsa_pass", "pp").unwrap(),
            "ECDSA nistp384, защищён парольной фразой"
        );
        let wrong = shell_with("k_ed25519_pass", "wrong").unwrap_err();
        assert!(wrong.contains("вход не выполнен"), "{wrong}");
        let dsa = shell_with("k_dsa_pem", "").unwrap_err();
        assert!(dsa.contains("DSA"), "{dsa}");
    }

    #[test]
    #[ignore]
    fn live_ppk_keys() {
        assert_eq!(shell_with("p_ed25519.ppk", "").unwrap(), "PuTTY .ppk, Ed25519");
        assert_eq!(
            shell_with("p_rsa_v3.ppk", "pp3").unwrap(),
            "PuTTY .ppk, RSA, защищён парольной фразой"
        );
        assert_eq!(
            shell_with("p_ecdsa_v2.ppk", "pp3").unwrap(),
            "PuTTY .ppk, ECDSA nistp384, защищён парольной фразой"
        );
        let wrong = shell_with("p_rsa_v3.ppk", "nope").unwrap_err();
        assert!(wrong.contains("неверная парольная фраза"), "{wrong}");
    }
}
