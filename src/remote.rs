use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use serde::{Deserialize, Serialize};

const MASK: &[u8] = b"tror-profile-mask";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(12);
const IO_TIMEOUT_MS: u32 = 30_000;
const KEEPALIVE: Duration = Duration::from_secs(25);

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
pub enum Protocol {
    #[default]
    Sftp,
    Ftp,
}

impl Protocol {
    pub fn label(self) -> &'static str {
        match self {
            Protocol::Sftp => "SFTP",
            Protocol::Ftp => "FTP",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Protocol::Sftp => 22,
            Protocol::Ftp => 21,
        }
    }
}

mod masked {
    use super::MASK;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn hide(plain: &str) -> String {
        plain
            .bytes()
            .enumerate()
            .map(|(i, b)| format!("{:02x}", b ^ MASK[i % MASK.len()]))
            .collect()
    }

    pub fn reveal(hidden: &str) -> Option<String> {
        if !hidden.len().is_multiple_of(2) || !hidden.is_ascii() {
            return None;
        }
        let bytes: Option<Vec<u8>> = (0..hidden.len() / 2)
            .map(|i| {
                u8::from_str_radix(&hidden[i * 2..i * 2 + 2], 16)
                    .ok()
                    .map(|b| b ^ MASK[i % MASK.len()])
            })
            .collect();
        String::from_utf8(bytes?).ok()
    }

    pub fn serialize<S: Serializer>(plain: &str, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hide(plain))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
        let hidden = String::deserialize(d)?;
        Ok(reveal(&hidden).unwrap_or_default())
    }
}

#[derive(Clone, Serialize, Deserialize, Default, PartialEq, Debug)]
#[serde(default)]
pub struct Profile {
    pub name: String,
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub user: String,
    #[serde(with = "masked")]
    pub password: String,
    pub key_file: String,
    pub directory: String,
}

impl Profile {
    pub fn new(name: String) -> Self {
        Self {
            name,
            port: 22,
            ..Default::default()
        }
    }

