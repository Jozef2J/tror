use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crate::remote::{Shell, ShellCmd, ShellEvent, TermProfile};
use crate::trace;

const IAC: u8 = 255;
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;
const SE: u8 = 240;
const ECHO: u8 = 1;
const SGA: u8 = 3;
const TTYPE: u8 = 24;
const NAWS: u8 = 31;
const BINARY: u8 = 0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum State {
    Data,
    Iac,
    Verb(u8),
    Sub,
    SubIac,
}

pub struct Telnet {
    state: State,
    sub: Vec<u8>,
    naws: bool,
    size: (u16, u16),
    agreed: Vec<(u8, u8)>,
}

fn option_name(option: u8) -> String {
    match option {
        BINARY => "BINARY".into(),
        ECHO => "ECHO".into(),
        SGA => "SUPPRESS-GO-AHEAD".into(),
        TTYPE => "TERMINAL-TYPE".into(),
        NAWS => "NAWS".into(),
        32 => "TERMINAL-SPEED".into(),
        33 => "REMOTE-FLOW-CONTROL".into(),
        34 => "LINEMODE".into(),
        36 => "ENVIRON".into(),
        39 => "NEW-ENVIRON".into(),
        other => other.to_string(),
    }
}

fn verb_name(verb: u8) -> &'static str {
    match verb {
        WILL => "WILL",
        WONT => "WONT",
        DO => "DO",
        _ => "DONT",
    }
}

impl Telnet {
    pub fn new(size: (u16, u16)) -> Self {
        Self {
            state: State::Data,
            sub: Vec::new(),
            naws: false,
            size,
            agreed: Vec::new(),
        }
    }

    fn naws_bytes(&self) -> Vec<u8> {
        let mut out = vec![IAC, SB, NAWS];
        for value in [self.size.0, self.size.1] {
            for byte in value.to_be_bytes() {
                out.push(byte);
                if byte == IAC {
                    out.push(IAC);
                }
            }
        }
        out.extend([IAC, SE]);
        out
    }

    pub fn resize(&mut self, cols: u16, rows: u16) -> Option<Vec<u8>> {
        self.size = (cols, rows);
        self.naws.then(|| self.naws_bytes())
    }

    fn answer(&mut self, verb: u8, option: u8, reply: &mut Vec<u8>) {
        trace::server(format!("[TELNET] < {} {}", verb_name(verb), option_name(option)));
        let response = match verb {
            DO => match option {
                TTYPE | NAWS | SGA | BINARY => WILL,
                _ => WONT,
            },
            DONT => WONT,
            WILL => match option {
                ECHO | SGA | BINARY => DO,
                _ => DONT,
            },
            _ => DONT,
        };
        if self.agreed.contains(&(response, option)) {
            return;
        }
        self.agreed.retain(|(_, o)| *o != option);
        self.agreed.push((response, option));
        trace::client(format!(
            "[TELNET] > {} {}",
            verb_name(response),
            option_name(option)
        ));
        reply.extend([IAC, response, option]);
        if option == NAWS && verb == DO {
            self.naws = true;
            reply.extend(self.naws_bytes());
        }
        if option == NAWS && verb == DONT {
            self.naws = false;
        }
    }

