//! Application messages inside sessions and handshakes, validated as strictly as
//! `web/src/app/protocol.ts` does: anything malformed is dropped.

use serde_json::{Value, json};

use crate::text::clean_name;

pub const MAX_TEXT: usize = 16_000;
pub const MAX_FILE: u64 = 4 * 1024 * 1024 * 1024;
pub const MAX_TTL: u64 = 30 * 86_400;
/// File chunks carry this much data.
pub const CHUNK_SIZE: usize = 48 * 1024;

/// The JSON payload of a handshake message.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Hello {
    pub name: String,
    pub sdp: Option<String>,
    /// Contact requests carry the requester's bundle (base64url).
    pub bundle: Option<String>,
    pub note: Option<String>,
}

fn str_max(v: &Value, max: usize) -> Option<&str> {
    v.as_str().filter(|s| s.chars().count() <= max)
}

pub fn hello(name: &str, sdp: Option<&str>) -> Vec<u8> {
    let mut v = json!({"v": 1, "name": name});
    if let Some(s) = sdp {
        v["sdp"] = json!(s);
    }
    v.to_string().into_bytes()
}

pub fn parse_hello(b: &[u8]) -> Option<Hello> {
    let v: Value = serde_json::from_slice(b).ok()?;
    if v["v"] != 1 {
        return None;
    }
    Some(Hello {
        name: str_max(&v["name"], 256).map(clean_name).unwrap_or_default(),
        sdp: str_max(&v["sdp"], 64_000).map(str::to_string),
        bundle: str_max(&v["bundle"], 16_000).map(str::to_string),
        note: str_max(&v["note"], 280).map(|s| crate::text::clean_text(s).trim().to_string()),
    })
}

fn is_id(v: &Value) -> Option<String> {
    let s = v.as_str()?;
    let ok = (8..=32).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    ok.then(|| s.to_string())
}

fn ttl(v: &Value) -> Option<Option<u64>> {
    match v {
        Value::Null => Some(None),
        _ => v.as_u64().filter(|t| *t <= MAX_TTL).map(|t| (t > 0).then_some(t)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ctrl {
    Verified,
    Name(String),
    Card(String),
    Lock(bool),
}

pub fn parse_ctrl(b: &[u8]) -> Option<Ctrl> {
    let v: Value = serde_json::from_slice(b).ok()?;
    match v["t"].as_str()? {
        "verified" => Some(Ctrl::Verified),
        "name" => str_max(&v["name"], 256).map(|n| Ctrl::Name(clean_name(n))),
        "card" => str_max(&v["card"], 16_000).map(|c| Ctrl::Card(c.to_string())),
        "lock" => v["locked"].as_bool().map(Ctrl::Lock),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMeta {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub mime: String,
    pub sha256: String,
    pub ttl: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatMsg {
    Msg { id: String, text: String, ttl: Option<u64> },
    Del { id: String },
    File(FileMeta),
    FileGet { id: String },
    FileEnd { id: String },
    FileCancel { id: String },
}

fn is_mime(s: &str) -> bool {
    let mut parts = s.splitn(2, '/');
    let ok = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.+-".contains(&b));
    matches!((parts.next(), parts.next()), (Some(a), Some(b)) if ok(a) && ok(b))
}

pub fn parse_chat(b: &[u8]) -> Option<ChatMsg> {
    let v: Value = serde_json::from_slice(b).ok()?;
    let id = || is_id(&v["id"]);
    match v["t"].as_str()? {
        "msg" => {
            v["ts"].as_f64().filter(|t| t.is_finite() && *t >= 0.0)?;
            Some(ChatMsg::Msg {
                id: id()?,
                text: str_max(&v["text"], MAX_TEXT)?.to_string(),
                ttl: ttl(&v["ttl"])?,
            })
        }
        "del" => Some(ChatMsg::Del { id: id()? }),
        "file" => {
            v["ts"].as_f64().filter(|t| t.is_finite() && *t >= 0.0)?;
            let size = v["size"]
                .as_f64()
                .filter(|s| s.is_finite() && *s >= 0.0 && *s <= MAX_FILE as f64)? as u64;
            let sha = v["sha256"]
                .as_str()
                .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()))?;
            let mime = str_max(&v["mime"], 128)?;
            let name = clean_name(&str_max(&v["name"], 512)?.replace(['/', '\\'], "_"));
            Some(ChatMsg::File(FileMeta {
                id: id()?,
                name: if name.is_empty() { "file".into() } else { name },
                size,
                mime: if is_mime(mime) {
                    mime.to_string()
                } else {
                    "application/octet-stream".into()
                },
                sha256: sha.to_string(),
                ttl: ttl(&v["ttl"])?,
            }))
        }
        "file-get" => Some(ChatMsg::FileGet { id: id()? }),
        "file-end" => Some(ChatMsg::FileEnd { id: id()? }),
        "file-cancel" => Some(ChatMsg::FileCancel { id: id()? }),
        _ => None,
    }
}

/// Image types shown inline and downloaded by themselves (never SVG: it can carry script).
pub fn previewable(mime: &str) -> bool {
    matches!(
        mime,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/avif"
    )
}

pub fn chunk(id: &str, index: u32, data: &[u8]) -> Vec<u8> {
    let mut c = Vec::with_capacity(21 + data.len());
    c.push(1u8);
    c.extend_from_slice(id.as_bytes());
    c.extend_from_slice(&index.to_be_bytes());
    c.extend_from_slice(data);
    c
}

pub fn parse_chunk(data: &[u8]) -> Option<(String, u32, &[u8])> {
    if data.len() < 21 || data[0] != 1 {
        return None;
    }
    let id = std::str::from_utf8(&data[1..17]).ok()?.to_string();
    let index = u32::from_be_bytes(data[17..21].try_into().ok()?);
    Some((id, index, &data[21..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_like_the_web_app() {
        assert_eq!(
            parse_chat(br#"{"t":"msg","id":"abcdefgh12345678","ts":1,"text":"hi","ttl":60}"#),
            Some(ChatMsg::Msg {
                id: "abcdefgh12345678".into(),
                text: "hi".into(),
                ttl: Some(60)
            })
        );
        assert!(parse_chat(br#"{"t":"msg","id":"a/b","ts":1,"text":"hi"}"#).is_none());
        assert!(parse_chat(br#"{"t":"msg","id":"abcdefgh","ts":1,"text":"hi","ttl":99999999}"#).is_none());
        let f = parse_chat(
            br#"{"t":"file","id":"abcdefgh12345678","ts":1,"name":"../a\u202e.txt","size":3,"mime":"text/html; x","sha256":"0000000000000000000000000000000000000000000000000000000000000000"}"#,
        )
        .unwrap();
        let ChatMsg::File(f) = f else { panic!() };
        assert_eq!(f.name, ".._a.txt");
        assert_eq!(f.mime, "application/octet-stream");
        assert_eq!(parse_ctrl(br#"{"t":"lock"}"#), None);
        assert_eq!(
            parse_ctrl(br#"{"t":"name","name":" Bo\u0007b "}"#),
            Some(Ctrl::Name("Bob".into()))
        );
        let h = parse_hello(br#"{"v":1,"name":"x","sdp":"s"}"#).unwrap();
        assert_eq!(h.sdp.as_deref(), Some("s"));
        assert!(parse_hello(br#"{"v":2}"#).is_none());
        let c = chunk("abcdefgh12345678", 7, b"data");
        assert_eq!(parse_chunk(&c), Some(("abcdefgh12345678".into(), 7, &b"data"[..])));
    }
}