    fn address(&self) -> String {
        format!("{}:{}", self.host.trim(), self.port)
    }
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Store {
    pub profiles: Vec<Profile>,
    pub known_hosts: BTreeMap<String, String>,
    pub hide_size: bool,
    pub show_date: bool,
    pub search_excludes: Option<String>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl Store {
    pub fn load(path: Option<PathBuf>) -> Self {
        let mut store: Store = path
            .as_deref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        store.path = path;
        store
    }

    pub fn save(&self) -> Result<(), String> {
        let path = self.path.as_deref().ok_or("нет каталога настроек")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| e.to_string())
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: Option<u64>,
}

pub enum Cmd {
    TrustHost(bool),
    List(String),
    Download(String),
    Upload {
        path: String,
        data: Vec<u8>,
        doc: Option<(u64, u64)>,
    },
    Mkdir(String),
    Rename(String, String),
    Delete {
        path: String,
        dir: bool,
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
    Changed(String),
    Log(String, bool),
    Error(String),
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

enum Backend {
    Sftp {
        _session: ssh2::Session,
        sftp: ssh2::Sftp,
    },
    Ftp(Box<suppaftp::FtpStream>),
}

fn tcp(profile: &Profile) -> Result<TcpStream, String> {
    let address = profile
        .address()
        .to_socket_addrs()
        .map_err(|e| format!("не удалось найти хост: {e}"))?
        .next()
        .ok_or("не удалось найти хост")?;
    TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)
        .map_err(|e| format!("нет соединения: {e}"))
}

fn fingerprint(session: &ssh2::Session) -> Option<String> {
    let hash = session.host_key_hash(ssh2::HashType::Sha256)?;
    Some(hash.iter().map(|b| format!("{b:02x}")).collect())
}

fn open_session(
    profile: &Profile,
    known: Option<&str>,
    confirm: &dyn Fn(&str, bool) -> bool,
) -> Result<(ssh2::Session, Option<String>), String> {
    let stream = tcp(profile)?;
    let mut session = ssh2::Session::new().map_err(|e| e.to_string())?;
    session.set_tcp_stream(stream);
    session.set_timeout(IO_TIMEOUT_MS);
    session
        .handshake()
        .map_err(|e| format!("ошибка SSH: {e}"))?;
    let print = fingerprint(&session).ok_or("сервер не прислал ключ")?;
    let mut accepted = None;
    if known != Some(print.as_str()) {
        if !confirm(&print, known.is_some()) {
            return Err("ключ сервера не подтверждён".into());
        }
        accepted = Some(print);
    }
    let user = profile.user.trim();
    let key = profile.key_file.trim();
    let auth = if !key.is_empty() {
        let phrase = (!profile.password.is_empty()).then_some(profile.password.as_str());
        session.userauth_pubkey_file(user, None, Path::new(key), phrase)
    } else if !profile.password.is_empty() {
        session.userauth_password(user, &profile.password)
    } else {
        session.userauth_agent(user)
    };
    auth.map_err(|e| format!("вход не выполнен: {e}"))?;
    Ok((session, accepted))
}

fn connect_sftp(
    profile: &Profile,
    known: Option<&str>,
    events: &Sender<Event>,
    cmds: &Receiver<Cmd>,
    wake: &dyn Fn(),
) -> Result<(Backend, String, Option<String>), String> {
    let confirm = |fingerprint: &str, changed: bool| {
        let _ = events.send(Event::HostKey {
            fingerprint: fingerprint.to_owned(),
            changed,
        });
        wake();
        matches!(cmds.recv(), Ok(Cmd::TrustHost(true)))
    };
    let (session, accepted) = open_session(profile, known, &confirm)?;
    let sftp = session
        .sftp()
        .map_err(|e| format!("SFTP недоступен: {e}"))?;
    let home = sftp
        .realpath(Path::new("."))
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| "/".into());
    Ok((
        Backend::Sftp {
            _session: session,
            sftp,
        },
        home,
        accepted,
    ))
}

pub enum ShellCmd {
    TrustHost(bool),
    Input(Vec<u8>),
    Resize(u16, u16),
}

pub enum ShellEvent {
    HostKey { fingerprint: String, changed: bool },
    Connected { fingerprint: Option<String> },
    Output(Vec<u8>),
    Closed(String),
}

pub struct Shell {
    pub host: String,
    pub tx: Sender<ShellCmd>,
    pub rx: Receiver<ShellEvent>,
}

fn run_shell(
    profile: &Profile,
    known: Option<&str>,
    size: (u16, u16),
    cmds: &Receiver<ShellCmd>,
    events: &Sender<ShellEvent>,
    wake: &dyn Fn(),
) -> Result<String, String> {
    let confirm = |fingerprint: &str, changed: bool| {
        let _ = events.send(ShellEvent::HostKey {
            fingerprint: fingerprint.to_owned(),
            changed,
        });
        wake();
        matches!(cmds.recv(), Ok(ShellCmd::TrustHost(true)))
    };
    let (session, fingerprint) = open_session(profile, known, &confirm)?;
    let mut channel = session
        .channel_session()
        .map_err(|e| format!("не удалось открыть канал: {e}"))?;
    channel
        .request_pty(
            "xterm-256color",
            None,
            Some((size.0 as u32, size.1 as u32, 0, 0)),
        )
        .map_err(|e| format!("сервер не дал терминал: {e}"))?;
    channel
        .shell()
        .map_err(|e| format!("сервер не запустил оболочку: {e}"))?;
    let _ = events.send(ShellEvent::Connected { fingerprint });
    wake();
    session.set_blocking(false);

    let mut pending: Vec<u8> = Vec::new();
    let mut resize = None;
    let mut buf = [0u8; 16 * 1024];
    loop {
        let mut active = false;
        match channel.read(&mut buf) {
            Ok(0) => {}
            Ok(n) => {
                let _ = events.send(ShellEvent::Output(buf[..n].to_vec()));
                wake();
                active = true;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(format!("соединение прервано: {e}")),
        }
        if channel.eof() {
            return Ok("сеанс завершён".into());
        }
        if !pending.is_empty() {
            match channel.write(&pending) {
                Ok(n) => {
                    pending.drain(..n);
                    active = true;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(format!("соединение прервано: {e}")),
            }
        }
        if let Some((cols, rows)) = resize
            && channel
                .request_pty_size(cols as u32, rows as u32, None, None)
                .is_ok()
        {
            resize = None;
        }
        let mut handle = |cmd: ShellCmd| match cmd {
            ShellCmd::Input(bytes) => pending.extend(bytes),
            ShellCmd::Resize(cols, rows) => resize = Some((cols, rows)),
            ShellCmd::TrustHost(_) => {}
        };
        loop {
            match cmds.try_recv() {
                Ok(cmd) => {
                    handle(cmd);
                    active = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return Ok("отключено".into());
                }
            }
        }
        if !active {
            match cmds.recv_timeout(Duration::from_millis(10)) {
                Ok(cmd) => handle(cmd),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Ok("отключено".into());
                }
            }
        }
    }
}

pub fn open_shell(
    profile: &Profile,
    known: Option<String>,
    size: (u16, u16),
    wake: impl Fn() + Send + 'static,
) -> Shell {
    let (tx, cmds) = channel();
    let (events, rx) = channel();
    let host = format!("{}@{}", profile.user.trim(), profile.host.trim());
    let profile = profile.clone();
    std::thread::spawn(move || {
        let message = run_shell(&profile, known.as_deref(), size, &cmds, &events, &wake)
            .unwrap_or_else(|e| e);
        let _ = events.send(ShellEvent::Closed(message));
        wake();
    });
    Shell { host, tx, rx }
}

fn connect_ftp(profile: &Profile) -> Result<(Backend, String), String> {
    let stream = tcp(profile)?;
    stream
        .set_read_timeout(Some(Duration::from_millis(IO_TIMEOUT_MS as u64)))
        .ok();
    let mut ftp =
        suppaftp::FtpStream::connect_with_stream(stream).map_err(|e| format!("ошибка FTP: {e}"))?;
    let user = if profile.user.trim().is_empty() {
        "anonymous"
    } else {
        profile.user.trim()
    };
    ftp.login(user, profile.password.as_str())
        .map_err(|e| format!("вход не выполнен: {e}"))?;
    ftp.transfer_type(suppaftp::types::FileType::Binary)
        .map_err(|e| e.to_string())?;
    let home = ftp.pwd().unwrap_or_else(|_| "/".into());
    Ok((Backend::Ftp(Box::new(ftp)), home))
}

impl Backend {
    fn list(&mut self, path: &str) -> Result<Vec<Entry>, String> {
        let mut entries = match self {
            Backend::Sftp { sftp, .. } => sftp
                .readdir(Path::new(path))
                .map_err(|e| e.to_string())?
                .into_iter()
                .filter_map(|(p, stat)| {
                    let name = p.file_name()?.to_string_lossy().into_owned();
                    let is_dir = stat.is_dir()
                        || (stat.file_type().is_symlink()
                            && sftp.stat(&p).map(|s| s.is_dir()).unwrap_or(false));
                    Some(Entry {
                        name,
                        is_dir,
                        size: stat.size.unwrap_or(0),
                        mtime: stat.mtime,
                    })
                })
                .collect::<Vec<_>>(),
            Backend::Ftp(ftp) => ftp
                .list(Some(path))
                .map_err(|e| e.to_string())?
                .iter()
                .filter_map(|line| line.parse::<suppaftp::list::File>().ok())
                .map(|f| Entry {
                    name: f.name().to_owned(),
                    is_dir: f.is_directory(),
                    size: f.size() as u64,
                    mtime: f
                        .modified()
                        .duration_since(std::time::UNIX_EPOCH)
                        .ok()
                        .map(|d| d.as_secs())
                        .filter(|secs| *secs > 0),
                })
                .collect(),
        };
        entries.retain(|e| e.name != "." && e.name != "..");
        sort(&mut entries);
        Ok(entries)
    }

    fn download(&mut self, path: &str) -> Result<Vec<u8>, String> {
        match self {
            Backend::Sftp { sftp, .. } => {
                let mut file = sftp.open(Path::new(path)).map_err(|e| e.to_string())?;
                let mut data = Vec::new();
                file.read_to_end(&mut data).map_err(|e| e.to_string())?;
                Ok(data)
            }
            Backend::Ftp(ftp) => ftp
                .retr_as_buffer(path)
                .map(Cursor::into_inner)
                .map_err(|e| e.to_string()),
        }
    }

    fn upload(&mut self, path: &str, data: &[u8]) -> Result<(), String> {
        match self {
            Backend::Sftp { sftp, .. } => {
                let mut file = sftp.create(Path::new(path)).map_err(|e| e.to_string())?;
                file.write_all(data).map_err(|e| e.to_string())
            }
            Backend::Ftp(ftp) => ftp
                .put_file(path, &mut Cursor::new(data))
                .map(|_| ())
                .map_err(|e| e.to_string()),
        }
    }

    fn mkdir(&mut self, path: &str) -> Result<(), String> {
        match self {
            Backend::Sftp { sftp, .. } => sftp
                .mkdir(Path::new(path), 0o755)
                .map_err(|e| e.to_string()),
            Backend::Ftp(ftp) => ftp.mkdir(path).map_err(|e| e.to_string()),
        }
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        match self {
            Backend::Sftp { sftp, .. } => sftp
                .rename(Path::new(from), Path::new(to), None)
                .map_err(|e| e.to_string()),
            Backend::Ftp(ftp) => ftp.rename(from, to).map_err(|e| e.to_string()),
        }
    }

    fn delete(&mut self, path: &str, dir: bool) -> Result<(), String> {
        match (self, dir) {
            (Backend::Sftp { sftp, .. }, true) => {
                sftp.rmdir(Path::new(path)).map_err(|e| e.to_string())
            }
            (Backend::Sftp { sftp, .. }, false) => {
                sftp.unlink(Path::new(path)).map_err(|e| e.to_string())
            }
            (Backend::Ftp(ftp), true) => ftp.rmdir(path).map_err(|e| e.to_string()),
            (Backend::Ftp(ftp), false) => ftp.rm(path).map_err(|e| e.to_string()),
        }
    }

    fn delete_tree(&mut self, path: &str, dir: bool, removed: &mut usize) -> Result<(), String> {
        if dir {
            for entry in self.list(path)? {
                self.delete_tree(&join(path, &entry.name), entry.is_dir, removed)?;
            }
        }
        self.delete(path, dir)?;
        *removed += 1;
        Ok(())
    }

    fn ping(&mut self) -> Result<(), String> {
        match self {
            Backend::Sftp { sftp, .. } => sftp
                .realpath(Path::new("."))
                .map(|_| ())
                .map_err(|e| e.to_string()),
            Backend::Ftp(ftp) => ftp.noop().map_err(|e| e.to_string()),
        }
    }
}

fn describe(cmd: &Cmd) -> String {
    match cmd {
        Cmd::TrustHost(_) => String::new(),
        Cmd::List(path) => format!("Чтение каталога {path}"),
        Cmd::Download(path) => format!("Скачивание {path}"),
        Cmd::Upload { path, data, .. } => format!("Загрузка {path} ({} байт)", data.len()),
        Cmd::Mkdir(path) => format!("Создание папки {path}"),
        Cmd::Rename(from, to) => format!("Перемещение {from} → {to}"),
        Cmd::Delete { path, .. } => format!("Удаление {path}"),
    }
}

fn execute(backend: &mut Backend, cmd: &Cmd) -> Result<(Event, String), String> {
    Ok(match cmd {
        Cmd::TrustHost(_) => (Event::Log(String::new(), false), String::new()),
        Cmd::List(path) => {
            let entries = backend.list(path)?;
            let summary = format!("объектов: {}", entries.len());
            (
                Event::Listing {
                    path: path.clone(),
                    entries,
                },
                summary,
            )
        }
        Cmd::Download(path) => {
            let data = backend.download(path)?;
            let summary = format!("получено байт: {}", data.len());
            (
                Event::Downloaded {
                    path: path.clone(),
                    data,
                },
                summary,
            )
        }
        Cmd::Upload { path, data, doc } => {
            backend.upload(path, data)?;
            (
                Event::Uploaded {
                    path: path.clone(),
                    doc: *doc,
                },
                "файл записан".into(),
            )
        }
        Cmd::Mkdir(path) => {
            backend.mkdir(path)?;
            (Event::Changed(parent(path)), "готово".into())
        }
        Cmd::Rename(from, to) => {
            backend.rename(from, to)?;
            (Event::Changed(parent(to)), "готово".into())
        }
        Cmd::Delete { path, dir } => {
            let mut removed = 0;
            backend.delete_tree(path, *dir, &mut removed)?;
            (
                Event::Changed(parent(path)),
                format!("удалено объектов: {removed}"),
            )
        }
    })
}

fn serve(
    profile: Profile,
    known: Option<String>,
    cmds: Receiver<Cmd>,
    events: Sender<Event>,
    wake: &dyn Fn(),
) {
    let send = |event: Event| {
        let _ = events.send(event);
        wake();
    };
    let log = |text: String, error: bool| send(Event::Log(text, error));
    let mut known = known;
    let open = |known: Option<&str>| match profile.protocol {
        Protocol::Sftp => connect_sftp(&profile, known, &events, &cmds, wake),
        Protocol::Ftp => connect_ftp(&profile).map(|(b, home)| (b, home, None)),
    };
    log(
        format!(
            "Подключение к {} ({}, пользователь {})",
            profile.address(),
            profile.protocol.label(),
            profile.user.trim()
        ),
        false,
    );
    let (mut backend, home, fingerprint) = match open(known.as_deref()) {
        Ok(ok) => ok,
        Err(e) => {
            log(format!("Не удалось подключиться: {e}"), true);
            send(Event::Closed(e));
            return;
        }
    };
    if fingerprint.is_some() {
        known = fingerprint.clone();
    }
    log(format!("Подключено, домашняя папка {home}"), false);
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
        let cmd = match cmds.recv_timeout(KEEPALIVE) {
            Ok(cmd) => cmd,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if alive && backend.ping().is_err() {
                    alive = false;
                    log("Сервер закрыл простаивающее соединение".into(), false);
                }
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if matches!(cmd, Cmd::TrustHost(_)) {
            continue;
        }
        log(describe(&cmd), false);
        let mut result = if alive {
            execute(&mut backend, &cmd)
        } else {
            Err("соединение закрыто".into())
        };
        if result.is_err() && (!alive || backend.ping().is_err()) {
            log("Соединение потеряно, подключаюсь заново".into(), false);
            match open(known.as_deref()) {
                Ok((fresh, _, fingerprint)) => {
                    backend = fresh;
                    alive = true;
                    if fingerprint.is_some() {
                        known = fingerprint;
                    }
                    log("Подключено заново, повторяю операцию".into(), false);
                    result = execute(&mut backend, &cmd);
                }
                Err(e) => {
                    log(format!("Не удалось подключиться заново: {e}"), true);
                    send(Event::Closed(e));
                    return;
                }
            }
        }
        match result {
            Ok((event, summary)) => {
                log(summary, false);
                send(event);
            }
            Err(e) => {
                log(format!("Ошибка: {e}"), true);
                send(Event::Error(e));
            }
        }
    }
    log("Отключено".into(), false);
}

pub struct SearchParams {
    pub root: String,
    pub query: String,
    pub content: bool,
    pub excludes: Vec<String>,
}

pub enum SearchEvent {
    Hit {
        path: String,
        line: Option<usize>,
        text: String,
    },
    Status(String),
    Done(String),
    Failed(String),
}

pub struct SearchJob {
    pub rx: Receiver<SearchEvent>,
    pub cancel: Arc<AtomicBool>,
}

const MAX_HITS: usize = 2000;
const MAX_GREP_FILE: u64 = 2 * 1024 * 1024;
const BINARY: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "ico", "bmp", "pdf", "zip", "gz", "tar", "rar", "7z",
    "woff", "woff2", "ttf", "eot", "otf", "mp3", "mp4", "avi", "mov", "exe", "dll", "so", "phar",
    "doc", "docx", "xls", "xlsx",
];

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

pub fn search_command(params: &SearchParams) -> String {
    let prunes: Vec<String> = params
        .excludes
        .iter()
        .map(|e| e.trim().trim_matches('/'))
        .filter(|e| !e.is_empty())
        .map(|e| {
            if e.contains('/') {
                format!("-path {}", quote(&format!("*/{e}")))
            } else {
                format!("-name {}", quote(e))
            }
        })
        .collect();
    let prune = if prunes.is_empty() {
        String::new()
    } else {
        format!("\\( {} \\) -prune -o ", prunes.join(" -o "))
    };
    let root = quote(&params.root);
    let check = "command -v find >/dev/null 2>&1 && command -v grep >/dev/null 2>&1 || exit 97; ";
    if params.content {
        format!(
            "{check}find -H {root} {prune}-type f -size -{}k -print0 2>/dev/null | xargs -0 grep -InH -i -F -e {} -- 2>/dev/null | head -n {MAX_HITS}",
            MAX_GREP_FILE / 1024,
            quote(&params.query)
        )
    } else {
        let mut glob = String::new();
        for c in params.query.chars() {
            if matches!(c, '*' | '?' | '[' | ']' | '\\') {
                glob.push('\\');
            }
            glob.push(c);
        }
        format!(
            "{check}find -H {root} {prune}-iname {} -print 2>/dev/null | head -n {MAX_HITS}",
            quote(&format!("*{glob}*"))
        )
    }
}

pub fn parse_grep_line(line: &str) -> Option<(String, usize, String)> {
    let mut from = 0;
    while let Some(i) = line[from..].find(':') {
        let colon = from + i;
        let rest = &line[colon + 1..];
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 && rest[digits..].starts_with(':') {
            let number = rest[..digits].parse().ok()?;
            return Some((
                line[..colon].to_owned(),
                number,
                rest[digits + 1..].trim().chars().take(200).collect(),
            ));
        }
        from = colon + 1;
    }
    None
}

fn excluded(path: &str, name: &str, excludes: &[String]) -> bool {
    excludes.iter().any(|e| {
        let e = e.trim().trim_matches('/');
        !e.is_empty()
            && if e.contains('/') {
                path.ends_with(&format!("/{e}"))
            } else {
                name == e
            }
    })
}

fn search_remote_shell(
    session: &ssh2::Session,
    params: &SearchParams,
    cancel: &AtomicBool,
    emit: &dyn Fn(SearchEvent),
) -> Result<Option<usize>, String> {
    let Ok(mut channel) = session.channel_session() else {
        return Ok(None);
    };
    if channel.exec(&search_command(params)).is_err() {
        return Ok(None);
    }
    session.set_blocking(false);
    let mut pending: Vec<u8> = Vec::new();
    let mut hits = 0;
    let mut buf = [0u8; 16 * 1024];
    let handle = |line: &str, hits: &mut usize| {
        if line.is_empty() {
            return;
        }
        if params.content {
            if let Some((path, number, text)) = parse_grep_line(line) {
                emit(SearchEvent::Hit {
                    path,
                    line: Some(number),
                    text,
                });
                *hits += 1;
            }
        } else {
            emit(SearchEvent::Hit {
                path: line.to_owned(),
                line: None,
                text: String::new(),
            });
            *hits += 1;
        }
    };
    loop {
        if cancel.load(Ordering::Relaxed) {
            session.set_blocking(true);
            let _ = channel.close();
            return Ok(Some(hits));
        }
        match channel.read(&mut buf) {
            Ok(0) if channel.eof() => break,
            Ok(0) => std::thread::sleep(Duration::from_millis(15)),
            Ok(n) => {
                pending.extend_from_slice(&buf[..n]);
                while let Some(end) = pending.iter().position(|b| *b == b'\n') {
                    let line: Vec<u8> = pending.drain(..=end).collect();
                    handle(String::from_utf8_lossy(&line).trim_end(), &mut hits);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if channel.eof() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(15));
            }
            Err(e) => {
                session.set_blocking(true);
                return Err(e.to_string());
            }
        }
    }
    handle(String::from_utf8_lossy(&pending).trim_end(), &mut hits);
    session.set_blocking(true);
    let _ = channel.wait_close();
    let status = channel.exit_status().unwrap_or(0);
    if hits == 0 && status != 0 {
        return Ok(None);
    }
    Ok(Some(hits))
}

fn search_walk(
    backend: &mut Backend,
    params: &SearchParams,
    cancel: &AtomicBool,
    emit: &dyn Fn(SearchEvent),
) -> usize {
    let needle = params.query.to_lowercase();
    let mut stack = vec![params.root.clone()];
    let mut hits = 0;
    let mut dirs = 0;
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) || hits >= MAX_HITS {
            break;
        }
        dirs += 1;
        emit(SearchEvent::Status(format!(
            "просмотрено каталогов: {dirs}"
        )));
        let Ok(entries) = backend.list(&dir) else {
            continue;
        };
        for entry in entries {
            if cancel.load(Ordering::Relaxed) || hits >= MAX_HITS {
                break;
            }
            let path = join(&dir, &entry.name);
            if excluded(&path, &entry.name, &params.excludes) {
                continue;
            }
            if entry.is_dir {
                stack.push(path.clone());
            }
            if !params.content {
                if entry.name.to_lowercase().contains(&needle) {
                    emit(SearchEvent::Hit {
                        path,
                        line: None,
                        text: String::new(),
                    });
                    hits += 1;
                }
                continue;
            }
            let ext = entry.name.rsplit('.').next().unwrap_or("").to_lowercase();
            if entry.is_dir || entry.size > MAX_GREP_FILE || BINARY.contains(&ext.as_str()) {
                continue;
            }
            let Ok(data) = backend.download(&path) else {
                continue;
            };
            if data.contains(&0) {
                continue;
            }
            let text = String::from_utf8_lossy(&data);
            for (number, line) in text.lines().enumerate() {
                if line.to_lowercase().contains(&needle) {
                    emit(SearchEvent::Hit {
                        path: path.clone(),
                        line: Some(number + 1),
                        text: line.trim().chars().take(200).collect(),
                    });
                    hits += 1;
                    if hits >= MAX_HITS {
                        break;
                    }
                }
            }
        }
    }
    hits
}

