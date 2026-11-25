use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::channel;

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

use crate::remote::{Shell, ShellCmd, ShellEvent};

pub fn program() -> String {
    if cfg!(windows) {
        let pwsh = ["pwsh.exe", "powershell.exe"].into_iter().find(|name| {
            std::env::var_os("PATH").is_some_and(|path| {
                std::env::split_paths(&path).any(|dir| dir.join(name).is_file())
            })
        });
        pwsh.map(str::to_owned)
            .or_else(|| std::env::var("COMSPEC").ok())
            .unwrap_or_else(|| "cmd.exe".into())
    } else {
        std::env::var("SHELL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "/bin/sh".into())
    }
}

pub fn open(
    cwd: Option<PathBuf>,
    size: (u16, u16),
    wake: impl Fn() + Send + Sync + 'static,
) -> Shell {
    let (tx, cmds) = channel::<ShellCmd>();
    let (events, rx) = channel();
    let size = PtySize {
        cols: size.0.max(2),
        rows: size.1.max(2),
        pixel_width: 0,
        pixel_height: 0,
    };
    let started = native_pty_system().openpty(size).and_then(|pair| {
        let mut command = CommandBuilder::new(program());
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        if let Some(dir) = cwd
            .filter(|d| d.is_dir())
            .or_else(|| std::env::home_dir().filter(|d| d.is_dir()))
        {
            command.cwd(dir);
        }
        let child = pair.slave.spawn_command(command)?;
        drop(pair.slave);
        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        Ok((pair.master, child, reader, writer))
    });
    match started {
        Err(e) => {
            let _ = events.send(ShellEvent::Closed(tf!(
                "не удалось запустить {}: {e}",
                "could not start {}: {e}",
                program()
            )));
            wake();
        }
        Ok((master, mut child, mut reader, mut writer)) => {
            let _ = events.send(ShellEvent::Connected { fingerprint: None });
            let out = events.clone();
            let wake = std::sync::Arc::new(wake);
            let wake_out = wake.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 16384];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if out.send(ShellEvent::Output(buf[..n].to_vec())).is_err() {
                                return;
                            }
                            wake_out();
                        }
                    }
                }
                let _ = out.send(ShellEvent::Closed(tr!("сеанс завершён").into()));
                wake_out();
            });
            std::thread::spawn(move || {
                while let Ok(cmd) = cmds.recv() {
                    match cmd {
                        ShellCmd::Input(data) => {
                            if writer
                                .write_all(&data)
                                .and_then(|_| writer.flush())
                                .is_err()
                            {
                                break;
                            }
                        }
                        ShellCmd::Resize(cols, rows) => {
                            let _ = master.resize(PtySize {
                                cols: cols.max(2),
                                rows: rows.max(2),
                                pixel_width: 0,
                                pixel_height: 0,
                            });
                        }
                        ShellCmd::TrustHost(_) => {}
                    }
                }
                let _ = child.kill();
                let _ = child.wait();
                wake();
            });
        }
    }
    Shell { tx, rx }
}
