use super::*;

pub(super) const MASK: &[u8] = b"tror-profile-mask";

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
            Protocol::FtpsImplicit => tr!("FTPS неявный"),
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

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct Store {
    pub profiles: Vec<Profile>,
    pub known_hosts: BTreeMap<String, String>,
    pub hide_size: bool,
    pub show_date: bool,
    pub search_excludes: Option<String>,
    pub databases: Vec<DbProfile>,
    pub terminals: Vec<TermProfile>,
    pub show_perm: bool,
    #[serde(skip)]
    path: Option<PathBuf>,
    #[serde(skip)]
    pub(super) locked: bool,
}

pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let temp = path.with_extension("tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    #[cfg(unix)]
    std::fs::set_permissions(&temp, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    std::fs::rename(&temp, path)
}

pub(super) static MIGRATED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(super) static STORE_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl Store {
    pub fn load(path: Option<PathBuf>) -> Self {
        let mut store: Store = path
            .as_deref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        store.path = path;
        if crate::remote::vault::in_file() {
            return store;
        }
        match crate::remote::vault::read() {
            Ok(map) => {
                let mut plain = false;
                for (key, field) in store.secrets() {
                    if !field.is_empty() {
                        plain = true;
                    } else if let Some(value) = map.get(&key) {
                        field.clone_from(value);
                    }
                }
                if plain && store.path.is_some() && !MIGRATED.swap(true, Ordering::Relaxed) {
                    let _ = store.save();
                }
            }
            Err(e) => {
                store.locked = true;
                static REPORTED: AtomicBool = AtomicBool::new(false);
                if !REPORTED.swap(true, Ordering::Relaxed) {
                    crate::trace::line(tf!("[Пароли] системное хранилище недоступно: {e}", "[Passwords] the system store is not available: {e}"), true);
                }
            }
        }
        store
    }

    pub(super) fn secrets(&mut self) -> Vec<(String, &mut String)> {
        let mut out = Vec::new();
        for p in &mut self.profiles {
            let id = format!("host|{}|{}@{}:{}", p.name, p.user, p.host, p.port);
            out.push((format!("{id}|password"), &mut p.password));
            out.push((format!("{id}|passphrase"), &mut p.passphrase));
        }
        for p in &mut self.terminals {
            let id = format!("term|{}|{}@{}:{}", p.name, p.user, p.host, p.port);
            out.push((format!("{id}|password"), &mut p.password));
            out.push((format!("{id}|passphrase"), &mut p.passphrase));
        }
        for p in &mut self.databases {
            let id = format!("db|{}|{}@{}:{}", p.name, p.user, p.host, p.port);
            out.push((format!("{id}|password"), &mut p.password));
        }
        out
    }

    pub fn switch_vault(path: Option<PathBuf>, in_file: bool) -> Result<(), String> {
        let store = Store::load(path);
        if store.locked {
            return Err(tr!("системное хранилище недоступно").into());
        }
        crate::remote::vault::set_in_file(in_file);
        if store.path.is_some() {
            store.save()?;
        }
        Ok(())
    }

    pub fn save(&self) -> Result<(), String> {
        let path = self.path.as_deref().ok_or(tr!("нет каталога настроек"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let mut copy = self.clone();
        if crate::remote::vault::in_file() {
            if !self.locked {
                crate::remote::vault::clear();
            }
        } else if !self.locked {
            let map: BTreeMap<String, String> = copy
                .secrets()
                .into_iter()
                .filter(|(_, field)| !field.is_empty())
                .map(|(key, field)| (key, field.clone()))
                .collect();
            if crate::remote::vault::write(&map).is_ok() {
                for (_, field) in copy.secrets() {
                    field.clear();
                }
            }
        }
        let text = serde_json::to_string_pretty(&copy).map_err(|e| e.to_string())?;
        write_private(path, text.as_bytes()).map_err(|e| e.to_string())?;
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
    Local,
}

impl TermProtocol {
    pub const ALL: [TermProtocol; 3] = [TermProtocol::Ssh, TermProtocol::Telnet, TermProtocol::Mosh];

    pub fn label(self) -> &'static str {
        match self {
            TermProtocol::Ssh => "SSH",
            TermProtocol::Telnet => "Telnet",
            TermProtocol::Mosh => "Mosh",
            TermProtocol::Local => tr!("Локальный"),
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            TermProtocol::Ssh | TermProtocol::Mosh => 22,
            TermProtocol::Telnet => 23,
            TermProtocol::Local => 0,
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


pub fn key_kind(path: &str) -> Result<String, String> {
    use base64::Engine;
    let text = std::fs::read_to_string(path.trim()).map_err(|e| e.to_string())?;
    if crate::remote::ppk::is_ppk(&text) {
        let first = text.lines().next().unwrap_or_default();
        let kind = first.split(':').nth(1).unwrap_or_default().trim();
        if kind == "ssh-dss" {
            return Err(tr!(super::ppk::DSA_NOTE).into());
        }
        let name = match kind {
            "ssh-ed25519" => "Ed25519".to_owned(),
            "ssh-rsa" => "RSA".to_owned(),
            k => format!("ECDSA {}", k.trim_start_matches("ecdsa-sha2-")),
        };
        let locked = !text.lines().any(|l| l.trim() == "Encryption: none");
        return Ok(if locked {
            tf!("PuTTY .ppk, {name}, защищён парольной фразой", "PuTTY .ppk, {name}, protected with a passphrase")
        } else {
            format!("PuTTY .ppk, {name}")
        });
    }
    let header = text
        .lines()
        .find(|l| l.starts_with("-----BEGIN "))
        .ok_or(tr!("это не закрытый ключ OpenSSH или PEM"))?;
    let encrypted = text.contains("ENCRYPTED");
    let lock = |name: &str| {
        if encrypted {
            tf!("{name}, защищён парольной фразой", "{name}, protected with a passphrase")
        } else {
            name.to_owned()
        }
    };
    Ok(match header {
        h if h.contains("RSA PRIVATE KEY") => lock("RSA"),
        h if h.contains("DSA PRIVATE KEY") => return Err(tr!(super::ppk::DSA_NOTE).into()),
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
                return Err(tr!("повреждённый ключ OpenSSH").into());
            }
            let mut at = magic.len();
            fn field(bytes: &[u8], at: &mut usize) -> Option<Vec<u8>> {
                let len = u32::from_be_bytes(bytes.get(*at..*at + 4)?.try_into().ok()?) as usize;
                let value = bytes.get(*at + 4..*at + 4 + len)?.to_vec();
                *at += 4 + len;
                Some(value)
            }
            let cipher = field(&bytes, &mut at).ok_or(tr!("повреждённый ключ OpenSSH"))?;
            field(&bytes, &mut at);
            field(&bytes, &mut at);
            at += 4;
            let public = field(&bytes, &mut at).ok_or(tr!("повреждённый ключ OpenSSH"))?;
            let len = public
                .get(..4)
                .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
                .unwrap_or(0);
            let kind = String::from_utf8_lossy(public.get(4..4 + len).unwrap_or_default());
            let name = match kind.as_ref() {
                "ssh-ed25519" => "Ed25519".to_owned(),
                "ssh-rsa" => "RSA".to_owned(),
                "ssh-dss" => return Err(tr!(super::ppk::DSA_NOTE).into()),
                k if k.starts_with("ecdsa-sha2-") => format!("ECDSA {}", &k[11..]),
                k if k.starts_with("sk-") => tf!("{k} (ключ безопасности)", "{k} (security key)"),
                k => k.to_owned(),
            };
            if cipher != b"none" {
                tf!("{name}, защищён парольной фразой", "{name}, protected with a passphrase")
            } else {
                name
            }
        }
        h if h.contains("ENCRYPTED PRIVATE KEY") => tr!("PKCS#8, защищён парольной фразой").into(),
        h if h.contains("PRIVATE KEY") => "PKCS#8".into(),
        _ => return Err(tr!("это не закрытый ключ").into()),
    })
}
