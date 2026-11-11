use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

const RELOAD: &str = "<script>(function(){var v=null;setInterval(function(){fetch('/__tror_version',{cache:'no-store'}).then(function(r){return r.text()}).then(function(t){if(v===null){v=t}else if(v!==t){location.reload()}}).catch(function(){})},700)})()</script>";

struct Site {
    root: PathBuf,
    port: u16,
}

#[derive(Default)]
pub struct Preview {
    version: Arc<AtomicU64>,
    site: Option<Site>,
    php: Option<(PathBuf, u16, Child)>,
}

impl Drop for Preview {
    fn drop(&mut self) {
        if let Some((_, _, child)) = &mut self.php {
            let _ = child.kill();
        }
    }
}

fn mime(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "html" | "htm" | "xhtml" | "twig" | "tpl" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "pdf" => "application/pdf",
        "xml" => "application/xml; charset=utf-8",
        "txt" | "md" | "log" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

pub fn decode(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = (bytes[i] == b'%' && i + 2 < bytes.len())
            .then(|| path.get(i + 1..i + 3))
            .flatten()
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match hex {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn encode(path: &str) -> String {
    path.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub fn resolve(root: &Path, target: &str) -> Option<PathBuf> {
    let path = target.split(['?', '#']).next().unwrap_or("");
    let mut full = root.to_path_buf();
    for part in decode(path).split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => return None,
            part if part.contains(':') => return None,
            part => full.push(part),
        }
    }
    if full.is_dir() {
        full.push("index.html");
    }
    Some(full)
}

fn inject(mut body: Vec<u8>) -> Vec<u8> {
    let lower = body.to_ascii_lowercase();
    let at = lower
        .windows(7)
        .rposition(|w| w == b"</body>")
        .unwrap_or(body.len());
    body.splice(at..at, RELOAD.bytes());
    body
}

fn respond(stream: &mut TcpStream, status: &str, kind: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

fn handle(mut stream: TcpStream, root: &Path, version: &AtomicU64) {
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() {
        return;
    }
    let mut parts = line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return;
    };
    if method != "GET" && method != "HEAD" {
        respond(&mut stream, "405 Method Not Allowed", "text/plain", b"");
        return;
    }
    if target.starts_with("/__tror_version") {
        let text = version.load(Ordering::Relaxed).to_string();
        respond(&mut stream, "200 OK", "text/plain", text.as_bytes());
        return;
    }
    let Some(path) = resolve(root, target) else {
        respond(&mut stream, "403 Forbidden", "text/plain", b"403");
        return;
    };
    match std::fs::read(&path) {
        Ok(body) => {
            let kind = mime(&path);
            let body = if kind.starts_with("text/html") {
                inject(body)
            } else {
                body
            };
            respond(&mut stream, "200 OK", kind, &body);
        }
        Err(_) => respond(
            &mut stream,
            "404 Not Found",
            "text/plain; charset=utf-8",
            "404 — файл не найден".as_bytes(),
        ),
    }
}

pub fn open_url(url: &str) -> Result<(), String> {
    let mut command = if cfg!(target_os = "windows") {
        let mut c = Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("не удалось открыть браузер: {e}"))
}

#[cfg(windows)]
fn quiet(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x0800_0000);
}

#[cfg(not(windows))]
fn quiet(_command: &mut Command) {}

fn free_port() -> Result<u16, String> {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| e.to_string())
}

impl Preview {
    pub fn touch(&self) {
        self.version.fetch_add(1, Ordering::Relaxed);
    }

    fn serve(&mut self, root: &Path) -> Result<u16, String> {
        if let Some(site) = &self.site
            && site.root == root
        {
            return Ok(site.port);
        }
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let version = self.version.clone();
        let dir = root.to_path_buf();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let dir = dir.clone();
                let version = version.clone();
                std::thread::spawn(move || handle(stream, &dir, &version));
            }
        });
        self.site = Some(Site {
            root: root.to_path_buf(),
            port,
        });
        Ok(port)
    }

    fn serve_php(&mut self, root: &Path) -> Result<u16, String> {
        if let Some((dir, port, child)) = &mut self.php {
            if dir == root && matches!(child.try_wait(), Ok(None)) {
                return Ok(*port);
            }
            let _ = child.kill();
            self.php = None;
        }
        let port = free_port()?;
        let mut command = Command::new("php");
        quiet(&mut command);
        let child = command
            .arg("-S")
            .arg(format!("127.0.0.1:{port}"))
            .arg("-t")
            .arg(root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| {
                "для предпросмотра PHP нужен установленный php в PATH (запускается php -S)"
                    .to_owned()
            })?;
        for _ in 0..60 {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        self.php = Some((root.to_path_buf(), port, child));
        Ok(port)
    }

    pub fn url(&mut self, file: &Path) -> Result<String, String> {
        let root = file.parent().ok_or("у файла нет каталога")?;
        let name = file
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("непонятное имя файла")?;
        let ext = file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let port = if matches!(ext.as_str(), "php" | "phtml") {
            self.serve_php(root)?
        } else {
            self.serve(root)?
        };
        Ok(format!("http://127.0.0.1:{port}/{}", encode(name)))
    }
}

