use super::*;

pub(super) fn tcp(profile: &Profile) -> Result<TcpStream, String> {
    let address = profile
        .address()
        .to_socket_addrs()
        .map_err(|e| tf!("не удалось найти хост: {e}", "could not resolve the host: {e}"))?
        .next()
        .ok_or(tr!("не удалось найти хост"))?;
    TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)
        .map_err(|e| tf!("нет соединения: {e}", "no connection: {e}"))
}

pub(super) fn fingerprint(session: &ssh2::Session) -> Option<String> {
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
        .map_err(|e| tf!("ошибка SSH: {e}", "SSH error: {e}"))?;
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
    let print = fingerprint(&session).ok_or(tr!("сервер не прислал ключ"))?;
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
            return Err(tr!("ключ сервера не подтверждён").into());
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
                tr!("файл ключа не найден"),
            ))
        } else {
            let phrase = if !profile.passphrase.is_empty() {
                Some(profile.passphrase.as_str())
            } else {
                (!profile.password.is_empty()).then_some(profile.password.as_str())
            };
            let text = std::fs::read_to_string(key).unwrap_or_default();
            if crate::remote::ppk::is_ppk(&text) {
                match crate::remote::ppk::to_openssh(&text, phrase.unwrap_or_default()) {
                    Ok(pem) => session.userauth_pubkey_memory(user, None, &pem, None),
                    Err(e) => {
                        trace::line(format!("[SSH] key file: {e}"), true);
                        return Err(tf!("вход не выполнен: {e}", "login failed: {e}"));
                    }
                }
            } else {
                let public = PathBuf::from(format!("{key}.pub"));
                let public = public.is_file().then_some(public.as_path());
                session.userauth_pubkey_file(user, public, Path::new(key), phrase)
            }
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
            return Err(tf!("вход не выполнен: {why}", "login failed: {why}"));
        }
    }
    auth.map_err(|e| tf!("вход не выполнен: {e}", "login failed: {e}"))?;
    trace::server("[SSH] authenticated");
    Ok((session, accepted))
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

pub(super) fn run_shell(
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
        .map_err(|e| tf!("не удалось открыть канал: {e}", "could not open a channel: {e}"))?;
    channel
        .request_pty(
            "xterm-256color",
            None,
            Some((size.0 as u32, size.1 as u32, 0, 0)),
        )
        .map_err(|e| tf!("сервер не дал терминал: {e}", "the server gave no terminal: {e}"))?;
    channel
        .shell()
        .map_err(|e| tf!("сервер не запустил оболочку: {e}", "the server did not start a shell: {e}"))?;
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
            Err(e) => return Err(tf!("соединение прервано: {e}", "connection interrupted: {e}")),
        }
        if channel.eof() {
            return Ok(tr!("сеанс завершён").into());
        }
        if !pending.is_empty() {
            match channel.write(&pending) {
                Ok(n) => {
                    pending.drain(..n);
                    active = true;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(tf!("соединение прервано: {e}", "connection interrupted: {e}")),
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
                    return Ok(tr!("отключено").into());
                }
            }
        }
        if !active {
            match cmds.recv_timeout(Duration::from_millis(10)) {
                Ok(cmd) => handle(cmd),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Ok(tr!("отключено").into());
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

pub struct Tunnel {
    pub port: u16,
    stop: Arc<AtomicBool>,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub(super) fn forward(
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
        tf!("{e} (для туннеля сначала подключитесь к хосту «{}» в панели хостов и подтвердите его ключ)", "{e} (for a tunnel, first connect to host \"{}\" in the hosts panel and confirm its key)", profile.name)
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

pub(super) fn ssh_probe(profile: &Profile, known: Option<&str>) -> Result<(ssh2::Session, String), String> {
    let changed_key = std::cell::Cell::new(false);
    let confirm = |_: &str, changed: bool| {
        changed_key.set(changed);
        !changed
    };
    match open_session(profile, known, &confirm) {
        Ok((session, new_key)) => {
            let note = match new_key {
                Some(print) => tf!("; ключ сервера новый (SHA-256 {}…) — при подключении его попросят подтвердить", "; the server key is new (SHA-256 {}…) — you will be asked to confirm it when connecting",
                    &print[..16.min(print.len())]
                ),
                None => String::new(),
            };
            Ok((session, note))
        }
        Err(_) if changed_key.get() => Err(
            tr!("ключ сервера изменился — подключитесь и сверьте отпечаток, пароль не отправлялся").into(),
        ),
        Err(e) => Err(e),
    }
}
