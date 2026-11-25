use crate::remote::{Profile, Protocol};
use base64::Engine;

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn remote_dir(raw: &str) -> String {
    let mut parts = raw.split(' ');
    let (Some(_), Some(_)) = (parts.next(), parts.next()) else {
        return String::new();
    };
    let rest: Vec<&str> = parts.collect();
    let joined = rest.join(" ");
    let mut out = String::new();
    let mut tail = joined.as_str();
    while let Some((len, after)) = tail.split_once(' ') {
        let Ok(len) = len.parse::<usize>() else {
            break;
        };
        let Some(name) = after.get(..len) else {
            break;
        };
        out.push('/');
        out.push_str(name);
        tail = after.get(len..).unwrap_or("").trim_start();
    }
    out
}

pub fn filezilla(xml: &str) -> Vec<Profile> {
    let mut out = Vec::new();
    let mut path: Vec<String> = Vec::new();
    let mut folders: Vec<String> = Vec::new();
    let mut fields: Vec<(String, String, String)> = Vec::new();
    let mut attrs = String::new();
    let mut rest = xml;
    while let Some(open) = rest.find('<') {
        let text = &rest[..open];
        let Some(close) = rest[open..].find('>') else {
            break;
        };
        let tag = &rest[open + 1..open + close];
        rest = &rest[open + close + 1..];
        if tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        if let Some(name) = tag.strip_prefix('/') {
            let name = name.trim();
            if path.last().is_some_and(|p| p == name) {
                path.pop();
            }
            match name {
                "Server" => {
                    if let Some(profile) = server(&fields, &folders) {
                        out.push(profile);
                    }
                    fields.clear();
                }
                "Folder" => {
                    folders.pop();
                }
                _ if path.last().is_some_and(|p| p == "Server") => {
                    fields.push((
                        name.to_owned(),
                        unescape(text.trim()),
                        std::mem::take(&mut attrs),
                    ));
                }
                _ => {}
            }
            continue;
        }
        if path.last().is_some_and(|p| p == "Folder")
            && !text.trim().is_empty()
            && let Some(folder) = folders.last_mut()
            && folder.is_empty()
        {
            *folder = unescape(text.trim());
        }
        let self_closing = tag.ends_with('/');
        let tag = tag.trim_end_matches('/');
        let (name, tag_attrs) = tag.split_once(char::is_whitespace).unwrap_or((tag, ""));
        if self_closing {
            continue;
        }
        if name == "Folder" {
            folders.push(String::new());
        }
        attrs = tag_attrs.to_owned();
        path.push(name.to_owned());
    }
    out
}

fn server(fields: &[(String, String, String)], folders: &[String]) -> Option<Profile> {
    let get = |key: &str| fields.iter().find(|(k, ..)| k == key);
    let host = get("Host")?.1.clone();
    if host.is_empty() {
        return None;
    }
    let protocol = match get("Protocol").map(|f| f.1.as_str()) {
        Some("1") => Protocol::Sftp,
        Some("3") => Protocol::FtpsImplicit,
        Some("4") => Protocol::Ftps,
        _ => Protocol::Ftp,
    };
    let name = get("Name")
        .map(|f| f.1.clone())
        .unwrap_or_else(|| host.clone());
    let prefix: Vec<&str> = folders
        .iter()
        .map(String::as_str)
        .filter(|f| !f.is_empty())
        .collect();
    let mut profile = Profile::new(if prefix.is_empty() {
        name
    } else {
        format!("{} / {name}", prefix.join(" / "))
    });
    profile.protocol = protocol;
    profile.host = host;
    profile.port = get("Port")
        .and_then(|f| f.1.parse().ok())
        .unwrap_or(if protocol == Protocol::Sftp { 22 } else { 21 });
    profile.user = get("User").map(|f| f.1.clone()).unwrap_or_default();
    if let Some((_, pass, attrs)) = get("Pass") {
        if attrs.contains("base64") {
            profile.password = base64::engine::general_purpose::STANDARD
                .decode(pass.as_bytes())
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
                .unwrap_or_default();
        } else if !attrs.contains("crypt") {
            profile.password = pass.clone();
        }
    }
    profile.key_file = get("Keyfile").map(|f| f.1.clone()).unwrap_or_default();
    profile.directory = get("RemoteDir")
        .map(|f| remote_dir(&f.1))
        .unwrap_or_default();
    Some(profile)
}