pub fn search(
    profile: &Profile,
    known: Option<String>,
    params: SearchParams,
    wake: impl Fn() + Send + 'static,
) -> SearchJob {
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let profile = profile.clone();
    std::thread::spawn(move || {
        let emit = |event: SearchEvent| {
            let _ = tx.send(event);
            wake();
        };
        let run = || -> Result<(usize, &'static str), String> {
            match profile.protocol {
                Protocol::Sftp => {
                    let (session, _) = open_session(&profile, known.as_deref(), &|_, _| false)?;
                    if let Some(hits) = search_remote_shell(&session, &params, &flag, &emit)? {
                        return Ok((hits, "поиск командами сервера"));
                    }
                    let sftp = session.sftp().map_err(|e| e.to_string())?;
                    let mut backend = Backend::Sftp {
                        _session: session,
                        sftp,
                    };
                    Ok((
                        search_walk(&mut backend, &params, &flag, &emit),
                        "обход каталогов",
                    ))
                }
                Protocol::Ftp => {
                    let (mut backend, _) = connect_ftp(&profile)?;
                    Ok((
                        search_walk(&mut backend, &params, &flag, &emit),
                        "обход каталогов",
                    ))
                }
            }
        };
        match run() {
            Ok((hits, how)) => {
                let stopped = if flag.load(Ordering::Relaxed) {
                    ", остановлен"
                } else if hits >= MAX_HITS {
                    ", показаны первые результаты"
                } else {
                    ""
                };
                emit(SearchEvent::Done(format!(
                    "найдено: {hits} ({how}{stopped})"
                )));
            }
            Err(e) => emit(SearchEvent::Failed(e)),
        }
    });
    SearchJob { rx, cancel }
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
    std::thread::spawn(move || serve(profile, known, cmds, events, &wake));
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
            directory: "/www".into(),
        });
        store
            .known_hosts
            .insert("example.org:22".into(), "ab12".into());
        store.save().unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("s3cret"));
        assert!(raw.contains("example.org"));
        let loaded = Store::load(Some(path));
        assert_eq!(loaded.profiles, store.profiles);
        assert_eq!(loaded.known_hosts, store.known_hosts);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn entries_sorted_dirs_first() {
        let mut entries = vec![
            Entry {
                name: "b.php".into(),
                is_dir: false,
                size: 1,
                mtime: None,
            },
            Entry {
                name: "Zeta".into(),
                is_dir: true,
                size: 0,
                mtime: None,
            },
            Entry {
                name: "a.php".into(),
                is_dir: false,
                size: 1,
                mtime: None,
            },
            Entry {
                name: "alpha".into(),
                is_dir: true,
                size: 0,
                mtime: None,
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
            directory: String::new(),
        };
        let conn = connect(&profile, None, || {});
        let next = || loop {
            match conn
                .rx
                .recv_timeout(Duration::from_secs(20))
                .expect("ответ сервера")
            {
                Event::Log(..) => {}
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
        assert!(matches!(next(), Event::Error(_)));
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
    fn live_sftp_rejects_wrong_password_and_changed_key() {
        let mut profile = Profile::new("x".into());
        profile.host = "127.0.0.1".into();
        profile.port = 2222;
        profile.user = "jozef".into();
        profile.password = "wrong".into();
        let conn = connect(&profile, Some("00".repeat(32)), || {});
        let next = || loop {
            match conn.rx.recv_timeout(Duration::from_secs(20)) {
                Ok(Event::Log(..)) => {}
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
