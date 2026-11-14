use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, channel},
};

use serde::Deserialize;
use sha2::{Digest, Sha256};

const LATEST: &str = "https://api.github.com/repos/Jozef2J/tror/releases/latest";
const DOWNLOADS: &str = "https://github.com/Jozef2J/tror/releases/download/";
pub const PAGE: &str = "https://github.com/Jozef2J/tror/releases/latest";
const MAX_SIZE: u64 = 200 * 1024 * 1024;

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

#[derive(Deserialize)]
struct Latest {
    tag_name: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub file: Option<(String, String)>,
    pub sums: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Installer,
    Installed,
    Portable,
    Page,
}

pub enum Event {
    Found(Release),
    Current,
    Failed(String),
    Ready(PathBuf, Kind),
}

pub fn parse_version(text: &str) -> Option<[u32; 3]> {
    let mut parts = text.trim().trim_start_matches('v').split('.');
    let mut out = [0; 3];
    for slot in &mut out {
        *slot = parts.next()?.parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}

pub fn newer(found: &str, current: &str) -> bool {
    match (parse_version(found), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

pub fn kind() -> Kind {
    if cfg!(target_os = "macos") {
        return Kind::Page;
    }
    let Ok(exe) = std::env::current_exe() else {
        return Kind::Page;
    };
    let Some(dir) = exe.parent() else {
        return Kind::Page;
    };
    let probe = dir.join(".tror-write-test");
    let writable = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    let installed = cfg!(windows) && dir.join("unins000.exe").is_file();
    let renamed = exe
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("trorpad.exe"));
    match (installed, writable && renamed) {
        (true, true) => Kind::Installed,
        (true, false) => Kind::Installer,
        (false, true) => Kind::Portable,
        (false, false) => Kind::Page,
    }
}

fn asset_name(kind: Kind) -> Option<&'static str> {
    match kind {
        Kind::Page => None,
        Kind::Installer => Some("trorpad-setup.exe"),
        Kind::Installed => Some("trorpad.exe"),
        Kind::Portable if cfg!(windows) => Some("trorpad.exe"),
        Kind::Portable => Some("trorpad-linux-x86_64"),
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .timeout_global(Some(std::time::Duration::from_secs(600)))
        .timeout_connect(Some(std::time::Duration::from_secs(15)))
        .user_agent(concat!("trorpad/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

fn release_of(json: &str, kind: Kind) -> Result<Release, String> {
    let latest: Latest = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let trusted = |asset: &Asset| asset.browser_download_url.starts_with(DOWNLOADS);
    let pick = |name: &str| {
        latest
            .assets
            .iter()
            .find(|a| a.name == name && trusted(a))
            .map(|a| a.browser_download_url.clone())
    };
    Ok(Release {
        version: latest.tag_name.trim_start_matches('v').to_owned(),
        notes: latest.body.chars().take(1500).collect(),
        file: asset_name(kind).and_then(|name| Some((name.to_owned(), pick(name)?))),
        sums: pick("SHA256SUMS"),
    })
}

pub fn check() -> Receiver<Event> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let result = agent()
            .get(LATEST)
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(|e| e.to_string())
            .and_then(|mut r| r.body_mut().read_to_string().map_err(|e| e.to_string()))
            .and_then(|json| release_of(&json, kind()));
        let _ = tx.send(match result {
            Ok(release) if newer(&release.version, env!("CARGO_PKG_VERSION")) => {
                Event::Found(release)
            }
            Ok(_) => Event::Current,
            Err(e) => Event::Failed(e),
        });
    });
    rx
}

fn expected_sum(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (sum, file) = line.split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name && sum.len() == 64)
            .then(|| sum.to_ascii_lowercase())
    })
}

fn fetch(release: &Release, kind: Kind) -> Result<PathBuf, String> {
    let target = match kind {
        Kind::Installer => {
            std::env::temp_dir().join(format!("trorpad-setup-{}.exe", release.version))
        }
        _ => {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            exe.with_extension("new")
        }
    };
    fetch_to(release, &target)
}

fn fetch_to(release: &Release, target: &Path) -> Result<PathBuf, String> {
    let (name, url) = release
        .file
        .as_ref()
        .ok_or("в выпуске нет файла для этой системы")?;
    let sums_url = release
        .sums
        .as_ref()
        .ok_or("в выпуске нет файла контрольных сумм")?;
    let agent = agent();
    let sums = agent
        .get(sums_url)
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    let expected = expected_sum(&sums, name).ok_or("нет контрольной суммы для файла")?;
    let mut data = Vec::new();
    agent
        .get(url)
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .as_reader()
        .take(MAX_SIZE)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    let actual: String = Sha256::digest(&data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual != expected {
        return Err("контрольная сумма скачанного файла не совпала".into());
    }
    std::fs::write(target, &data).map_err(|e| e.to_string())?;
    Ok(target.to_path_buf())
}

pub fn download(release: Release) -> Receiver<Event> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let kind = kind();
        let _ = tx.send(match fetch(&release, kind) {
            Ok(path) => Event::Ready(path, kind),
            Err(e) => Event::Failed(e),
        });
    });
    rx
}