    pub fn feed(&mut self, input: &[u8], data: &mut Vec<u8>, reply: &mut Vec<u8>) {
        for &byte in input {
            self.state = match (self.state, byte) {
                (State::Data, IAC) => State::Iac,
                (State::Data, b) => {
                    data.push(b);
                    State::Data
                }
                (State::Iac, IAC) => {
                    data.push(IAC);
                    State::Data
                }
                (State::Iac, v @ (WILL | WONT | DO | DONT)) => State::Verb(v),
                (State::Iac, SB) => {
                    self.sub.clear();
                    State::Sub
                }
                (State::Iac, _) => State::Data,
                (State::Verb(verb), option) => {
                    self.answer(verb, option, reply);
                    State::Data
                }
                (State::Sub, IAC) => State::SubIac,
                (State::Sub, b) => {
                    if self.sub.len() < 512 {
                        self.sub.push(b);
                    }
                    State::Sub
                }
                (State::SubIac, SE) => {
                    if self.sub.first() == Some(&TTYPE) && self.sub.get(1) == Some(&1) {
                        trace::client("[TELNET] > TERMINAL-TYPE IS XTERM-256COLOR");
                        reply.extend([IAC, SB, TTYPE, 0]);
                        reply.extend_from_slice(b"XTERM-256COLOR");
                        reply.extend([IAC, SE]);
                    }
                    State::Data
                }
                (State::SubIac, IAC) => {
                    self.sub.push(IAC);
                    State::Sub
                }
                (State::SubIac, _) => State::Sub,
            };
        }
    }
}

pub fn escape(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() + 2);
    for &byte in input {
        match byte {
            IAC => out.extend([IAC, IAC]),
            b'\r' => out.extend(b"\r\n"),
            b => out.push(b),
        }
    }
    out
}

