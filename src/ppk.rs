use base64::Engine;
use sha1::Sha1;
use sha2::{Digest, Sha256};

pub fn is_ppk(text: &str) -> bool {
    text.starts_with("PuTTY-User-Key-File-")
}

struct Ppk {
    version: u8,
    kind: String,
    encryption: String,
    comment: String,
    public: Vec<u8>,
    private: Vec<u8>,
    mac: Vec<u8>,
    argon: Option<(argon2::Algorithm, u32, u32, u32, Vec<u8>)>,
}

fn hex(text: &str) -> Result<Vec<u8>, String> {
    let text = text.trim();
    if !text.len().is_multiple_of(2) {
        return Err(tr!("повреждённый ключ PuTTY").into());
    }
    (0..text.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&text[i..i + 2], 16)
                .map_err(|_| tr!("повреждённый ключ PuTTY").to_string())
        })
        .collect()
}

fn parse(text: &str) -> Result<Ppk, String> {
    let bad = || tr!("повреждённый ключ PuTTY").to_string();
    let mut lines = text.lines();
    let mut ppk = Ppk {
        version: 0,
        kind: String::new(),
        encryption: String::new(),
        comment: String::new(),
        public: Vec::new(),
        private: Vec::new(),
        mac: Vec::new(),
        argon: None,
    };
    let mut flavor = None;
    let (mut memory, mut passes, mut lanes, mut salt) = (0, 0, 0, Vec::new());
    let engine = base64::engine::general_purpose::STANDARD;
    while let Some(line) = lines.next() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key {
            "PuTTY-User-Key-File-2" | "PuTTY-User-Key-File-3" => {
                ppk.version = if key.ends_with('3') { 3 } else { 2 };
                ppk.kind = value.to_owned();
            }
            "Encryption" => ppk.encryption = value.to_owned(),
            "Comment" => ppk.comment = value.to_owned(),
            "Public-Lines" | "Private-Lines" => {
                let count: usize = value.parse().map_err(|_| bad())?;
                let body: String = lines.by_ref().take(count).collect();
                let data = engine.decode(body.trim()).map_err(|_| bad())?;
                if key == "Public-Lines" {
                    ppk.public = data;
                } else {
                    ppk.private = data;
                }
            }
            "Private-MAC" => ppk.mac = hex(value)?,
            "Key-Derivation" => {
                flavor = Some(match value {
                    "Argon2id" => argon2::Algorithm::Argon2id,
                    "Argon2i" => argon2::Algorithm::Argon2i,
                    "Argon2d" => argon2::Algorithm::Argon2d,
                    _ => {
                        return Err(tf!(
                            "ключ PuTTY: неизвестная схема {value}",
                            "PuTTY key: unknown scheme {value}"
                        ));
                    }
                })
            }
            "Argon2-Memory" => memory = value.parse().map_err(|_| bad())?,
            "Argon2-Passes" => passes = value.parse().map_err(|_| bad())?,
            "Argon2-Parallelism" => lanes = value.parse().map_err(|_| bad())?,
            "Argon2-Salt" => salt = hex(value)?,
            _ => {}
        }
    }
    if ppk.version == 0 || ppk.public.is_empty() || ppk.private.is_empty() {
        return Err(bad());
    }
    if let Some(flavor) = flavor {
        ppk.argon = Some((flavor, memory, passes, lanes, salt));
    }
    Ok(ppk)
}

fn hmac<D: Digest>(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut block = [0u8; 64];
    if key.len() > 64 {
        let digest = D::digest(key);
        block[..digest.len()].copy_from_slice(&digest);
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = D::new();
    inner.update(block.map(|b| b ^ 0x36));
    inner.update(data);
    let inner = inner.finalize();
    let mut outer = D::new();
    outer.update(block.map(|b| b ^ 0x5c));
    outer.update(inner);
    outer.finalize().to_vec()
}

fn put(out: &mut Vec<u8>, data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(data);
}

fn take<'a>(data: &'a [u8], at: &mut usize) -> Result<&'a [u8], String> {
    let bad = || tr!("повреждённый ключ PuTTY").to_string();
    let len = u32::from_be_bytes(
        data.get(*at..*at + 4)
            .ok_or_else(bad)?
            .try_into()
            .map_err(|_| bad())?,
    ) as usize;
    let value = data.get(*at + 4..*at + 4 + len).ok_or_else(bad)?;
    *at += 4 + len;
    Ok(value)
}

