use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use aes::Aes128;
use ocb3::aead::{AeadInOut, KeyInit};
use ocb3::{Nonce, Ocb3, consts::U12};

use crate::remote::{Profile, Shell, ShellCmd, ShellEvent, TermProfile};
use crate::trace;

const PROTOCOL_VERSION: u64 = 2;
const DIRECTION: u64 = 1 << 63;
const CHUNK: usize = 1000;
const TICK: Duration = Duration::from_millis(20);
const HEARTBEAT: Duration = Duration::from_millis(3000);
const RESEND: Duration = Duration::from_millis(300);
const SILENT: Duration = Duration::from_secs(8);

type Cipher = Ocb3<Aes128, U12>;

pub struct Crypto {
    cipher: Cipher,
}

impl Crypto {
    pub fn new(key: &str) -> Result<Self, String> {
        use base64::Engine;
        let mut text = key.trim().to_owned();
        while !text.len().is_multiple_of(4) {
            text.push('=');
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(text)
            .map_err(|_| "сервер прислал неверный ключ сеанса")?;
        let key: [u8; 16] = bytes
            .try_into()
            .map_err(|_| "сервер прислал ключ сеанса неверной длины")?;
        Ok(Self {
            cipher: Cipher::new(&key.into()),
        })
    }

    fn nonce(value: u64) -> Nonce<U12> {
        let mut nonce = [0u8; 12];
        nonce[4..].copy_from_slice(&value.to_be_bytes());
        nonce.into()
    }

    pub fn seal(&self, value: u64, plain: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(plain.len() + 24);
        out.extend_from_slice(&value.to_be_bytes());
        let mut body = plain.to_vec();
        let tag = self
            .cipher
            .encrypt_inout_detached(&Self::nonce(value), &[], body.as_mut_slice().into())
            .expect("OCB");
        out.extend_from_slice(&body);
        out.extend_from_slice(&tag);
        out
    }

    pub fn open(&self, packet: &[u8]) -> Option<(u64, Vec<u8>)> {
        if packet.len() < 24 {
            return None;
        }
        let value = u64::from_be_bytes(packet[..8].try_into().ok()?);
        let (body, tag) = packet[8..].split_at(packet.len() - 24);
        let mut body = body.to_vec();
        let tag: [u8; 16] = tag.try_into().ok()?;
        self.cipher
            .decrypt_inout_detached(
                &Self::nonce(value),
                &[],
                body.as_mut_slice().into(),
                &tag.into(),
            )
            .ok()?;
        Some((value, body))
    }
}

fn varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn put_varint(out: &mut Vec<u8>, field: u64, value: u64) {
    varint(out, field << 3);
    varint(out, value);
}

fn put_bytes(out: &mut Vec<u8>, field: u64, value: &[u8]) {
    varint(out, field << 3 | 2);
    varint(out, value.len() as u64);
    out.extend_from_slice(value);
}

enum Value<'a> {
    Int(u64),
    Bytes(&'a [u8]),
}

fn fields(mut data: &[u8]) -> Option<Vec<(u64, Value<'_>)>> {
    fn read(data: &mut &[u8]) -> Option<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let (&byte, rest) = data.split_first()?;
            *data = rest;
            value |= ((byte & 0x7f) as u64) << shift;
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }
    let mut out = Vec::new();
    while !data.is_empty() {
        let key = read(&mut data)?;
        let field = key >> 3;
        match key & 7 {
            0 => out.push((field, Value::Int(read(&mut data)?))),
            1 => data = data.get(8..)?,
            2 => {
                let len = read(&mut data)? as usize;
                let (value, rest) = (data.get(..len)?, data.get(len..)?);
                data = rest;
                out.push((field, Value::Bytes(value)));
            }
            5 => data = data.get(4..)?,
            _ => return None,
        }
    }
    Some(out)
}

#[derive(Default, Debug, PartialEq)]
pub struct Instruction {
    pub old_num: u64,
    pub new_num: u64,
    pub ack_num: u64,
    pub throwaway_num: u64,
    pub diff: Vec<u8>,
}

