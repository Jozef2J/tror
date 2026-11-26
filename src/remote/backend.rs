use super::*;

pub(super) enum Backend {
    Sftp {
        session: ssh2::Session,
        sftp: ssh2::Sftp,
        socket: Option<TcpStream>,
        chunk: usize,
    },
    Ftp(Box<suppaftp::NativeTlsFtpStream>),
}

pub(super) fn connect_sftp(
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
    let stream = tcp(profile)?;
    let socket = stream.try_clone().ok();
    let (session, accepted) = handshake(stream, profile, known, &confirm)?;
    let backend = Backend::sftp(session, socket)?;
    let home = match &backend {
        Backend::Sftp { sftp, .. } => sftp
            .realpath(Path::new("."))
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| "/".into()),
        Backend::Ftp(_) => "/".into(),
    };
    Ok((backend, home, accepted))
}

pub(super) fn tls_connector(profile: &Profile) -> Result<suppaftp::NativeTlsConnector, String> {
    let connector = native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(profile.insecure_tls)
        .danger_accept_invalid_hostnames(profile.insecure_tls)
        .build()
        .map_err(|e| format!("TLS: {e}"))?;
    Ok(suppaftp::NativeTlsConnector::from(connector))
}

pub(super) fn connect_ftp(profile: &Profile) -> Result<(Backend, String), String> {
    let mut ftp = if profile.protocol == Protocol::FtpsImplicit {
        let address = profile
            .address()
            .to_socket_addrs()
            .map_err(|e| tf!("не удалось найти хост: {e}", "could not resolve the host: {e}"))?
            .next()
            .ok_or(tr!("не удалось найти хост"))?;
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
        .map_err(|e| tf!("защищённое соединение не установлено: {e}", "secure connection not established: {e}"))?;
        trace::server("[FTPS] TLS session established");
        ftp
    } else {
        let stream = tcp(profile)?;
        stream
            .set_read_timeout(Some(Duration::from_millis(IO_TIMEOUT_MS as u64)))
            .ok();
        let mut ftp = suppaftp::NativeTlsFtpStream::connect_with_stream(stream)
            .map_err(|e| tf!("ошибка FTP: {e}", "FTP error: {e}"))?;
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
                .map_err(|e| tf!("защищённое соединение не установлено: {e}", "secure connection not established: {e}"))?;
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
        .map_err(|e| tf!("вход не выполнен: {e}", "login failed: {e}"))?;
    ftp.transfer_type(suppaftp::types::FileType::Binary)
        .map_err(|e| e.to_string())?;
    let home = ftp.pwd().unwrap_or_else(|_| "/".into());
    Ok((Backend::Ftp(Box::new(ftp)), home))
}

pub(super) const CHUNK: usize = 64 * 1024;

pub(super) struct Counting<'a> {
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
    pub(super) fn sftp(session: ssh2::Session, socket: Option<TcpStream>) -> Result<Backend, String> {
        let window = session
            .channel_session()
            .ok()
            .map(|mut channel| {
                let window = channel.write_window().remaining as usize;
                let _ = channel.close();
                window
            });
        let chunk = match window {
            Some(window) if window / 3 > CHUNK + 512 => CHUNK,
            Some(window) => (window / 3).saturating_sub(512).max(1024),
            None => 16 * 1024,
        };
        trace::client(format!("[SFTP] write window {}, chunk {chunk} bytes", window.map_or("?".into(), |w| w.to_string())));
        let sftp = session
            .sftp()
            .map_err(|e| tf!("SFTP недоступен: {e}", "SFTP is not available: {e}"))?;
        Ok(Backend::Sftp {
            session,
            sftp,
            socket,
            chunk,
        })
    }

    pub(super) fn list(&mut self, path: &str) -> Result<Vec<Entry>, String> {
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
                        link: stat.file_type().is_symlink(),
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
                    link: f.is_symlink(),
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

    pub(super) fn download(&mut self, path: &str, progress: &dyn Fn(u64, u64)) -> Result<Vec<u8>, String> {
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

    pub(super) fn write(
        &mut self,
        path: &str,
        data: &[u8],
        progress: &dyn Fn(u64, u64),
    ) -> Result<(), String> {
        let total = data.len() as u64;
        match self {
            Backend::Sftp { sftp, chunk, .. } => {
                let mut file = sftp.create(Path::new(path)).map_err(|e| e.to_string())?;
                let mut done = 0;
                for chunk in data.chunks(*chunk) {
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

    pub(super) fn upload(
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

    pub(super) fn walk(
        &mut self,
        root: &str,
        files: &mut Vec<(String, u64)>,
        times: &mut Vec<Option<u64>>,
    ) -> Result<(), String> {
        for entry in self.list(root)? {
            let path = join(root, &entry.name);
            if entry.is_dir && entry.link {
                continue;
            }
            if entry.is_dir {
                self.walk(&path, files, times)?;
            } else {
                files.push((path, entry.size));
                times.push(entry.mtime);
            }
        }
        Ok(())
    }

    pub(super) fn mkdir(&mut self, path: &str) -> Result<(), String> {
        match self {
            Backend::Sftp { sftp, .. } => sftp
                .mkdir(Path::new(path), 0o755)
                .map_err(|e| e.to_string()),
            Backend::Ftp(ftp) => ftp.mkdir(path).map_err(|e| e.to_string()),
        }
    }

    pub(super) fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        match self {
            Backend::Sftp { sftp, .. } => sftp
                .rename(Path::new(from), Path::new(to), None)
                .map_err(|e| e.to_string()),
            Backend::Ftp(ftp) => ftp.rename(from, to).map_err(|e| e.to_string()),
        }
    }

    pub(super) fn delete(&mut self, path: &str, dir: bool) -> Result<(), String> {
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

    pub(super) fn is_link(&mut self, path: &str) -> bool {
        match self {
            Backend::Sftp { sftp, .. } => sftp
                .lstat(Path::new(path))
                .is_ok_and(|s| s.file_type().is_symlink()),
            Backend::Ftp(_) => false,
        }
    }

    pub(super) fn delete_tree(&mut self, path: &str, dir: bool, removed: &mut usize) -> Result<(), String> {
        let dir = dir && !self.is_link(path);
        if dir {
            for entry in self.list(path)? {
                self.delete_tree(&join(path, &entry.name), entry.is_dir && !entry.link, removed)?;
            }
        }
        self.delete(path, dir)?;
        *removed += 1;
        Ok(())
    }

    pub(super) fn stat(&mut self, path: &str) -> Result<Attrs, String> {
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
                    .ok_or(tr!("объект не найден"))?;
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

    pub(super) fn chmod_one(&mut self, path: &str, mode: u32) -> Result<(), String> {
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

    pub(super) fn chmod(
        &mut self,
        path: &str,
        mode: u32,
        recursive: bool,
        dirs: bool,
        files: bool,
        changed: &mut usize,
    ) -> Result<(), String> {
        let is_dir = self.stat(path).map(|a| a.is_dir).unwrap_or(false);
        let recursive = recursive && !self.is_link(path);
        if !recursive || (is_dir && dirs) || (!is_dir && files) {
            self.chmod_one(path, mode)?;
            *changed += 1;
        }
        if recursive && is_dir {
            for entry in self.list(path)? {
                if entry.link {
                    continue;
                }
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

    pub(super) fn extract(&mut self, path: &str, dest: Option<&str>) -> Result<(), String> {
        let command = extract_command(path, dest).ok_or(tr!("неизвестный формат архива"))?;
        self.exec(&command, tr!("распаковка"))
    }

    pub(super) fn exec(&mut self, command: &str, what: &str) -> Result<(), String> {
        let Backend::Sftp { session, .. } = self else {
            return Err(tf!("{what}: доступно только по SFTP", "{what}: available over SFTP only"));
        };
        let mut channel = session.channel_session().map_err(|e| e.to_string())?;
        channel.exec(command).map_err(|e| e.to_string())?;
        let _ = channel.send_eof();
        let mut out = String::new();
        let _ = channel.read_to_string(&mut out);
        let mut err = String::new();
        let _ = channel.stderr().read_to_string(&mut err);
        let _ = channel.wait_close();
        match channel.exit_status() {
            Ok(0) => Ok(()),
            Ok(code) => {
                let text = err.trim();
                Err(if text.is_empty() {
                    tf!("код завершения {code}", "exit code {code}")
                } else {
                    text.to_owned()
                })
            }
            Err(e) => Err(e.to_string()),
        }
    }

    pub(super) fn touch(&mut self, path: &str, mtime: u64) -> Result<(), String> {
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

    pub(super) fn socket(&self) -> Option<TcpStream> {
        match self {
            Backend::Sftp { socket, .. } => socket.as_ref().and_then(|s| s.try_clone().ok()),
            Backend::Ftp(ftp) => ftp.get_ref().try_clone().ok(),
        }
    }

    pub(super) fn abandon(&self) {
        if let Backend::Sftp { session, .. } = self {
            session.set_timeout(1);
        }
    }

    pub(super) fn close(&mut self) {
        match self {
            Backend::Sftp { session, .. } => {
                session.set_timeout(3000);
                let _ = session.disconnect(None, "bye", None);
            }
            Backend::Ftp(ftp) => {
                let _ = ftp.quit();
            }
        }
    }

    pub(super) fn ping(&mut self) -> Result<(), String> {
        match self {
            Backend::Sftp { sftp, .. } => sftp
                .realpath(Path::new("."))
                .map(|_| ())
                .map_err(|e| e.to_string()),
            Backend::Ftp(ftp) => ftp.noop().map_err(|e| e.to_string()),
        }
    }
}

pub(super) fn ftp_mode(file: &suppaftp::list::File) -> Option<u32> {
    use suppaftp::list::PosixPexQuery as Who;
    let bits = |who: Who| {
        (file.can_read(who) as u32) << 2
            | (file.can_write(who) as u32) << 1
            | file.can_execute(who) as u32
    };
    Some(bits(Who::Owner) << 6 | bits(Who::Group) << 3 | bits(Who::Others))
}