fn decrypt(cipher_key: &[u8], iv: &[u8], data: &mut [u8]) -> Result<(), String> {
    use aes::cipher::{BlockCipherDecrypt, KeyInit};
    if !data.len().is_multiple_of(16) {
        return Err(tr!("повреждённый ключ PuTTY").into());
    }
    let cipher = aes::Aes256::new_from_slice(cipher_key)
        .map_err(|_| tr!("ключ PuTTY: неверная длина ключа"))?;
    let mut prev = [0u8; 16];
    prev.copy_from_slice(iv);
    for chunk in data.chunks_mut(16) {
        let mut saved = [0u8; 16];
        saved.copy_from_slice(chunk);
        let block: &mut aes::Block = chunk
            .try_into()
            .map_err(|_| tr!("повреждённый ключ PuTTY"))?;
        cipher.decrypt_block(block);
        for (b, p) in chunk.iter_mut().zip(prev) {
            *b ^= p;
        }
        prev = saved;
    }
    Ok(())
}

fn unlock(ppk: &mut Ppk, passphrase: &str) -> Result<(), String> {
    let encrypted = ppk.encryption != "none";
    if encrypted && ppk.encryption != "aes256-cbc" {
        return Err(tf!(
            "ключ PuTTY: неизвестное шифрование {}",
            "PuTTY key: unknown encryption {}",
            ppk.encryption
        ));
    }
    if encrypted && passphrase.is_empty() {
        return Err(tr!("ключ PuTTY защищён парольной фразой — укажите её в профиле").into());
    }
    let mut mac_data = Vec::new();
    let mac_key;
    if ppk.version == 2 {
        if encrypted {
            let mut key = Vec::new();
            for n in 0u32..2 {
                let mut h = Sha1::new();
                h.update(n.to_be_bytes());
                h.update(passphrase.as_bytes());
                key.extend_from_slice(&h.finalize());
            }
            decrypt(&key[..32], &[0u8; 16], &mut ppk.private)?;
        }
        let mut h = Sha1::new();
        h.update(b"putty-private-key-file-mac-key");
        if encrypted {
            h.update(passphrase.as_bytes());
        }
        mac_key = h.finalize().to_vec();
    } else if encrypted {
        let (flavor, memory, passes, lanes, salt) =
            ppk.argon.clone().ok_or(tr!("повреждённый ключ PuTTY"))?;
        let params =
            argon2::Params::new(memory, passes, lanes, Some(80)).map_err(|e| e.to_string())?;
        let mut out = [0u8; 80];
        argon2::Argon2::new(flavor, argon2::Version::V0x13, params)
            .hash_password_into(passphrase.as_bytes(), &salt, &mut out)
            .map_err(|e| e.to_string())?;
        decrypt(&out[..32], &out[32..48], &mut ppk.private)?;
        mac_key = out[48..].to_vec();
    } else {
        mac_key = Vec::new();
    }
    for part in [
        ppk.kind.as_bytes(),
        ppk.encryption.as_bytes(),
        ppk.comment.as_bytes(),
        &ppk.public,
        &ppk.private,
    ] {
        put(&mut mac_data, part);
    }
    let expected = if ppk.version == 2 {
        hmac::<Sha1>(&mac_key, &mac_data)
    } else {
        hmac::<Sha256>(&mac_key, &mac_data)
    };
    if expected != ppk.mac {
        return Err(if encrypted {
            tr!("неверная парольная фраза ключа PuTTY").into()
        } else {
            tr!("ключ PuTTY повреждён (не сходится контрольная сумма)").into()
        });
    }
    Ok(())
}