pub fn swap(exe: &Path, fresh: &Path) -> Result<(), String> {
    let old = exe.with_extension("old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old).map_err(|e| e.to_string())?;
    if let Err(e) = std::fs::rename(fresh, exe) {
        let _ = std::fs::rename(&old, exe);
        return Err(e.to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(exe, std::fs::Permissions::from_mode(0o755));
    }
    Ok(())
}

pub fn apply(path: &Path, version: &str) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    swap(&exe, path)?;
    if in_setup_dir(&exe) {
        registry_version(version);
    }
    Ok(())
}

fn in_setup_dir(exe: &Path) -> bool {
    cfg!(windows)
        && exe
            .parent()
            .is_some_and(|d| d.join("unins000.exe").is_file())
}

#[cfg(windows)]
fn registry_version(version: &str) {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("reg")
        .args([
            "add",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\{8F3B6C1E-5C0A-4E4B-9A55-7A6B1C0D2E11}_is1",
            "/v",
            "DisplayVersion",
            "/d",
            version,
            "/f",
        ])
        .creation_flags(0x0800_0000)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg(not(windows))]
fn registry_version(_version: &str) {}

pub fn notes(text: &str) -> Vec<(bool, String)> {
    let mut out: Vec<(bool, String)> = Vec::new();
    let mut open = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with("Co-Authored-By:")
            || line.starts_with("Claude-Session:")
            || line.starts_with('🤖')
            || line.contains("claude.ai/code")
        {
            continue;
        }
        if line.is_empty() {
            open = false;
            continue;
        }
        if let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            out.push((true, item.trim().to_owned()));
            open = true;
        } else if open && let Some(last) = out.last_mut() {
            last.1.push(' ');
            last.1.push_str(line);
        } else {
            out.push((false, line.to_owned()));
            open = out.len() > 1;
        }
    }
    out
}

pub fn run_setup(path: &Path) -> Result<(), String> {
    std::process::Command::new(path)
        .args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/SP-"])
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn restart() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::process::Command::new(exe)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn cleanup() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("old"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_join_wrapped_lines_and_drop_attribution() {
        let text = "0.17.0: title\n\nFirst para\ncontinues.\n\n- one\n  more\n- two\n\nCo-Authored-By: X\nClaude-Session: https://claude.ai/code/x\n";
        assert_eq!(
            notes(text),
            vec![
                (false, "0.17.0: title".into()),
                (false, "First para continues.".into()),
                (true, "one more".into()),
                (true, "two".into()),
            ]
        );
    }

    #[test]
    #[ignore]
    fn live_latest_release_downloads_and_matches_its_checksum() {
        let json = agent()
            .get(LATEST)
            .call()
            .unwrap()
            .body_mut()
            .read_to_string()
            .unwrap();
        let release = release_of(&json, Kind::Portable).unwrap();
        assert!(parse_version(&release.version).is_some());
        let dir = std::env::temp_dir().join(format!("tror-live-update-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = fetch_to(&release, &dir.join("tror.new")).unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > 1_000_000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(newer("v0.16.0", "0.15.1"));
        assert!(newer("0.10.0", "0.9.9"));
        assert!(!newer("0.15.1", "0.15.1"));
        assert!(!newer("0.15.0", "0.15.1"));
        assert!(!newer("latest", "0.15.1"));
        assert!(!newer("1.2", "0.15.1"));
    }

    #[test]
    fn only_assets_from_the_project_releases_are_accepted() {
        let json = r#"{"tag_name":"v9.0.0","body":"notes","assets":[
            {"name":"trorpad-setup.exe","browser_download_url":"https://evil.example/trorpad-setup.exe"},
            {"name":"SHA256SUMS","browser_download_url":"https://github.com/Jozef2J/tror/releases/download/v9.0.0/SHA256SUMS"}]}"#;
        let release = release_of(json, Kind::Installer).unwrap();
        assert_eq!(release.version, "9.0.0");
        assert_eq!(release.file, None);
        assert!(release.sums.is_some());
        let good = json.replace(
            "https://evil.example/",
            "https://github.com/Jozef2J/tror/releases/download/v9.0.0/",
        );
        assert!(release_of(&good, Kind::Installer).unwrap().file.is_some());
        assert_eq!(release_of(&good, Kind::Page).unwrap().file, None);
    }

    #[test]
    fn checksum_line_is_found_by_exact_name() {
        let sum = "a".repeat(64);
        let sums = format!(
            "{}  tror.exe.sig\n{sum}  tror.exe\nshort  tror-setup.exe\n",
            "b".repeat(64)
        );
        assert_eq!(expected_sum(&sums, "tror.exe"), Some(sum));
        assert_eq!(expected_sum(&sums, "tror-setup.exe"), None);
        assert_eq!(expected_sum(&sums, "other"), None);
    }

    #[test]
    fn swap_replaces_the_program_and_keeps_the_old_copy() {
        let dir = std::env::temp_dir().join(format!("tror-swap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("tror.exe");
        std::fs::write(&exe, b"old").unwrap();
        std::fs::write(dir.join("tror.new"), b"new").unwrap();
        swap(&exe, &dir.join("tror.new")).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        assert_eq!(std::fs::read(dir.join("tror.old")).unwrap(), b"old");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