impl Instruction {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_varint(&mut out, 1, PROTOCOL_VERSION);
        put_varint(&mut out, 2, self.old_num);
        put_varint(&mut out, 3, self.new_num);
        put_varint(&mut out, 4, self.ack_num);
        put_varint(&mut out, 5, self.throwaway_num);
        put_bytes(&mut out, 6, &self.diff);
        out
    }

    pub fn decode(data: &[u8]) -> Option<Self> {
        let mut inst = Self::default();
        let mut version = 0;
        for (field, value) in fields(data)? {
            match (field, value) {
                (1, Value::Int(v)) => version = v,
                (2, Value::Int(v)) => inst.old_num = v,
                (3, Value::Int(v)) => inst.new_num = v,
                (4, Value::Int(v)) => inst.ack_num = v,
                (5, Value::Int(v)) => inst.throwaway_num = v,
                (6, Value::Bytes(v)) => inst.diff = v.to_vec(),
                _ => {}
            }
        }
        (version == PROTOCOL_VERSION).then_some(inst)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum UserEvent {
    Keys(Vec<u8>),
    Resize(u16, u16),
}

pub fn user_diff(events: &[UserEvent]) -> Vec<u8> {
    let mut out = Vec::new();
    for event in events {
        let mut inner = Vec::new();
        match event {
            UserEvent::Keys(keys) => {
                let mut keystroke = Vec::new();
                put_bytes(&mut keystroke, 4, keys);
                put_bytes(&mut inner, 2, &keystroke);
            }
            UserEvent::Resize(cols, rows) => {
                let mut resize = Vec::new();
                put_varint(&mut resize, 5, *cols as u64);
                put_varint(&mut resize, 6, *rows as u64);
                put_bytes(&mut inner, 3, &resize);
            }
        }
        put_bytes(&mut out, 1, &inner);
    }
    out
}

pub fn host_bytes(diff: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    for (field, value) in fields(diff)? {
        let (1, Value::Bytes(inst)) = (field, value) else {
            continue;
        };
        for (field, value) in fields(inst)? {
            if let (2, Value::Bytes(host)) = (field, value) {
                for (field, value) in fields(host)? {
                    if let (4, Value::Bytes(bytes)) = (field, value) {
                        out.extend_from_slice(bytes);
                    }
                }
            }
        }
    }
    Some(out)
}

fn compress(data: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = encoder.write_all(data);
    encoder.finish().unwrap_or_default()
}

fn decompress(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .take(64 * 1024 * 1024)
        .read_to_end(&mut out)
        .ok()?;
    Some(out)
}

pub fn fragments(id: u64, instruction: &Instruction) -> Vec<Vec<u8>> {
    let payload = compress(&instruction.encode());
    let chunks: Vec<&[u8]> = if payload.is_empty() {
        vec![&[]]
    } else {
        payload.chunks(CHUNK).collect()
    };
    let last = chunks.len() - 1;
    chunks
        .into_iter()
        .enumerate()
        .map(|(i, chunk)| {
            let mut out = Vec::with_capacity(chunk.len() + 10);
            out.extend_from_slice(&id.to_be_bytes());
            let num = i as u16 | if i == last { 0x8000 } else { 0 };
            out.extend_from_slice(&num.to_be_bytes());
            out.extend_from_slice(chunk);
            out
        })
        .collect()
}

#[derive(Default)]
pub struct Assembly {
    id: Option<u64>,
    parts: BTreeMap<u16, Vec<u8>>,
    last: Option<u16>,
}

impl Assembly {
    pub fn add(&mut self, fragment: &[u8]) -> Option<Instruction> {
        if fragment.len() < 10 {
            return None;
        }
        let id = u64::from_be_bytes(fragment[..8].try_into().ok()?);
        let raw = u16::from_be_bytes([fragment[8], fragment[9]]);
        let (num, last) = (raw & 0x7fff, raw & 0x8000 != 0);
        if self.id != Some(id) {
            self.id = Some(id);
            self.parts.clear();
            self.last = None;
        }
        self.parts.insert(num, fragment[10..].to_vec());
        if last {
            self.last = Some(num);
        }
        let last = self.last?;
        if self.parts.len() != last as usize + 1 {
            return None;
        }
        let payload: Vec<u8> = self.parts.values().flatten().copied().collect();
        self.parts.clear();
        self.last = None;
        Instruction::decode(&decompress(&payload)?)
    }
}

pub struct Session {
    crypto: Crypto,
    seq: u64,
    events: Vec<UserEvent>,
    states: Vec<(u64, usize)>,
    acked: (u64, usize),
    remote: u64,
    instruction_id: u64,
    last_sent: Option<Instant>,
    sent_ack: u64,
    assembly: Assembly,
    their_stamp: Option<(u16, Instant)>,
    expected: u64,
    started: Instant,
}

impl Session {
    pub fn new(crypto: Crypto) -> Self {
        Self {
            crypto,
            seq: 0,
            events: Vec::new(),
            states: vec![(0, 0)],
            acked: (0, 0),
            remote: 0,
            instruction_id: 0,
            last_sent: None,
            sent_ack: 0,
            assembly: Assembly::default(),
            their_stamp: None,
            expected: 0,
            started: Instant::now(),
        }
    }

    pub fn push(&mut self, event: UserEvent) {
        let unsent = self.states.last().is_some_and(|s| s.1 < self.events.len());
        if let (Some(UserEvent::Keys(previous)), UserEvent::Keys(keys)) =
            (self.events.last_mut(), &event)
            && unsent
        {
            previous.extend_from_slice(keys);
            return;
        }
        self.events.push(event);
    }

    fn stamp(&self) -> u16 {
        (self.started.elapsed().as_millis() & 0xffff) as u16
    }

    fn packet(&mut self, payload: &[u8]) -> Vec<u8> {
        let mut plain = Vec::with_capacity(payload.len() + 4);
        plain.extend_from_slice(&self.stamp().to_be_bytes());
        let reply = match self.their_stamp.take() {
            Some((stamp, at)) if at.elapsed() < Duration::from_secs(1) => {
                stamp.wrapping_add(at.elapsed().as_millis() as u16)
            }
            _ => u16::MAX,
        };
        plain.extend_from_slice(&reply.to_be_bytes());
        plain.extend_from_slice(payload);
        let value = self.seq & !DIRECTION;
        self.seq += 1;
        self.crypto.seal(value, &plain)
    }

    pub fn outgoing(&mut self, now: Instant) -> Vec<Vec<u8>> {
        let last = *self.states.last().unwrap_or(&(0, 0));
        let fresh = self.events.len() > last.1;
        let unacked = last.0 > self.acked.0;
        let quiet = self.last_sent.is_none_or(|t| now.duration_since(t) >= HEARTBEAT);
        let resend = unacked && self.last_sent.is_none_or(|t| now.duration_since(t) >= RESEND);
        let ack = self.remote != self.sent_ack;
        if !(fresh || resend || ack || quiet) {
            return Vec::new();
        }
        let target = if fresh {
            let state = (last.0 + 1, self.events.len());
            self.states.push(state);
            state
        } else {
            last
        };
        let instruction = Instruction {
            old_num: self.acked.0,
            new_num: target.0,
            ack_num: self.remote,
            throwaway_num: self.acked.0,
            diff: user_diff(&self.events[self.acked.1..target.1]),
        };
        self.sent_ack = self.remote;
        self.last_sent = Some(now);
        self.instruction_id += 1;
        let id = self.instruction_id;
        fragments(id, &instruction)
            .into_iter()
            .map(|fragment| self.packet(&fragment))
            .collect()
    }

    pub fn incoming(&mut self, packet: &[u8]) -> Option<Vec<u8>> {
        let (value, plain) = self.crypto.open(packet)?;
        if value & DIRECTION == 0 || plain.len() < 4 {
            return None;
        }
        let seq = value & !DIRECTION;
        if seq < self.expected {
            return None;
        }
        self.expected = seq + 1;
        let stamp = u16::from_be_bytes([plain[0], plain[1]]);
        self.their_stamp = Some((stamp, Instant::now()));
        let inst = self.assembly.add(&plain[4..])?;
        if let Some(state) = self.states.iter().find(|s| s.0 == inst.ack_num).copied() {
            if state.0 >= self.acked.0 {
                self.acked = state;
                self.states.retain(|s| s.0 >= state.0);
            }
        }
        if inst.new_num <= self.remote || inst.old_num != self.remote {
            return None;
        }
        self.remote = inst.new_num;
        if inst.diff.is_empty() {
            return Some(Vec::new());
        }
        host_bytes(&inst.diff)
    }

    #[cfg(test)]
    pub fn acked_everything(&self) -> bool {
        self.states.last().is_some_and(|s| s.0 == self.acked.0)
    }
}

pub fn parse_connect(text: &str) -> Option<(u16, String)> {
    text.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("MOSH CONNECT ")?;
        let mut parts = rest.split_whitespace();
        let port = parts.next()?.parse().ok()?;
        let key = parts.next()?.to_owned();
        (key.len() == 22).then_some((port, key))
    })
}