pub fn to_openssh(text: &str, passphrase: &str) -> Result<String, String> {
    let mut ppk = parse(text)?;
    unlock(&mut ppk, passphrase)?;
    let (public, private) = (&ppk.public, &ppk.private);
    let mut at = 0;
    let kind = take(public, &mut at)?;
    let mut section = Vec::new();
    put(&mut section, kind);
    let mut p = 0;
    match kind {
        b"ssh-rsa" => {
            let e = take(public, &mut at)?;
            let n = take(public, &mut at)?;
            let d = take(private, &mut p)?;
            let pr = take(private, &mut p)?;
            let q = take(private, &mut p)?;
            let iqmp = take(private, &mut p)?;
            for part in [n, e, d, iqmp, pr, q] {
                put(&mut section, part);
            }
        }
        b"ssh-ed25519" => {
            let pk = take(public, &mut at)?;
            let mut seed = take(private, &mut p)?.to_vec();
            if seed.len() > 32 || pk.len() != 32 {
                return Err(tr!("повреждённый ключ PuTTY").into());
            }
            seed.resize(32, 0);
            put(&mut section, pk);
            seed.extend_from_slice(pk);
            put(&mut section, &seed);
        }
        k if k.starts_with(b"ecdsa-sha2-") => {
            let curve = take(public, &mut at)?;
            let point = take(public, &mut at)?;
            let d = take(private, &mut p)?;
            for part in [curve, point, d] {
                put(&mut section, part);
            }
        }
        b"ssh-dss" => return Err(tr!(super::remote::DSA_NOTE).into()),
        other => {
            return Err(tf!(
                "ключ PuTTY: тип {} не поддерживается",
                "PuTTY key: type {} is not supported",
                String::from_utf8_lossy(other)
            ));
        }
    }
    put(&mut section, ppk.comment.as_bytes());
    let mut body = Vec::new();
    body.extend_from_slice(&[0x54, 0x52, 0x4f, 0x52, 0x54, 0x52, 0x4f, 0x52]);
    body.extend_from_slice(&section);
    let mut pad = 1u8;
    while body.len() % 8 != 0 {
        body.push(pad);
        pad += 1;
    }
    let mut blob = b"openssh-key-v1\0".to_vec();
    put(&mut blob, b"none");
    put(&mut blob, b"none");
    put(&mut blob, b"");
    blob.extend_from_slice(&1u32.to_be_bytes());
    put(&mut blob, public);
    put(&mut blob, &body);
    let encoded = base64::engine::general_purpose::STANDARD.encode(blob);
    let mut pem = String::from("-----BEGIN OPENSSH PRIVATE KEY-----\n");
    for chunk in encoded.as_bytes().chunks(70) {
        pem.push_str(&String::from_utf8_lossy(chunk));
        pem.push('\n');
    }
    pem.push_str("-----END OPENSSH PRIVATE KEY-----\n");
    Ok(pem)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn puttygen() -> bool {
        std::process::Command::new("puttygen")
            .arg("--version")
            .output()
            .is_ok()
    }

    #[test]
    fn hmac_matches_rfc2202() {
        let mac = hmac::<Sha1>(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(mac[..4], [0xef, 0xfc, 0xdf, 0x6a]);
        let mac = hmac::<Sha256>(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(mac[..4], [0x5b, 0xdc, 0xc1, 0x46]);
    }

    #[test]
    fn converts_every_kind_and_format() {
        if !puttygen() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("tror-ppk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let phrase = dir.join("phrase");
        std::fs::write(&phrase, "секрет-1\n").unwrap();
        let empty = dir.join("empty");
        std::fs::write(&empty, "").unwrap();
        for kind in ["ed25519", "rsa", "ecdsa"] {
            for (tag, extra) in [
                ("plain", vec!["--new-passphrase", empty.to_str().unwrap()]),
                ("v3", vec!["--new-passphrase", phrase.to_str().unwrap()]),
                (
                    "v2",
                    vec![
                        "--new-passphrase",
                        phrase.to_str().unwrap(),
                        "--ppk-param",
                        "version=2",
                    ],
                ),
            ] {
                let ppk = dir.join(format!("{kind}-{tag}.ppk"));
                let ok = std::process::Command::new("puttygen")
                    .args(["-t", kind, "-q", "-o", ppk.to_str().unwrap()])
                    .args(&extra)
                    .status()
                    .unwrap();
                assert!(ok.success());
                let text = std::fs::read_to_string(&ppk).unwrap();
                let pass = if tag == "plain" { "" } else { "секрет-1" };
                if !pass.is_empty() {
                    assert!(
                        to_openssh(&text, "неверно")
                            .unwrap_err()
                            .contains("неверная")
                    );
                }
                let pem = to_openssh(&text, pass).unwrap();
                let file = dir.join(format!("{kind}-{tag}"));
                std::fs::write(&file, &pem).unwrap();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600))
                        .unwrap();
                }
                let ours = std::process::Command::new("ssh-keygen")
                    .args(["-y", "-f", file.to_str().unwrap()])
                    .output()
                    .unwrap();
                assert!(
                    ours.status.success(),
                    "{kind}-{tag}: {}",
                    String::from_utf8_lossy(&ours.stderr)
                );
                let pubfile = dir.join(format!("{kind}-{tag}.pub"));
                let mut theirs = std::process::Command::new("puttygen");
                theirs
                    .arg(&ppk)
                    .args(["-O", "public-openssh", "-o"])
                    .arg(&pubfile);
                if !pass.is_empty() {
                    theirs.args(["--old-passphrase", phrase.to_str().unwrap()]);
                }
                assert!(theirs.status().unwrap().success());
                let theirs = std::fs::read(&pubfile).unwrap();
                let key = |out: &[u8]| {
                    String::from_utf8_lossy(out)
                        .split_whitespace()
                        .take(2)
                        .collect::<Vec<_>>()
                        .join(" ")
                };
                assert_eq!(key(&ours.stdout), key(&theirs), "{kind}-{tag}");
            }
        }
        std::fs::remove_dir_all(dir).ok();
    }
}