fn run(
    profile: &TermProfile,
    size: (u16, u16),
    cmds: &Receiver<ShellCmd>,
    events: &Sender<ShellEvent>,
    wake: &dyn Fn(),
) -> Result<String, String> {
    let address = profile
        .address()
        .to_socket_addrs()
        .map_err(|e| format!("не удалось найти хост: {e}"))?
        .next()
        .ok_or("не удалось найти хост")?;
    trace::line(format!("[TELNET] connecting to {address}"), false);
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(12))
        .map_err(|e| format!("нет соединения: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_millis(15)))
        .map_err(|e| e.to_string())?;
    let _ = stream.set_nodelay(true);
    trace::server("[TELNET] connected (traffic is not encrypted)");
    let _ = events.send(ShellEvent::Connected { fingerprint: None });
    wake();
    let mut telnet = Telnet::new(size);
    let mut buf = [0u8; 16 * 1024];
    let started = Instant::now();
    let mut seen = String::new();
    let mut sent_user = profile.user.trim().is_empty();
    let mut sent_password = profile.password.is_empty();
    loop {
        match stream.read(&mut buf) {
            Ok(0) => return Ok("соединение закрыто сервером".into()),
            Ok(n) => {
                let mut data = Vec::new();
                let mut reply = Vec::new();
                telnet.feed(&buf[..n], &mut data, &mut reply);
                if !reply.is_empty() {
                    stream.write_all(&reply).map_err(|e| e.to_string())?;
                }
                if !data.is_empty() {
                    if (!sent_user || !sent_password) && started.elapsed().as_secs() < 30 {
                        seen.push_str(&String::from_utf8_lossy(&data).to_lowercase());
                        if seen.len() > 400 {
                            let cut = seen.len() - 200;
                            let cut = (cut..seen.len())
                                .find(|i| seen.is_char_boundary(*i))
                                .unwrap_or(0);
                            seen.drain(..cut);
                        }
                        let tail = seen.trim_end();
                        if !sent_user
                            && (tail.ends_with("login:")
                                || tail.ends_with("username:")
                                || tail.ends_with("user name:")
                                || tail.ends_with("user:"))
                        {
                            sent_user = true;
                            seen.clear();
                            trace::client("[TELNET] > (имя пользователя)");
                            stream
                                .write_all(&escape(format!("{}\r", profile.user.trim()).as_bytes()))
                                .map_err(|e| e.to_string())?;
                        } else if !sent_password && tail.ends_with("password:") {
                            sent_password = true;
                            sent_user = true;
                            seen.clear();
                            trace::client("[TELNET] > (пароль)");
                            stream
                                .write_all(&escape(format!("{}\r", profile.password).as_bytes()))
                                .map_err(|e| e.to_string())?;
                        }
                    }
                    let _ = events.send(ShellEvent::Output(data));
                    wake();
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) => return Err(format!("соединение прервано: {e}")),
        }
        loop {
            match cmds.try_recv() {
                Ok(ShellCmd::Input(bytes)) => {
                    stream.write_all(&escape(&bytes)).map_err(|e| e.to_string())?;
                }
                Ok(ShellCmd::Resize(cols, rows)) => {
                    if let Some(bytes) = telnet.resize(cols, rows) {
                        stream.write_all(&bytes).map_err(|e| e.to_string())?;
                    }
                }
                Ok(ShellCmd::TrustHost(_)) => {}
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return Ok("отключено".into());
                }
            }
        }
    }
}

pub fn open(profile: &TermProfile, size: (u16, u16), wake: impl Fn() + Send + 'static) -> Shell {
    let (tx, cmds) = channel();
    let (events, rx) = channel();
    let profile = profile.clone();
    std::thread::spawn(move || {
        let message = run(&profile, size, &cmds, &events, &wake).unwrap_or_else(|e| e);
        trace::line(format!("[TELNET] {message}"), false);
        let _ = events.send(ShellEvent::Closed(message));
        wake();
    });
    Shell { tx, rx }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiation_answers_and_strips_commands() {
        let mut t = Telnet::new((100, 40));
        let mut data = Vec::new();
        let mut reply = Vec::new();
        t.feed(
            &[
                b'h', IAC, DO, TTYPE, IAC, WILL, ECHO, b'i', IAC, IAC, IAC, DO, NAWS, IAC, DO, 39,
            ],
            &mut data,
            &mut reply,
        );
        assert_eq!(data, vec![b'h', b'i', IAC]);
        assert_eq!(
            reply,
            vec![
                IAC, WILL, TTYPE, IAC, DO, ECHO, IAC, WILL, NAWS, IAC, SB, NAWS, 0, 100, 0, 40,
                IAC, SE, IAC, WONT, 39
            ]
        );
        reply.clear();
        t.feed(&[IAC, DO, TTYPE], &mut data, &mut reply);
        assert!(reply.is_empty());
        t.feed(&[IAC, SB, TTYPE, 1, IAC, SE], &mut data, &mut reply);
        assert!(reply.starts_with(&[IAC, SB, TTYPE, 0, b'X']));
        assert_eq!(
            t.resize(255, 50),
            Some(vec![IAC, SB, NAWS, 0, 255, 255, 0, 50, IAC, SE])
        );
        assert_eq!(escape(b"a\r\xff"), b"a\r\n\xff\xff");
    }
}

#[cfg(test)]
mod live {
    use super::*;

    #[test]
    #[ignore]
    fn live_telnet_login_and_command() {
        let profile = TermProfile {
            name: "router".into(),
            protocol: crate::remote::TermProtocol::Telnet,
            host: "127.0.0.1".into(),
            port: 2323,
            user: "admin".into(),
            password: "cisco".into(),
            ..Default::default()
        };
        let shell = open(&profile, (90, 30), || {});
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut output = Vec::new();
        let mut sent = false;
        while Instant::now() < deadline {
            match shell.rx.recv_timeout(Duration::from_millis(200)) {
                Ok(ShellEvent::Output(bytes)) => output.extend(bytes),
                Ok(ShellEvent::Closed(e)) => panic!("closed: {e}: {}", String::from_utf8_lossy(&output)),
                _ => {}
            }
            let text = String::from_utf8_lossy(&output).into_owned();
            if !sent && text.contains("router#") {
                assert!(text.contains("Welcome admin"), "{text}");
                shell
                    .tx
                    .send(ShellCmd::Input(b"echo tror-$((6*7))\r".to_vec()))
                    .unwrap();
                sent = true;
            }
            if sent && text.contains("tror-42") {
                return;
            }
        }
        panic!("{}", String::from_utf8_lossy(&output));
    }
}
