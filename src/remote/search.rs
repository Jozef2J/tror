use super::*;

pub struct SearchParams {
    pub root: String,
    pub query: String,
    pub content: bool,
    pub excludes: Vec<String>,
    pub case: bool,
    pub word: bool,
    pub regex: bool,
}

pub(super) fn ere_escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if "\\^$.|?*+()[]{}".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
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

pub(super) const MAX_HITS: usize = 2000;

pub(super) const MAX_GREP_FILE: u64 = 2 * 1024 * 1024;

pub(super) const BINARY: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "ico", "bmp", "pdf", "zip", "gz", "tar", "rar", "7z",
    "woff", "woff2", "ttf", "eot", "otf", "mp3", "mp4", "avi", "mov", "exe", "dll", "so", "phar",
    "doc", "docx", "xls", "xlsx",
];

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
        let mut flags = String::from("-InH");
        if !params.case {
            flags.push_str(" -i");
        }
        if params.word {
            flags.push_str(" -w");
        }
        flags.push_str(if params.regex { " -E" } else { " -F" });
        format!(
            "{check}find -H {root} {prune}-type f -size -{}k -print0 2>/dev/null | xargs -0 grep {flags} -e {} -- 2>/dev/null | head -n {MAX_HITS}",
            MAX_GREP_FILE / 1024,
            quote(&params.query)
        )
    } else if params.regex || params.word {
        let mut re = if params.regex {
            params.query.clone()
        } else {
            ere_escape(&params.query)
        };
        if params.word {
            re = format!("(^|[^[:alnum:]_])({re})([^[:alnum:]_]|$)");
        }
        let (name, re) = if params.case {
            ("$NF", re)
        } else {
            ("tolower($NF)", re.to_lowercase())
        };
        format!(
            "{check}find -H {root} {prune}-print 2>/dev/null | RE={} awk -F/ '{name} ~ ENVIRON[\"RE\"]' | head -n {MAX_HITS}",
            quote(&re)
        )
    } else {
        let mut glob = String::new();
        for c in params.query.chars() {
            if matches!(c, '*' | '?' | '[' | ']' | '\\') {
                glob.push('\\');
            }
            glob.push(c);
        }
        let test = if params.case { "-name" } else { "-iname" };
        format!(
            "{check}find -H {root} {prune}{test} {} -print 2>/dev/null | head -n {MAX_HITS}",
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

pub(super) fn excluded(path: &str, name: &str, excludes: &[String]) -> bool {
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

pub(super) fn search_remote_shell(
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

pub(super) fn search_walk(
    backend: &mut Backend,
    params: &SearchParams,
    cancel: &AtomicBool,
    emit: &dyn Fn(SearchEvent),
) -> usize {
    let re = match crate::findfiles::pattern(&params.query, params.regex, params.case, params.word)
    {
        Ok(re) => re,
        Err(e) => {
            emit(SearchEvent::Status(e));
            return 0;
        }
    };
    let mut stack = vec![params.root.clone()];
    let mut hits = 0;
    let mut dirs = 0;
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) || hits >= MAX_HITS {
            break;
        }
        dirs += 1;
        emit(SearchEvent::Status(tf!("просмотрено каталогов: {dirs}", "folders scanned: {dirs}"
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
            if entry.is_dir && !entry.link {
                stack.push(path.clone());
            }
            if !params.content {
                if re.is_match(&entry.name) {
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
                if re.is_match(line) {
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
                        return Ok((hits, tr!("поиск командами сервера")));
                    }
                    let sftp = session.sftp().map_err(|e| e.to_string())?;
                    let mut backend = Backend::Sftp {
                        session,
                        sftp,
                        socket: None,
                    };
                    Ok((
                        search_walk(&mut backend, &params, &flag, &emit),
                        tr!("обход каталогов"),
                    ))
                }
                _ => {
                    let (mut backend, _) = connect_ftp(&profile)?;
                    Ok((
                        search_walk(&mut backend, &params, &flag, &emit),
                        tr!("обход каталогов"),
                    ))
                }
            }
        };
        match run() {
            Ok((hits, how)) => {
                let stopped = if flag.load(Ordering::Relaxed) {
                    tr!(", остановлен")
                } else if hits >= MAX_HITS {
                    tr!(", показаны первые результаты")
                } else {
                    ""
                };
                emit(SearchEvent::Done(tf!("найдено: {hits} ({how}{stopped})", "found: {hits} ({how}{stopped})"
                )));
            }
            Err(e) => emit(SearchEvent::Failed(e)),
        }
    });
    SearchJob { rx, cancel }
}