fn url_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = text.get(i + 1..i + 3)
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn winscp_password(host: &str, user: &str, encrypted: &str) -> Option<String> {
    let nibbles: Vec<u8> = encrypted
        .chars()
        .map(|c| c.to_digit(16).map(|d| d as u8))
        .collect::<Option<_>>()?;
    let mut at = 0;
    let mut next = || -> Option<u8> {
        let a = *nibbles.get(at)?;
        let b = *nibbles.get(at + 1)?;
        at += 2;
        Some(!(((a << 4) | b) ^ 0xA3))
    };
    let flag = next()?;
    let length = if flag == 0xFF {
        next()?;
        next()?
    } else {
        flag
    };
    let skip = next()?;
    for _ in 0..skip {
        next()?;
    }
    let bytes: Vec<u8> = (0..length).map(|_| next()).collect::<Option<_>>()?;
    let clear = String::from_utf8_lossy(&bytes).into_owned();
    if flag == 0xFF {
        clear
            .strip_prefix(&format!("{user}{host}"))
            .map(str::to_owned)
    } else {
        Some(clear)
    }
}

pub fn winscp(sections: &[(String, Vec<(String, String)>)]) -> Vec<Profile> {
    let mut out = Vec::new();
    for (section, values) in sections {
        let get = |key: &str| {
            values
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v.as_str())
        };
        let Some(host) = get("HostName").filter(|h| !h.is_empty()) else {
            continue;
        };
        let name = url_decode(section);
        if name == "Default Settings" {
            continue;
        }
        let ftp = get("FSProtocol") == Some("5");
        let protocol = match (ftp, get("Ftps")) {
            (false, _) => Protocol::Sftp,
            (true, Some("1")) => Protocol::FtpsImplicit,
            (true, Some("2" | "3")) => Protocol::Ftps,
            (true, _) => Protocol::Ftp,
        };
        let mut profile = Profile::new(name.replace('/', " / "));
        profile.protocol = protocol;
        profile.host = host.to_owned();
        profile.port = get("PortNumber")
            .and_then(parse_number)
            .unwrap_or(if protocol == Protocol::Sftp { 22 } else { 21 });
        profile.user = get("UserName").map(url_decode).unwrap_or_default();
        if let Some(encrypted) = get("Password") {
            profile.password = winscp_password(host, &profile.user, encrypted).unwrap_or_default();
        }
        profile.directory = get("RemoteDirectory").map(url_decode).unwrap_or_default();
        profile.key_file = get("PublicKeyFile").map(url_decode).unwrap_or_default();
        out.push(profile);
    }
    out
}

fn parse_number(text: &str) -> Option<u16> {
    let text = text.trim();
    match text.strip_prefix("0x") {
        Some(hex) => u16::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}

pub fn winscp_ini(text: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut sections: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some(name) = line
            .strip_prefix("[Sessions\\")
            .and_then(|l| l.strip_suffix(']'))
        {
            sections.push((name.to_owned(), Vec::new()));
        } else if line.starts_with('[') {
            sections.push((String::new(), Vec::new()));
        } else if let Some((key, value)) = line.split_once('=')
            && let Some(last) = sections.last_mut()
            && !last.0.is_empty()
        {
            last.1
                .push((key.trim().to_owned(), value.trim().to_owned()));
        }
    }
    sections.retain(|(name, _)| !name.is_empty());
    sections
}

#[cfg(any(windows, test))]
pub fn winscp_registry(text: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut sections: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for line in text.lines() {
        if let Some((_, name)) = line.trim().split_once("\\Sessions\\") {
            sections.push((name.trim().to_owned(), Vec::new()));
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2
            && parts[1].starts_with("REG_")
            && let Some(last) = sections.last_mut()
        {
            let value = line
                .split_once(parts[1])
                .map(|(_, v)| v.trim().to_owned())
                .unwrap_or_default();
            last.1.push((parts[0].to_owned(), value));
        }
    }
    sections
}

pub fn filezilla_file() -> Option<std::path::PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(std::path::PathBuf::from)?
            .join("FileZilla")
    } else {
        let home = std::path::PathBuf::from(std::env::var_os("HOME")?);
        std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("filezilla")
    };
    let path = base.join("sitemanager.xml");
    path.is_file().then_some(path)
}