fn bootstrap(
    profile: &TermProfile,
    known: Option<&str>,
    size: (u16, u16),
    cmds: &Receiver<ShellCmd>,
    events: &Sender<ShellEvent>,
    wake: &dyn Fn(),
) -> Result<(u16, String, Option<String>), String> {
    let ssh: Profile = profile.ssh();
    let confirm = |fingerprint: &str, changed: bool| {
        let _ = events.send(ShellEvent::HostKey {
            fingerprint: fingerprint.to_owned(),
            changed,
        });
        wake();
        matches!(cmds.recv(), Ok(ShellCmd::TrustHost(true)))
    };
    let (session, fingerprint) = crate::remote::open_session(&ssh, known, &confirm)?;
    let server = if profile.mosh_server.trim().is_empty() {
        "mosh-server"
    } else {
        profile.mosh_server.trim()
    };
    let ports = profile.mosh_ports.trim();
    let ports = if ports.is_empty() {
        String::new()
    } else {
        format!(" -p {ports}")
    };
    let mut last_output = String::new();
    for locale in ["en_US.UTF-8", "C.UTF-8"] {
        let command = format!("{server} new -s -c 256{ports} -l LANG={locale} -l LC_ALL={locale}");
        trace::client(format!("[MOSH] exec: {command}"));
        let _ = events.send(ShellEvent::Status("запуск mosh-server…".into()));
        wake();
        let mut channel = session
            .channel_session()
            .map_err(|e| format!("не удалось открыть канал: {e}"))?;
        channel
            .request_pty("xterm-256color", None, Some((size.0 as u32, size.1 as u32, 0, 0)))
            .map_err(|e| format!("сервер не дал терминал: {e}"))?;
        channel
            .exec(&command)
            .map_err(|e| format!("не удалось запустить mosh-server: {e}"))?;
        let mut output = Vec::new();
        let mut buf = [0u8; 4096];
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            match channel.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    output.extend_from_slice(&buf[..n]);
                    if parse_connect(&String::from_utf8_lossy(&output)).is_some() {
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(format!("mosh-server: {e}")),
            }
            if Instant::now() > deadline {
                break;
            }
        }
        let text = String::from_utf8_lossy(&output).into_owned();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            if line.starts_with("MOSH CONNECT") {
                trace::server("[MOSH] MOSH CONNECT <порт> <ключ>");
            } else {
                trace::server(format!("[MOSH] {}", line.trim()));
            }
        }
        let _ = channel.close();
        if let Some((port, key)) = parse_connect(&text) {
            return Ok((port, key, fingerprint));
        }
        last_output = text;
        if !last_output.contains("UTF-8") && !last_output.contains("locale") {
            break;
        }
    }
    let lower = last_output.to_lowercase();
    Err(
        if lower.contains("not found") || lower.contains("не найден") || lower.contains("no such file") {
            format!(
                "на сервере не найден {server}: установите пакет mosh (apt install mosh / yum install mosh) или укажите путь к mosh-server в профиле"
            )
        } else if lower.contains("utf-8") || lower.contains("locale") {
            "mosh-server не нашёл на сервере локаль UTF-8 (нужна en_US.UTF-8 или C.UTF-8)".into()
        } else {
            let tail: String = last_output.trim().chars().take(300).collect();
            format!("mosh-server не запустился{}", if tail.is_empty() { String::new() } else { format!(": {tail}") })
        },
    )
}

