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

const MASK: &[u8] = b"tror-profile-mask";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(12);
const IO_TIMEOUT_MS: u32 = 30_000;
const KEEPALIVE: Duration = Duration::from_secs(25);

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
pub enum Protocol {
    #[default]
    Sftp,
    Ftp,
    Ftps,
    FtpsImplicit,
}

impl Protocol {
    pub const ALL: [Protocol; 4] = [
        Protocol::Sftp,
        Protocol::Ftp,
        Protocol::Ftps,
        Protocol::FtpsImplicit,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Protocol::Sftp => "SFTP",
            Protocol::Ftp => "FTP",
            Protocol::Ftps => "FTPS",
            Protocol::FtpsImplicit => "FTPS неявный",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Protocol::Sftp => 22,
            Protocol::Ftp | Protocol::Ftps => 21,
            Protocol::FtpsImplicit => 990,
        }
    }

    pub fn tls(self) -> bool {
        matches!(self, Protocol::Ftps | Protocol::FtpsImplicit)
    }
}

pub mod masked {
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
    #[serde(with = "masked")]
    pub passphrase: String,
    pub directory: String,
    pub url: String,
    pub insecure_tls: bool,
    pub term_theme: String,
    pub term_font: String,
    pub term_font_size: f32,
}

impl Profile {
    pub fn new(name: String) -> Self {
        Self {
            name,
            port: 22,
            ..Default::default()
        }
    }