pub fn remote_url(base: &str, directory: &str, path: &str) -> Option<String> {
    let base = base.trim().trim_end_matches('/');
    if base.is_empty() {
        return None;
    }
    let base = if base.contains("://") {
        base.to_owned()
    } else {
        format!("http://{base}")
    };
    let directory = directory.trim().trim_end_matches('/');
    let relative = path.strip_prefix(directory).unwrap_or(path);
    Some(format!(
        "{base}/{}",
        encode(relative.trim_start_matches('/'))
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn paths_stay_inside_root() {
        let root = Path::new("/srv/site");
        assert_eq!(
            resolve(root, "/css/a%20b.css?v=1"),
            Some(PathBuf::from("/srv/site/css/a b.css"))
        );
        assert_eq!(resolve(root, "/../etc/passwd"), None);
        assert_eq!(resolve(root, "/a/%2e%2e/%2e%2e/x"), None);
        assert_eq!(resolve(root, "/c:/windows"), None);
        assert_eq!(
            encode("папка/a b.html"),
            "%D0%BF%D0%B0%D0%BF%D0%BA%D0%B0/a%20b.html"
        );
        assert_eq!(
            decode("%D0%BF%D0%B0%D0%BF%D0%BA%D0%B0/a%20b.html"),
            "папка/a b.html"
        );
        assert_eq!(decode("100%"), "100%");
    }

    #[test]
    fn remote_urls_are_relative_to_profile_directory() {
        assert_eq!(
            remote_url(
                "https://site.ru/",
                "/home/u/www",
                "/home/u/www/catalog/index.php"
            ),
            Some("https://site.ru/catalog/index.php".into())
        );
        assert_eq!(
            remote_url("site.ru", "", "/a b.html"),
            Some("http://site.ru/a%20b.html".into())
        );
        assert_eq!(remote_url(" ", "", "/x"), None);
    }

    #[test]
    fn server_serves_files_with_reload_script() {
        let dir = std::env::temp_dir().join(format!("tror_preview_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("page.html"),
            "<html><body><h1>Привет</h1></body></html>",
        )
        .unwrap();
        std::fs::write(dir.join("s.css"), "h1{color:red}").unwrap();
        let mut preview = Preview::default();
        let url = preview.url(&dir.join("page.html")).unwrap();
        let port: u16 = url
            .split(':')
            .nth(2)
            .unwrap()
            .split('/')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let get = |path: &str| {
            let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(stream, "GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
            let mut text = String::new();
            stream.read_to_string(&mut text).unwrap();
            text
        };
        let page = get("/page.html");
        assert!(page.starts_with("HTTP/1.1 200 OK"));
        assert!(page.contains("<h1>Привет</h1><script>"));
        assert!(page.contains("__tror_version"));
        let css = get("/s.css");
        assert!(css.contains("text/css") && css.ends_with("h1{color:red}"));
        assert!(get("/nope.html").starts_with("HTTP/1.1 404"));
        assert!(get("/../x").starts_with("HTTP/1.1 403"));
        let before = get("/__tror_version");
        preview.touch();
        assert_ne!(before, get("/__tror_version"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore]
    fn php_files_run_through_php_server() {
        let dir = std::env::temp_dir().join(format!("tror_php_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("i.php"), "<?php echo 6 * 7;").unwrap();
        let mut preview = Preview::default();
        let url = preview.url(&dir.join("i.php")).unwrap();
        let port: u16 = url
            .split(':')
            .nth(2)
            .unwrap()
            .split('/')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            stream,
            "GET /i.php HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut text = String::new();
        stream.read_to_string(&mut text).unwrap();
        assert!(text.ends_with("42"), "{text}");
        assert_eq!(preview.url(&dir.join("i.php")).unwrap(), url);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