fn run(
    profile: &TermProfile,
    known: Option<&str>,
    size: (u16, u16),
    cmds: &Receiver<ShellCmd>,
    events: &Sender<ShellEvent>,
    wake: &dyn Fn(),
) -> Result<String, String> {
    let (port, key, fingerprint) = bootstrap(profile, known, size, cmds, events, wake)?;
    let ip = profile
        .address()
        .to_socket_addrs()
        .map_err(|e| format!("не удалось найти хост: {e}"))?
        .next()
        .ok_or("не удалось найти хост")?
        .ip();
    let target = SocketAddr::new(ip, port);
    let socket = UdpSocket::bind(if ip.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" })
        .map_err(|e| e.to_string())?;
    socket.connect(target).map_err(|e| e.to_string())?;
    socket.set_read_timeout(Some(TICK)).map_err(|e| e.to_string())?;
    trace::line(format!("[MOSH] UDP session with {target}"), false);
    let mut session = Session::new(Crypto::new(&key)?);
    session.push(UserEvent::Resize(size.0, size.1));
    let _ = events.send(ShellEvent::Connected { fingerprint });
    let _ = events.send(ShellEvent::Status(String::new()));
    wake();
    let mut buf = vec![0u8; 65536];
    let mut heard = Instant::now();
    let mut ever_heard = false;
    let mut warned = false;
    loop {
        loop {
            match cmds.try_recv() {
                Ok(ShellCmd::Input(bytes)) => session.push(UserEvent::Keys(bytes)),
                Ok(ShellCmd::Resize(cols, rows)) => session.push(UserEvent::Resize(cols, rows)),
                Ok(ShellCmd::TrustHost(_)) => {}
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return Ok("отключено (сеанс mosh на сервере завершится сам)".into());
                }
            }
        }
        for packet in session.outgoing(Instant::now()) {
            let _ = socket.send(&packet);
        }
        match socket.recv(&mut buf) {
            Ok(n) => {
                if let Some(bytes) = session.incoming(&buf[..n]) {
                    heard = Instant::now();
                    ever_heard = true;
                    if warned {
                        warned = false;
                        let _ = events.send(ShellEvent::Status(String::new()));
                        wake();
                    }
                    if !bytes.is_empty() {
                        let _ = events.send(ShellEvent::Output(bytes));
                        wake();
                    }
                } else {
                    heard = Instant::now();
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => std::thread::sleep(TICK),
        }
        if heard.elapsed() > SILENT && !warned {
            warned = true;
            let text = if ever_heard {
                format!(
                    "нет связи с сервером {} с — mosh переподключится сам, когда сеть вернётся",
                    heard.elapsed().as_secs()
                )
            } else {
                format!("сервер не отвечает по UDP {port} — проверьте, что порт открыт в файрволе")
            };
            let _ = events.send(ShellEvent::Status(text));
            wake();
        }
    }
}

pub fn open(
    profile: &TermProfile,
    known: Option<String>,
    size: (u16, u16),
    wake: impl Fn() + Send + 'static,
) -> Shell {
    let (tx, cmds) = channel();
    let (events, rx) = channel();
    let profile = profile.clone();
    std::thread::spawn(move || {
        let message =
            run(&profile, known.as_deref(), size, &cmds, &events, &wake).unwrap_or_else(|e| e);
        trace::line(format!("[MOSH] {message}"), false);
        let _ = events.send(ShellEvent::Closed(message));
        wake();
    });
    Shell { tx, rx }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ocb_matches_rfc7253_vector() {
        let cipher = Cipher::new(&[
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
        ]
        .into());
        let nonce: Nonce<U12> = [0xbb, 0xaa, 0x99, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, 0x00].into();
        let mut body: Vec<u8> = Vec::new();
        let tag = cipher
            .encrypt_inout_detached(&nonce, &[], body.as_mut_slice().into())
            .unwrap();
        assert_eq!(
            tag.as_slice(),
            [
                0x78, 0x54, 0x07, 0xbf, 0xff, 0xc8, 0xad, 0x9e, 0xdc, 0xc5, 0x52, 0x0a, 0xc9,
                0x11, 0x1e, 0xe6
            ]
        );
    }

    #[test]
    fn instructions_and_messages_roundtrip() {
        let inst = Instruction {
            old_num: 3,
            new_num: 300,
            ack_num: 7,
            throwaway_num: 3,
            diff: user_diff(&[UserEvent::Keys(b"ls\r".to_vec()), UserEvent::Resize(120, 40)]),
        };
        assert_eq!(Instruction::decode(&inst.encode()), Some(inst));
        let mut host = Vec::new();
        let mut bytes = Vec::new();
        put_bytes(&mut bytes, 4, b"\x1b[Hhello");
        let mut wrapped = Vec::new();
        put_bytes(&mut wrapped, 2, &bytes);
        put_bytes(&mut host, 1, &wrapped);
        let mut echo = Vec::new();
        put_varint(&mut echo, 8, 5);
        let mut wrapped = Vec::new();
        put_bytes(&mut wrapped, 7, &echo);
        put_bytes(&mut host, 1, &wrapped);
        assert_eq!(host_bytes(&host).unwrap(), b"\x1b[Hhello");
        assert_eq!(
            parse_connect("\r\nMOSH CONNECT 60001 4NeCCgvZFe2RnPgrcU1PQw\r\n"),
            Some((60001, "4NeCCgvZFe2RnPgrcU1PQw".into()))
        );
    }

    #[test]
    fn fragments_reassemble_and_sessions_talk() {
        let big = Instruction {
            old_num: 0,
            new_num: 1,
            ack_num: 0,
            throwaway_num: 0,
            diff: (0..20000u32)
                .scan(0x9e3779b9u32, |x, _| {
                    *x ^= *x << 13;
                    *x ^= *x >> 17;
                    *x ^= *x << 5;
                    Some(*x as u8)
                })
                .collect(),
        };
        let parts = fragments(9, &big);
        assert!(parts.len() > 1);
        let mut assembly = Assembly::default();
        let mut result = None;
        for part in parts.iter().rev() {
            result = assembly.add(part).or(result);
        }
        assert_eq!(result, Some(big));

        let key = "4NeCCgvZFe2RnPgrcU1PQw";
        let mut client = Session::new(Crypto::new(key).unwrap());
        let server = Crypto::new(key).unwrap();
        client.push(UserEvent::Keys(b"a".to_vec()));
        client.push(UserEvent::Keys(b"b".to_vec()));
        let sent = client.outgoing(Instant::now());
        assert_eq!(sent.len(), 1);
        let (value, plain) = server.open(&sent[0]).unwrap();
        assert_eq!(value & DIRECTION, 0);
        let inst = Assembly::default().add(&plain[4..]).unwrap();
        assert_eq!((inst.old_num, inst.new_num), (0, 1));
        assert_eq!(inst.diff, user_diff(&[UserEvent::Keys(b"ab".to_vec())]));
        assert!(client.outgoing(Instant::now()).is_empty());

        let mut host = Vec::new();
        let mut bytes = Vec::new();
        put_bytes(&mut bytes, 4, b"hi");
        let mut wrapped = Vec::new();
        put_bytes(&mut wrapped, 2, &bytes);
        put_bytes(&mut host, 1, &wrapped);
        let reply = Instruction {
            old_num: 0,
            new_num: 1,
            ack_num: 1,
            throwaway_num: 0,
            diff: host,
        };
        let mut plain = vec![0, 5, 0xff, 0xff];
        plain.extend(fragments(1, &reply).remove(0));
        let packet = server.seal(DIRECTION, &plain);
        assert_eq!(client.incoming(&packet), Some(b"hi".to_vec()));
        assert!(client.acked_everything());
        assert_eq!(client.incoming(&packet), None);
        let ack = client.outgoing(Instant::now());
        let (_, plain) = server.open(&ack[0]).unwrap();
        let inst = Assembly::default().add(&plain[4..]).unwrap();
        assert_eq!((inst.old_num, inst.new_num, inst.ack_num), (1, 1, 1));
        assert!(inst.diff.is_empty());
    }
}

#[cfg(test)]
mod live {
    use super::*;

    #[test]
    #[ignore]
    fn live_mosh_session() {
        let profile = TermProfile {
            name: "mosh".into(),
            protocol: crate::remote::TermProtocol::Mosh,
            host: "127.0.0.1".into(),
            port: 2222,
            user: "jozef".into(),
            password: "secret".into(),
            ..Default::default()
        };
        let shell = open(&profile, None, (100, 30), || {});
        let deadline = Instant::now() + Duration::from_secs(40);
        let mut output = Vec::new();
        let mut sent = false;
        let mut resized = false;
        while Instant::now() < deadline {
            match shell.rx.recv_timeout(Duration::from_millis(200)) {
                Ok(ShellEvent::HostKey { .. }) => shell.tx.send(ShellCmd::TrustHost(true)).unwrap(),
                Ok(ShellEvent::Connected { .. }) => {}
                Ok(ShellEvent::Output(bytes)) => output.extend(bytes),
                Ok(ShellEvent::Status(_)) => {}
                Ok(ShellEvent::Closed(e)) => panic!("closed: {e}"),
                Err(_) => {}
            }
            let text = String::from_utf8_lossy(&output).into_owned();
            if !sent && (text.contains('$') || text.contains('#')) {
                shell
                    .tx
                    .send(ShellCmd::Input(b"echo tror-$((6*7)); tput cols\r".to_vec()))
                    .unwrap();
                sent = true;
            }
            if sent && !resized && text.contains("tror-42") {
                assert!(text.contains("100"), "{text}");
                shell.tx.send(ShellCmd::Resize(120, 40)).unwrap();
                shell.tx.send(ShellCmd::Input(b"tput cols\r".to_vec())).unwrap();
                resized = true;
            }
            if resized && text.contains("120") {
                return;
            }
        }
        panic!("no echo: {}", String::from_utf8_lossy(&output));
    }
}
