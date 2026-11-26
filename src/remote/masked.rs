use serde::{Deserialize, Deserializer, Serializer};

const MASK: &[u8] = b"tror-profile-mask";

pub fn hide(plain: &str) -> String {
    plain
        .bytes()
        .enumerate()
        .map(|(i, b)| format!("{:02x}", b ^ MASK[i % MASK.len()]))
        .collect()
}

pub fn reveal(hidden: &str) -> Option<String> {
    if !hidden.len().is_multiple_of(2) || !hidden.is_ascii() {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..hidden.len() / 2)
        .map(|i| {
            u8::from_str_radix(&hidden[i * 2..i * 2 + 2], 16)
                .ok()
                .map(|b| b ^ MASK[i % MASK.len()])
        })
        .collect();
    String::from_utf8(bytes?).ok()
}

pub fn serialize<S: Serializer>(plain: &str, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&hide(plain))
}

pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let hidden = String::deserialize(d)?;
    Ok(reveal(&hidden).unwrap_or_default())
}