    pub fn address(&self) -> String {
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
    pub databases: Vec<crate::db::DbProfile>,
    pub terminals: Vec<TermProfile>,
    pub show_perm: bool,
    #[serde(skip)]
    path: Option<PathBuf>,
}

static STORE_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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
        std::fs::write(path, text).map_err(|e| e.to_string())?;
        STORE_GENERATION.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    pub fn generation() -> u64 {
        STORE_GENERATION.load(Ordering::Relaxed)
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
pub enum TermProtocol {
    #[default]
    Ssh,
    Telnet,
    Mosh,
}

impl TermProtocol {
    pub const ALL: [TermProtocol; 3] = [TermProtocol::Ssh, TermProtocol::Telnet, TermProtocol::Mosh];

    pub fn label(self) -> &'static str {
        match self {
            TermProtocol::Ssh => "SSH",
            TermProtocol::Telnet => "Telnet",
            TermProtocol::Mosh => "Mosh",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            TermProtocol::Ssh | TermProtocol::Mosh => 22,
            TermProtocol::Telnet => 23,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
#[serde(default)]
pub struct TermProfile {
    pub name: String,
    pub protocol: TermProtocol,
    pub host: String,
    pub port: u16,
    pub user: String,
    #[serde(with = "masked")]
    pub password: String,
    pub key_file: String,
    #[serde(with = "masked")]
    pub passphrase: String,
    pub theme: String,
    pub font: String,
    pub font_size: f32,
    pub mosh_server: String,
    pub mosh_ports: String,
    pub startup: String,
    #[serde(skip)]
    pub origin: Option<String>,
}

impl Default for TermProfile {
    fn default() -> Self {
        Self {
            name: String::new(),
            protocol: TermProtocol::Ssh,
            host: String::new(),
            port: 22,
            user: String::new(),
            password: String::new(),
            key_file: String::new(),
            passphrase: String::new(),
            theme: String::new(),
            font: String::new(),
            font_size: 0.0,
            mosh_server: String::new(),
            mosh_ports: String::new(),
            startup: String::new(),
            origin: None,
        }
    }
}

impl TermProfile {
    pub fn ssh(&self) -> Profile {
        Profile {
            name: self.name.clone(),
            protocol: Protocol::Sftp,
            host: self.host.clone(),
            port: self.port,
            user: self.user.clone(),
            password: self.password.clone(),
            key_file: self.key_file.clone(),
            passphrase: self.passphrase.clone(),
            ..Default::default()
        }
    }

    pub fn from_host(profile: &Profile) -> Self {
        Self {
            name: profile.name.clone(),
            protocol: TermProtocol::Ssh,
            host: profile.host.clone(),
            port: if profile.protocol == Protocol::Sftp {
                profile.port
            } else {
                22
            },
            user: profile.user.clone(),
            password: profile.password.clone(),
            key_file: profile.key_file.clone(),
            passphrase: profile.passphrase.clone(),
            theme: profile.term_theme.clone(),
            font: profile.term_font.clone(),
            font_size: profile.term_font_size,
            origin: Some(profile.name.clone()),
            ..Default::default()
        }
    }

    pub fn address(&self) -> String {
        format!("{}:{}", self.host.trim(), self.port)
    }
}

const DSA_NOTE: &str = "ключ DSA: этот тип устарел, в OpenSSH 10 удалён, и библиотека SSH программы входить по нему не умеет — создайте ключ Ed25519 (ssh-keygen -t ed25519)";

pub fn key_kind(path: &str) -> Result<String, String> {
    use base64::Engine;
    let text = std::fs::read_to_string(path.trim()).map_err(|e| e.to_string())?;
    if text.starts_with("PuTTY-User-Key-File") {
        return Err("ключ в формате PuTTY (.ppk) — экспортируйте его в PuTTYgen: Conversions → Export OpenSSH key".into());
    }
    let header = text
        .lines()
        .find(|l| l.starts_with("-----BEGIN "))
        .ok_or("это не закрытый ключ OpenSSH или PEM")?;
    let encrypted = text.contains("ENCRYPTED");
    let lock = |name: &str| {
        if encrypted {
            format!("{name}, защищён парольной фразой")
        } else {
            name.to_owned()
        }
    };
    Ok(match header {
        h if h.contains("RSA PRIVATE KEY") => lock("RSA"),
        h if h.contains("DSA PRIVATE KEY") => return Err(DSA_NOTE.into()),
        h if h.contains("EC PRIVATE KEY") => lock("ECDSA"),
        h if h.contains("OPENSSH PRIVATE KEY") => {
            let body: String = text
                .lines()
                .skip_while(|l| !l.starts_with("-----BEGIN"))
                .skip(1)
                .take_while(|l| !l.starts_with("-----END"))
                .collect();
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(body.trim())
                .map_err(|e| e.to_string())?;
            let magic = b"openssh-key-v1\0";
            if !bytes.starts_with(magic) {
                return Err("повреждённый ключ OpenSSH".into());
            }
            let mut at = magic.len();
            fn field(bytes: &[u8], at: &mut usize) -> Option<Vec<u8>> {
                let len = u32::from_be_bytes(bytes.get(*at..*at + 4)?.try_into().ok()?) as usize;
                let value = bytes.get(*at + 4..*at + 4 + len)?.to_vec();
                *at += 4 + len;
                Some(value)
            }
            let cipher = field(&bytes, &mut at).ok_or("повреждённый ключ OpenSSH")?;
            field(&bytes, &mut at);
            field(&bytes, &mut at);
            at += 4;
            let public = field(&bytes, &mut at).ok_or("повреждённый ключ OpenSSH")?;
            let len = public
                .get(..4)
                .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
                .unwrap_or(0);
            let kind = String::from_utf8_lossy(public.get(4..4 + len).unwrap_or_default());
            let name = match kind.as_ref() {
                "ssh-ed25519" => "Ed25519".to_owned(),
                "ssh-rsa" => "RSA".to_owned(),
                "ssh-dss" => return Err(DSA_NOTE.into()),
                k if k.starts_with("ecdsa-sha2-") => format!("ECDSA {}", &k[11..]),
                k if k.starts_with("sk-") => format!("{k} (ключ безопасности)"),
                k => k.to_owned(),
            };
            if cipher != b"none" {
                format!("{name}, защищён парольной фразой")
            } else {
                name
            }
        }
        h if h.contains("ENCRYPTED PRIVATE KEY") => "PKCS#8, защищён парольной фразой".into(),
        h if h.contains("PRIVATE KEY") => "PKCS#8".into(),
        _ => return Err("это не закрытый ключ".into()),
    })
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
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
    Progress {
        done: u64,
        total: u64,
    },
    Walked {
        root: String,
        files: Vec<(String, u64)>,
    },
    Stat {
        path: String,
        attrs: Attrs,
    },
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
    Ftp(Box<suppaftp::NativeTlsFtpStream>),
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

pub(crate) fn open_session(
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
    if let Some(banner) = session.banner() {
        trace::server(format!("[SSH] remote version: {}", banner.trim()));
    }
    let method = |kind| session.methods(kind).unwrap_or("-").to_owned();
    trace::server(
        format!(
            "[SSH] kex: {}; host key: {}; cipher: {} / {}; mac: {} / {}",
            method(ssh2::MethodType::Kex),
            method(ssh2::MethodType::HostKey),
            method(ssh2::MethodType::CryptCs),
            method(ssh2::MethodType::CryptSc),
            method(ssh2::MethodType::MacCs),
            method(ssh2::MethodType::MacSc),
        ),
    );
    let print = fingerprint(&session).ok_or("сервер не прислал ключ")?;
    let mut accepted = None;
    if known == Some(print.as_str()) {
        trace::server(format!("[SSH] host key SHA256:{print} (known)"));
    } else {
        let state = if known.is_some() { "CHANGED" } else { "new" };
        trace::line(
            format!("[SSH] host key SHA256:{print} ({state}), waiting for confirmation"),
            known.is_some(),
        );
        if !confirm(&print, known.is_some()) {
            trace::line("[SSH] host key rejected", true);
            return Err("ключ сервера не подтверждён".into());
        }
        trace::line("[SSH] host key accepted", false);
        accepted = Some(print);
    }
    let user = profile.user.trim();
    if let Ok(methods) = session.auth_methods(user) {
        trace::server(format!("[SSH] auth methods: {methods}"));
    }
    let key = profile.key_file.trim();
    let mut auth = if !key.is_empty() {
        trace::client(format!("[SSH] userauth publickey ({key}) as {user}"));
        if !Path::new(key).is_file() {
            Err(ssh2::Error::new(
                ssh2::ErrorCode::Session(-16),
                "файл ключа не найден",
            ))
        } else {
            let phrase = if !profile.passphrase.is_empty() {
                Some(profile.passphrase.as_str())
            } else {
                (!profile.password.is_empty()).then_some(profile.password.as_str())
            };
            let public = PathBuf::from(format!("{key}.pub"));
            let public = public.is_file().then_some(public.as_path());
            session.userauth_pubkey_file(user, public, Path::new(key), phrase)
        }
    } else if !profile.password.is_empty() {
        trace::client(format!("[SSH] userauth password as {user}"));
        session.userauth_password(user, &profile.password)
    } else {
        trace::client(format!("[SSH] userauth agent as {user}"));
        session.userauth_agent(user)
    };
    if auth.is_err()
        && !key.is_empty()
        && !profile.passphrase.is_empty()
        && !profile.password.is_empty()
        && !session.authenticated()
    {
        if let Err(e) = &auth {
            trace::line(format!("[SSH] {e}"), true);
        }
        trace::client(format!("[SSH] userauth password as {user}"));
        auth = session.userauth_password(user, &profile.password);
    }
    if let Err(e) = &auth {
        trace::line(format!("[SSH] {e}"), true);
        if !key.is_empty()
            && let Err(why) = key_kind(key)
        {
            trace::line(format!("[SSH] key file: {why}"), true);
            return Err(format!("вход не выполнен: {why}"));
        }
    }
    auth.map_err(|e| format!("вход не выполнен: {e}"))?;
    trace::server("[SSH] authenticated");
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
    Status(String),
    Closed(String),
}

pub struct Shell {
    pub tx: Sender<ShellCmd>,
    pub rx: Receiver<ShellEvent>,
}

fn run_shell(
    profile: &Profile,
    known: Option<&str>,
    size: (u16, u16),
    startup: &str,
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
    trace::client(format!(
        "[SSH] channel session: pty xterm-256color {}x{}, shell started",
        size.0, size.1
    ));
    let _ = events.send(ShellEvent::Connected { fingerprint });
    wake();
    session.set_blocking(false);

    let mut pending: Vec<u8> = Vec::new();
    let startup = startup.trim();
    if !startup.is_empty() {
        pending.extend_from_slice(startup.as_bytes());
        pending.push(b'\r');
    }
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
    startup: String,
    wake: impl Fn() + Send + 'static,
) -> Shell {
    let (tx, cmds) = channel();
    let (events, rx) = channel();
    let profile = profile.clone();
    std::thread::spawn(move || {
        let message = run_shell(&profile, known.as_deref(), size, &startup, &cmds, &events, &wake)
            .unwrap_or_else(|e| e);
        let _ = events.send(ShellEvent::Closed(message));
        wake();
    });
    Shell { tx, rx }
}

fn tls_connector(profile: &Profile) -> Result<suppaftp::NativeTlsConnector, String> {
    let connector = native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(profile.insecure_tls)
        .danger_accept_invalid_hostnames(profile.insecure_tls)
        .build()
        .map_err(|e| format!("TLS: {e}"))?;
    Ok(suppaftp::NativeTlsConnector::from(connector))
}

fn connect_ftp(profile: &Profile) -> Result<(Backend, String), String> {
    let mut ftp = if profile.protocol == Protocol::FtpsImplicit {
        let address = profile
            .address()
            .to_socket_addrs()
            .map_err(|e| format!("не удалось найти хост: {e}"))?
            .next()
            .ok_or("не удалось найти хост")?;
        trace::line(
            format!(
                "[FTPS] implicit TLS on {address}, certificate check {}",
                if profile.insecure_tls { "OFF" } else { "on" }
            ),
            profile.insecure_tls,
        );
        let ftp = suppaftp::NativeTlsFtpStream::connect_secure_implicit(
            address,
            tls_connector(profile)?,
            profile.host.trim(),
        )
        .map_err(|e| format!("защищённое соединение не установлено: {e}"))?;
        trace::server("[FTPS] TLS session established");
        ftp
    } else {
        let stream = tcp(profile)?;
        stream
            .set_read_timeout(Some(Duration::from_millis(IO_TIMEOUT_MS as u64)))
            .ok();
        let mut ftp = suppaftp::NativeTlsFtpStream::connect_with_stream(stream)
            .map_err(|e| format!("ошибка FTP: {e}"))?;
        if profile.protocol == Protocol::Ftps {
            trace::line(
                format!(
                    "[FTPS] AUTH TLS, certificate check {}",
                    if profile.insecure_tls { "OFF" } else { "on" }
                ),
                profile.insecure_tls,
            );
            ftp = ftp
                .into_secure(tls_connector(profile)?, profile.host.trim())
                .map_err(|e| format!("защищённое соединение не установлено: {e}"))?;
            trace::server("[FTPS] TLS session established");
        }
        ftp
    };

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

const CHUNK: usize = 64 * 1024;

struct Counting<'a> {
    data: &'a [u8],
    done: usize,
    progress: &'a dyn Fn(u64, u64),
}

impl Read for Counting<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = buf.len().min(self.data.len() - self.done);
        buf[..n].copy_from_slice(&self.data[self.done..self.done + n]);
        self.done += n;
        (self.progress)(self.done as u64, self.data.len() as u64);
        Ok(n)
    }
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
                        perm: stat.perm.map(|p| p & 0o7777),
                        owner: match (stat.uid, stat.gid) {
                            (Some(u), Some(g)) => format!("{u}:{g}"),
                            _ => String::new(),
                        },
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
                    perm: ftp_mode(&f),
                    owner: match (f.uid(), f.gid()) {
                        (Some(u), Some(g)) => format!("{u}:{g}"),
                        _ => String::new(),
                    },
                })
                .collect(),
        };
        entries.retain(|e| e.name != "." && e.name != "..");
        sort(&mut entries);
        Ok(entries)
    }

    fn download(&mut self, path: &str, progress: &dyn Fn(u64, u64)) -> Result<Vec<u8>, String> {
        let mut data = Vec::new();
        let mut buf = vec![0u8; CHUNK];
        match self {
            Backend::Sftp { sftp, .. } => {
                let mut file = sftp.open(Path::new(path)).map_err(|e| e.to_string())?;
                let total = file.stat().ok().and_then(|s| s.size).unwrap_or(0);
                loop {
                    let n = file.read(&mut buf).map_err(|e| e.to_string())?;
                    if n == 0 {
                        break;
                    }
                    data.extend_from_slice(&buf[..n]);
                    progress(data.len() as u64, total);
                }
            }
            Backend::Ftp(ftp) => {
                let total = ftp.size(path).unwrap_or(0) as u64;
                ftp.retr(path, |reader| {
                    loop {
                        let n = reader
                            .read(&mut buf)
                            .map_err(suppaftp::FtpError::ConnectionError)?;
                        if n == 0 {
                            break;
                        }
                        data.extend_from_slice(&buf[..n]);
                        progress(data.len() as u64, total);
                    }
                    Ok(())
                })
                .map_err(|e| e.to_string())?;
            }
        }
        Ok(data)
    }

    fn write(
        &mut self,
        path: &str,
        data: &[u8],
        progress: &dyn Fn(u64, u64),
    ) -> Result<(), String> {
        let total = data.len() as u64;
        match self {
            Backend::Sftp { sftp, .. } => {
                let mut file = sftp.create(Path::new(path)).map_err(|e| e.to_string())?;
                let mut done = 0;
                for chunk in data.chunks(CHUNK) {
                    file.write_all(chunk).map_err(|e| e.to_string())?;
                    done += chunk.len() as u64;
                    progress(done, total);
                }
                Ok(())
            }
            Backend::Ftp(ftp) => {
                let mut reader = Counting {
                    data,
                    done: 0,
                    progress,
                };
                ftp.put_file(path, &mut reader)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
        }
    }

    fn upload(
        &mut self,
        path: &str,
        data: &[u8],
        progress: &dyn Fn(u64, u64),
    ) -> Result<(), String> {
        let first = match self.write(path, data, progress) {
            Ok(()) => return Ok(()),
            Err(e) => e,
        };
        let dir = parent(path);
        if dir == "/" || self.list(&dir).is_ok() {
            return Err(first);
        }
        let mut current = String::new();
        for part in dir.split('/').filter(|p| !p.is_empty()) {
            current = format!("{current}/{part}");
            let _ = self.mkdir(&current);
        }
        self.write(path, data, progress)
    }

    fn walk(&mut self, root: &str, files: &mut Vec<(String, u64)>) -> Result<(), String> {
        for entry in self.list(root)? {
            let path = join(root, &entry.name);
            if entry.is_dir {
                self.walk(&path, files)?;
            } else {
                files.push((path, entry.size));
            }
        }
        Ok(())
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

    fn stat(&mut self, path: &str) -> Result<Attrs, String> {
        match self {
            Backend::Sftp { sftp, .. } => {
                let stat = sftp.stat(Path::new(path)).map_err(|e| e.to_string())?;
                Ok(Attrs {
                    perm: stat.perm.map(|p| p & 0o7777),
                    uid: stat.uid,
                    gid: stat.gid,
                    size: stat.size,
                    mtime: stat.mtime,
                    atime: stat.atime,
                    is_dir: stat.is_dir(),
                })
            }
            Backend::Ftp(_) => {
                let dir = parent(path);
                let name = file_name(path).to_owned();
                let entry = self
                    .list(&dir)?
                    .into_iter()
                    .find(|e| e.name == name)
                    .ok_or("объект не найден")?;
                let mut ids = entry.owner.split(':').map(|v| v.parse().ok());
                Ok(Attrs {
                    perm: entry.perm,
                    uid: ids.next().flatten(),
                    gid: ids.next().flatten(),
                    size: Some(entry.size),
                    mtime: entry.mtime,
                    atime: None,
                    is_dir: entry.is_dir,
                })
            }
        }
    }

    fn chmod_one(&mut self, path: &str, mode: u32) -> Result<(), String> {
        match self {
            Backend::Sftp { sftp, .. } => sftp
                .setstat(
                    Path::new(path),
                    ssh2::FileStat {
                        size: None,
                        uid: None,
                        gid: None,
                        perm: Some(mode & 0o7777),
                        atime: None,
                        mtime: None,
                    },
                )
                .map_err(|e| e.to_string()),
            Backend::Ftp(ftp) => ftp
                .site(format!("CHMOD {:o} {path}", mode & 0o7777))
                .map(|_| ())
                .map_err(|e| e.to_string()),
        }
    }

    fn chmod(
        &mut self,
        path: &str,
        mode: u32,
        recursive: bool,
        dirs: bool,
        files: bool,
        changed: &mut usize,
    ) -> Result<(), String> {
        let is_dir = self.stat(path).map(|a| a.is_dir).unwrap_or(false);
        if !recursive || (is_dir && dirs) || (!is_dir && files) {
            self.chmod_one(path, mode)?;
            *changed += 1;
        }
        if recursive && is_dir {
            for entry in self.list(path)? {
                let child = join(path, &entry.name);
                if entry.is_dir {
                    self.chmod(&child, mode, true, dirs, files, changed)?;
                } else if files {
                    self.chmod_one(&child, mode)?;
                    *changed += 1;
                }
            }
        }
        Ok(())
    }

    fn touch(&mut self, path: &str, mtime: u64) -> Result<(), String> {
        match self {
            Backend::Sftp { sftp, .. } => {
                let atime = sftp.stat(Path::new(path)).ok().and_then(|s| s.atime);
                sftp.setstat(
                    Path::new(path),
                    ssh2::FileStat {
                        size: None,
                        uid: None,
                        gid: None,
                        perm: None,
                        atime: Some(atime.unwrap_or(mtime)),
                        mtime: Some(mtime),
                    },
                )
                .map_err(|e| e.to_string())
            }
            Backend::Ftp(ftp) => {
                let stamp = chrono::DateTime::from_timestamp(mtime as i64, 0)
                    .map(|d| d.format("%Y%m%d%H%M%S").to_string())
                    .unwrap_or_default();
                ftp.custom_command(
                    format!("MFMT {stamp} {path}"),
                    &[suppaftp::Status::File],
                )
                .map(|_| ())
                .map_err(|e| e.to_string())
            }
        }
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
    }
}

fn ftp_mode(file: &suppaftp::list::File) -> Option<u32> {
    use suppaftp::list::PosixPexQuery as Who;
    let bits = |who: Who| {
        (file.can_read(who) as u32) << 2
            | (file.can_write(who) as u32) << 1
            | file.can_execute(who) as u32
    };
    Some(bits(Who::Owner) << 6 | bits(Who::Group) << 3 | bits(Who::Others))
}

fn execute(
    backend: &mut Backend,
    cmd: &Cmd,
    progress: &dyn Fn(u64, u64),
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
        Cmd::Upload { path, data, doc } => {
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
            backend.walk(root, &mut files)?;
            let summary = format!("{} files", files.len());
            (
                Event::Walked {
                    root: root.clone(),
                    files,
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
        Cmd::Touch { path, mtime } => {
            backend.touch(path, *mtime)?;
            (Event::Changed(parent(path)), "ok".into())
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
    let log = |text: String, error: bool| {
        trace::line(text, error);
        wake();
    };
    let last = std::cell::Cell::new(std::time::Instant::now());
    let progress = |done: u64, total: u64| {
        if done >= total || last.get().elapsed() > Duration::from_millis(60) {
            last.set(std::time::Instant::now());
            send(Event::Progress { done, total });
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
        Protocol::Sftp => connect_sftp(&profile, known, &events, &cmds, wake),
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
        let cmd = match cmds.recv_timeout(KEEPALIVE) {
            Ok(cmd) => cmd,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if alive && backend.ping().is_err() {
                    alive = false;
                    log(format!("{tag} idle connection closed by server"), false);
                }
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if matches!(cmd, Cmd::TrustHost(_)) {
            continue;
        }
        if sftp {
            trace::client(describe(&cmd));
        }
        let mut result = if alive {
            execute(&mut backend, &cmd, &progress)
        } else {
            Err("connection closed".into())
        };
        if result.is_err() && (!alive || backend.ping().is_err()) {
            if let Err(e) = &result {
                log(format!("{tag} {e}"), true);
            }
            log(format!("{tag} connection lost, reconnecting"), false);
            match open(known.as_deref()) {
                Ok((fresh, _, fingerprint)) => {
                    backend = fresh;
                    alive = true;
                    if fingerprint.is_some() {
                        known = fingerprint;
                    }
                    log(format!("{tag} reconnected, retrying"), false);
                    if sftp {
                        trace::client(describe(&cmd));
                    }
                    result = execute(&mut backend, &cmd, &progress);
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
                if sftp {
                    trace::server(format!("[SFTP] {summary}"));
                }
                send(event);
            }
            Err(e) => {
                log(format!("{tag} {e}"), true);
                send(Event::Error(e));
            }
        }
    }
    log(format!("{tag} disconnected"), false);
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
    let command = search_command(params);
    trace::client(format!("[SSH] exec: {command}"));
    if channel.exec(&command).is_err() {
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
            let Ok(data) = backend.download(&path, &|_, _| {}) else {
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
                _ => {
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

pub struct Tunnel {
    pub port: u16,
    stop: Arc<AtomicBool>,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn forward(
    session: &ssh2::Session,
    mut stream: TcpStream,
    host: &str,
    port: u16,
    stop: &AtomicBool,
) -> Result<(), String> {
    session.set_blocking(true);
    let mut channel = session
        .channel_direct_tcpip(host, port, None)
        .map_err(|e| e.to_string())?;
    session.set_blocking(false);
    stream.set_nonblocking(true).map_err(|e| e.to_string())?;
    let limit = 1024 * 1024;
    let (mut up, mut down): (Vec<u8>, Vec<u8>) = (Vec::new(), Vec::new());
    let mut buf = [0u8; 32 * 1024];
    let mut pinged = std::time::Instant::now();
    let blocked = |e: &std::io::Error| e.kind() == std::io::ErrorKind::WouldBlock;
    let result = loop {
        if stop.load(Ordering::Relaxed) {
            break Ok(());
        }
        let mut active = false;
        if up.len() < limit {
            match stream.read(&mut buf) {
                Ok(0) => break Ok(()),
                Ok(n) => {
                    up.extend_from_slice(&buf[..n]);
                    active = true;
                }
                Err(e) if blocked(&e) => {}
                Err(e) => break Err(e.to_string()),
            }
        }
        if !up.is_empty() {
            match channel.write(&up) {
                Ok(n) => {
                    up.drain(..n);
                    active |= n > 0;
                }
                Err(e) if blocked(&e) => {}
                Err(e) => break Err(e.to_string()),
            }
        }
        if down.len() < limit {
            match channel.read(&mut buf) {
                Ok(0) if channel.eof() => break Ok(()),
                Ok(n) => {
                    down.extend_from_slice(&buf[..n]);
                    active |= n > 0;
                }
                Err(e) if blocked(&e) => {}
                Err(e) => break Err(e.to_string()),
            }
        }
        if !down.is_empty() {
            match stream.write(&down) {
                Ok(n) => {
                    down.drain(..n);
                    active |= n > 0;
                }
                Err(e) if blocked(&e) => {}
                Err(e) => break Err(e.to_string()),
            }
        }
        if !active {
            if pinged.elapsed() > KEEPALIVE {
                pinged = std::time::Instant::now();
                let _ = session.keepalive_send();
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    session.set_blocking(true);
    let _ = channel.close();
    result
}

pub fn tunnel(
    profile: &Profile,
    known: Option<&str>,
    host: &str,
    port: u16,
) -> Result<Tunnel, String> {
    let (session, _) = open_session(profile, known, &|_, _| false).map_err(|e| {
        format!("{e} (для туннеля сначала подключитесь к хосту «{}» в панели хостов и подтвердите его ключ)", profile.name)
    })?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let local = listener.local_addr().map_err(|e| e.to_string())?.port();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let host = host.to_owned();
    trace::line(
        format!("[SSH] direct-tcpip tunnel 127.0.0.1:{local} -> {host}:{port}"),
        false,
    );
    std::thread::spawn(move || {
        let mut pinged = std::time::Instant::now();
        while !flag.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    if let Err(e) = forward(&session, stream, &host, port, &flag) {
                        trace::line(format!("[SSH] tunnel: {e}"), true);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if pinged.elapsed() > KEEPALIVE {
                        pinged = std::time::Instant::now();
                        let _ = session.keepalive_send();
                    }
                    std::thread::sleep(Duration::from_millis(30));
                }
                Err(_) => break,
            }
        }
        trace::line(format!("[SSH] tunnel 127.0.0.1:{local} closed"), false);
    });
    Ok(Tunnel { port: local, stop })
}

fn ssh_probe(profile: &Profile, known: Option<&str>) -> Result<(ssh2::Session, String), String> {
    let changed_key = std::cell::Cell::new(false);
    let confirm = |_: &str, changed: bool| {
        changed_key.set(changed);
        !changed
    };
    match open_session(profile, known, &confirm) {
        Ok((session, new_key)) => {
            let note = match new_key {
                Some(print) => format!(
                    "; ключ сервера новый (SHA-256 {}…) — при подключении его попросят подтвердить",
                    &print[..16.min(print.len())]
                ),
                None => String::new(),
            };
            Ok((session, note))
        }
        Err(_) if changed_key.get() => Err(
            "ключ сервера изменился — подключитесь и сверьте отпечаток, пароль не отправлялся".into(),
        ),
        Err(e) => Err(e),
    }
}

fn timed(started: std::time::Instant, result: Result<String, String>) -> Result<String, String> {
    let result = result.map(|text| format!("{text} ({} мс)", started.elapsed().as_millis()));
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
            let sftp = session.sftp().map_err(|e| format!("SFTP недоступен: {e}"))?;
            let home = sftp
                .realpath(Path::new("."))
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| "/".into());
            let start = profile.directory.trim();
            if !start.is_empty() && sftp.stat(Path::new(start)).is_err() {
                return Err(format!(
                    "вход выполнен, но начальной папки {start} нет на сервере"
                ));
            }
            Ok(format!("вход выполнен, домашняя папка {home}{note}"))
        }),
        _ => connect_ftp(profile).and_then(|(mut backend, home)| {
            let start = profile.directory.trim();
            if !start.is_empty() && backend.list(start).is_err() {
                return Err(format!(
                    "вход выполнен, но начальной папки {start} нет на сервере"
                ));
            }
            if let Backend::Ftp(ftp) = &mut backend {
                let _ = ftp.quit();
            }
            Ok(format!("вход выполнен, папка {home}"))
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
        TermProtocol::Telnet => (|| {
            let address = profile
                .address()
                .to_socket_addrs()
                .map_err(|e| format!("не удалось найти хост: {e}"))?
                .next()
                .ok_or("не удалось найти хост")?;
            let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)
                .map_err(|e| format!("нет соединения: {e}"))?;
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
                "порт открыт, сервер принял соединение".to_owned()
            } else {
                format!("порт открыт, сервер ответил: «{line}»")
            })
        })(),
        TermProtocol::Ssh => ssh_probe(&profile.ssh(), known.as_deref())
            .map(|(_, note)| format!("вход выполнен{note}")),
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
                Err(format!(
                    "вход по SSH выполнен, но {server} на сервере не найден — установите пакет mosh"
                ))
            } else {
                Ok(format!("вход выполнен, найден {path}{note}"))
            }
        }),
    };
    timed(started, result)
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
}