pub fn winscp_sessions() -> Vec<(String, Vec<(String, String)>)> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if let Ok(output) = std::process::Command::new("reg")
            .args([
                "query",
                r"HKCU\Software\Martin Prikryl\WinSCP 2\Sessions",
                "/s",
            ])
            .creation_flags(0x0800_0000)
            .output()
        {
            let text = String::from_utf8_lossy(&output.stdout);
            let found = winscp_registry(&text);
            if !found.is_empty() {
                return found;
            }
        }
        if let Some(path) = std::env::var_os("APPDATA")
            .map(|d| std::path::PathBuf::from(d).join("WinSCP.ini"))
            .filter(|p| p.is_file())
            && let Ok(text) = std::fs::read_to_string(path)
        {
            return winscp_ini(&text);
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filezilla_sites_with_folders_and_passwords() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<FileZilla3 version="3.66.0" platform="windows">
  <Servers>
    <Folder expanded="1">Клиенты
      <Server>
        <Host>shop.example.de</Host>
        <Port>2222</Port>
        <Protocol>1</Protocol>
        <Type>0</Type>
        <User>web &amp; co</User>
        <Pass encoding="base64">c2VjcmV0</Pass>
        <Logontype>1</Logontype>
        <Name>Магазин</Name>
        <RemoteDir>1 0 3 www 4 html</RemoteDir>
        <Keyfile/>
      </Server>
    </Folder>
    <Server>
      <Host>ftp.example.com</Host>
      <Port>21</Port>
      <Protocol>4</Protocol>
      <User>bob</User>
      <Pass encoding="crypt">ABCDEF</Pass>
      <Name>Old</Name>
    </Server>
  </Servers>
</FileZilla3>"#;
        let found = filezilla(xml);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].name, "Клиенты / Магазин");
        assert_eq!(found[0].protocol, Protocol::Sftp);
        assert_eq!(found[0].port, 2222);
        assert_eq!(found[0].user, "web & co");
        assert_eq!(found[0].password, "secret");
        assert_eq!(found[0].directory, "/www/html");
        assert_eq!(found[1].protocol, Protocol::Ftps);
        assert_eq!(found[1].password, "");
    }

    #[test]
    fn winscp_ini_and_registry_with_encrypted_password() {
        let ini = "[Configuration]\nX=1\n[Sessions\\Clients/My%20Site]\nHostName=example.com\nUserName=user\nPortNumber=2200\nPassword=A35C755E6D593D323332313139666D6E726D6A64726D6E726D6A64726D6E726D6A64726D6E72\nRemoteDirectory=/var/www\n[Sessions\\Default%20Settings]\nHostName=x\n[Sessions\\Ftp]\nHostName=ftp.example.com\nFSProtocol=5\nFtps=3\n";
        let found = winscp(&winscp_ini(ini));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].name, "Clients / My Site");
        assert_eq!(found[0].port, 2200);
        assert_eq!(found[0].directory, "/var/www");
        assert_eq!(found[1].protocol, Protocol::Ftps);
        assert_eq!(found[1].port, 21);
        let reg = "\r\nHKEY_CURRENT_USER\\Software\\Martin Prikryl\\WinSCP 2\\Sessions\\site%20one\r\n    HostName    REG_SZ    host.example\r\n    PortNumber    REG_DWORD    0x16\r\n    UserName    REG_SZ    root\r\n";
        let found = winscp(&winscp_registry(reg));
        assert_eq!(found[0].name, "site one");
        assert_eq!(found[0].host, "host.example");
        assert_eq!(found[0].port, 22);
        assert_eq!(found[0].user, "root");
    }

    #[test]
    fn winscp_password_roundtrip() {
        let encrypt = |host: &str, user: &str, pass: &str| {
            let plain = format!("{user}{host}{pass}");
            let enc = |b: u8| format!("{:02X}", !(b) ^ 0xA3);
            let mut out = String::new();
            out.push_str(&enc(0xFF));
            out.push_str(&enc(0));
            out.push_str(&enc(plain.len() as u8));
            out.push_str(&enc(0));
            for b in plain.bytes() {
                out.push_str(&enc(b));
            }
            out
        };
        let encrypted = encrypt("example.com", "user", "p@ss wörd");
        assert_eq!(
            winscp_password("example.com", "user", &encrypted).as_deref(),
            Some("p@ss wörd")
        );
    }
}
