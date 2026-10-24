//! Invitation links: `https://…/hush/#2.<mode>.<secret>.<expiry>[.h<header>.s<servers>][.p]`.
//!
//! The 32-byte secret only ever lives in the URL fragment, which browsers never send to the web
//! server. Every key of the session's bootstrap is derived from it:
//!
//! ```text
//! root     = HKDF-Extract("hush/v1/link", L)
//! topic_d  = Expand(root, "hush/v1/link/topic" || u32 day)    rendezvous, rotates daily
//! env_key  = Expand(root, "hush/v1/link/env")                  seals signaling envelopes
//! room_id  = Expand(root, "hush/v1/link/room")[..16]           bound into the handshake
//! psk      = Expand(root, "hush/v1/link/psk")                  or, with a passphrase:
//! psk      = Expand(Extract(root, Argon2id(NFKC(pass), H("hush/v1/link/pass" || L)[..16])), "hush/v1/link/psk")
//! ```
//!
//! Version 2 changed the link handshake (see `handshake`); version 1 links are refused with
//! [`Error::Outdated`] so an old link fails clearly instead of as a wrong passphrase.
//!
//! The topic never depends on the passphrase, so a wrong passphrase fails the handshake loudly
//! instead of silently waiting on another topic.
//!
//! A drop link (mode `d`) also says where its sealed copy is stored: the SHA-256 of the header blob
//! (`h`) and the servers holding it (`s`, hosts joined by `~`). A drop sealed to an identity has no
//! secret at all: `#2.r.<header>.<servers>` only helps whoever holds the recipient's keys.
//! ```text
//! drop_root = Extract("hush/v2/drop/link", L)                           or, with a passphrase:
//! drop_root = Extract(Extract("hush/v2/drop/link", L), Argon2id(NFKC(pass), H("hush/v1/link/pass" || L)[..16]))
//! ```

use zeroize::Zeroizing;

use crate::kdf::{expand16, expand32, extract, sha256};
use crate::vault::{KdfParams, argon2id};
use crate::{Error, Result, Rng, b64};

/// What the link opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A one-to-one conversation; the first person to open the link takes it.
    Direct,
    /// A group room: everyone holding the link may join (until the room is locked).
    Group,
    /// A file drop: the sender serves the files peer to peer to the first person who opens it,
    /// and a sealed copy may wait on blob servers (see `drop`). One-to-one, like `Direct`.
    Drop,
}

impl Mode {
    #[must_use]
    pub fn code(self) -> char {
        match self {
            Mode::Direct => 's',
            Mode::Group => 'g',
            Mode::Drop => 'd',
        }
    }

    fn from_code(c: &str) -> Result<Self> {
        match c {
            "s" => Ok(Mode::Direct),
            "g" => Ok(Mode::Group),
            "d" => Ok(Mode::Drop),
            _ => Err(Error::Malformed),
        }
    }
}

/// The link format, first field of the fragment.
pub const LINK_VERSION: &str = "2";

/// Argon2id costs for link passphrases: 64 MiB, 3 passes, 1 lane.
pub const LINK_PASS_PARAMS: KdfParams = KdfParams {
    m_kib: 64 * 1024,
    t: 3,
    p: 1,
};

/// A parsed invitation link.
#[derive(Clone)]
pub struct Link {
    pub mode: Mode,
    secret: Zeroizing<[u8; 32]>,
    /// Expiry, in minutes since the Unix epoch.
    pub expires_min: u64,
    /// Whether the link needs a passphrase shared through another channel.
    pub passphrase: bool,
    /// Where the sealed copy of a drop is stored (drop links only, absent for peer-to-peer only).
    pub locator: Option<Locator>,
}

impl core::fmt::Debug for Link {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Link")
            .field("mode", &self.mode)
            .field("expires_min", &self.expires_min)
            .field("passphrase", &self.passphrase)
            .field("locator", &self.locator)
            .finish_non_exhaustive()
    }
}

fn base36(mut n: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    loop {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
        if n == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).expect("ascii")
}

fn parse_base36(s: &str) -> Result<u64> {
    if s.is_empty() || s.len() > 12 || s.starts_with('0') && s.len() > 1 {
        return Err(Error::Malformed);
    }
    u64::from_str_radix(s, 36).map_err(|_| Error::Malformed)
}

impl Link {
    /// A fresh link valid for `ttl_min` minutes.
    pub fn generate<R: Rng + ?Sized>(mode: Mode, now_ms: u64, ttl_min: u64, passphrase: bool, rng: &mut R) -> Self {
        Self {
            mode,
            secret: Zeroizing::new(crate::random(rng)),
            expires_min: now_ms / 60_000 + ttl_min.max(1),
            passphrase,
            locator: None,
        }
    }

    /// Parses the fragment (with or without the leading `#`, or a full URL).
    ///
    /// # Errors
    /// [`Error::Malformed`] when it is not a hush link, [`Error::Outdated`] for a version 1 link.
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        let fragment = match input.rfind('#') {
            Some(i) => &input[i + 1..],
            None => input,
        };
        // Hosts contain dots, so only the first four fields split on them; the rest is
        // `[h<header>.s<servers>][.p]` (no host ends in `.p`, see `valid_host`).
        let parts: Vec<&str> = fragment.splitn(5, '.').collect();
        let (version, mode, secret, expiry, rest) = match parts.as_slice() {
            [v, m, s, e] => (*v, *m, *s, *e, ""),
            [v, m, s, e, rest] => (*v, *m, *s, *e, *rest),
            _ => return Err(Error::Malformed),
        };
        match version {
            LINK_VERSION => {}
            "1" => return Err(Error::Outdated),
            _ => return Err(Error::Malformed),
        }
        let mode = Mode::from_code(mode)?;
        let (rest, passphrase) = match rest {
            "p" => ("", true),
            r => r.strip_suffix(".p").map_or((r, false), |r| (r, true)),
        };
        let locator = match rest {
            "" => None,
            r if mode == Mode::Drop => Some(Locator::from_rest(r)?),
            _ => return Err(Error::Malformed),
        };
        Ok(Self {
            mode,
            secret: Zeroizing::new(b64::decode_array(secret)?),
            expires_min: parse_base36(expiry)?,
            passphrase,
            locator,
        })
    }

    /// The fragment, without `#`.
    #[must_use]
    pub fn fragment(&self) -> String {
        let mut s = format!(
            "{LINK_VERSION}.{}.{}.{}",
            self.mode.code(),
            b64::encode(&*self.secret),
            base36(self.expires_min)
        );
        if let Some(locator) = &self.locator {
            s.push('.');
            s.push_str(&locator.parts());
        }
        if self.passphrase {
            s.push_str(".p");
        }
        s
    }

    /// The full URL under `base` (e.g. `https://maxgfr.github.io/hush/`).
    #[must_use]
    pub fn url(&self, base: &str) -> String {
        let base = base.split('#').next().unwrap_or(base);
        format!("{base}#{}", self.fragment())
    }

    #[must_use]
    pub fn is_expired(&self, now_ms: u64) -> bool {
        now_ms / 60_000 >= self.expires_min
    }

    /// Derives the bootstrap keys. Slow (Argon2id) when the link has a passphrase.
    ///
    /// # Errors
    /// [`Error::InvalidParameter`] when a passphrase is required but missing, or given but not expected.
    pub fn keys(&self, passphrase: Option<&str>) -> Result<LinkKeys> {
        self.keys_with(passphrase, LINK_PASS_PARAMS)
    }

    /// [`Link::keys`] with explicit Argon2 costs (tests use cheap ones).
    ///
    /// # Errors
    /// See [`Link::keys`].
    pub fn keys_with(&self, passphrase: Option<&str>, params: KdfParams) -> Result<LinkKeys> {
        let root = Zeroizing::new(extract(b"hush/v1/link", &*self.secret));
        let psk = match (self.passphrase, passphrase) {
            (false, None) => expand32(&root, b"hush/v1/link/psk", &[]),
            (true, Some(pass)) => {
                let normalized = crate::vault::normalize(pass);
                let salt = sha256(&[b"hush/v1/link/pass", &*self.secret]);
                let stretched = argon2id(normalized.as_bytes(), &salt[..16], params)?;
                let mixed = Zeroizing::new(extract(&*root, &*stretched));
                expand32(&mixed, b"hush/v1/link/psk", &[])
            }
            _ => return Err(Error::InvalidParameter),
        };
        Ok(LinkKeys {
            env_key: Zeroizing::new(expand32(&root, b"hush/v1/link/env", &[])),
            room_id: expand16(&root, b"hush/v1/link/room", &[]),
            psk: Zeroizing::new(psk),
            mode: self.mode,
            expires_min: self.expires_min,
            root,
        })
    }
}

impl Link {
    /// The root that wraps a drop's file key (see `drop::Recipient::Secret`). Slow (Argon2id) when
    /// the link has a passphrase.
    ///
    /// # Errors
    /// [`Error::InvalidParameter`] when a passphrase is required but missing, or given but not expected.
    pub fn drop_root(&self, passphrase: Option<&str>) -> Result<Zeroizing<[u8; 32]>> {
        self.drop_root_with(passphrase, LINK_PASS_PARAMS)
    }

    /// [`Link::drop_root`] with explicit Argon2 costs (tests use cheap ones).
    ///
    /// # Errors
    /// See [`Link::drop_root`].
    pub fn drop_root_with(&self, passphrase: Option<&str>, params: KdfParams) -> Result<Zeroizing<[u8; 32]>> {
        let root = Zeroizing::new(extract(b"hush/v2/drop/link", &*self.secret));
        match (self.passphrase, passphrase) {
            (false, None) => Ok(root),
            (true, Some(pass)) => {
                let normalized = crate::vault::normalize(pass);
                let salt = sha256(&[b"hush/v1/link/pass", &*self.secret]);
                let stretched = argon2id(normalized.as_bytes(), &salt[..16], params)?;
                Ok(Zeroizing::new(extract(&*root, &*stretched)))
            }
            _ => Err(Error::InvalidParameter),
        }
    }
}

/// Most servers a locator names: the sealed copy is mirrored, so one is enough to fetch it.
pub const MAX_SERVERS: usize = 4;

/// Where a sealed drop is stored: the SHA-256 of its header blob, and the blob servers holding it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locator {
    pub header: [u8; 32],
    /// Hosts (`name[:port]`, lowercase); see [`Locator::server_url`].
    pub servers: Vec<String>,
}

fn valid_host(h: &str) -> bool {
    !h.is_empty()
        && h.len() <= 253
        && h.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-' || b == b':')
        && !h.starts_with(['.', '-', ':'])
        && !h.ends_with(".p")
}

impl Locator {
    /// Builds a locator, checking every host.
    ///
    /// # Errors
    /// [`Error::Malformed`] for no host, too many, or one that is not a plain `name[:port]`.
    pub fn new(header: [u8; 32], servers: Vec<String>) -> Result<Self> {
        if servers.is_empty() || servers.len() > MAX_SERVERS || !servers.iter().all(|h| valid_host(h)) {
            return Err(Error::Malformed);
        }
        Ok(Self { header, servers })
    }

    /// Parses `h<header>.s<servers>`.
    fn from_rest(rest: &str) -> Result<Self> {
        let (h, s) = rest.split_once('.').ok_or(Error::Malformed)?;
        let header = b64::decode_array(h.strip_prefix('h').ok_or(Error::Malformed)?)?;
        let servers = s.strip_prefix('s').ok_or(Error::Malformed)?;
        Self::new(header, servers.split('~').map(str::to_string).collect())
    }

    fn parts(&self) -> String {
        format!("h{}.s{}", b64::encode(&self.header), self.servers.join("~"))
    }

    /// The fragment of a drop sealed to an identity, without `#`: it holds no secret.
    #[must_use]
    pub fn sealed_fragment(&self) -> String {
        format!("{LINK_VERSION}.r.{}", self.parts())
    }

    /// The full URL of a drop sealed to an identity, under `base`.
    #[must_use]
    pub fn sealed_url(&self, base: &str) -> String {
        let base = base.split('#').next().unwrap_or(base);
        format!("{base}#{}", self.sealed_fragment())
    }

    /// Parses a sealed drop's fragment (with or without `#`, or a full URL).
    ///
    /// # Errors
    /// [`Error::Malformed`] when it is not one.
    pub fn parse_sealed(input: &str) -> Result<Self> {
        let input = input.trim();
        let fragment = input.rfind('#').map_or(input, |i| &input[i + 1..]);
        let rest = fragment
            .strip_prefix(LINK_VERSION)
            .and_then(|r| r.strip_prefix(".r."))
            .ok_or(Error::Malformed)?;
        Self::from_rest(rest)
    }

    /// The base URL of a server: plain HTTP only for the local machine (tests), HTTPS otherwise.
    #[must_use]
    pub fn server_url(host: &str) -> String {
        let local = host == "localhost"
            || host.starts_with("localhost:")
            || host == "127.0.0.1"
            || host.starts_with("127.0.0.1:");
        format!("{}://{host}", if local { "http" } else { "https" })
    }
}

/// Keys derived from a link.
#[derive(Clone)]
pub struct LinkKeys {
    root: Zeroizing<[u8; 32]>,
    pub env_key: Zeroizing<[u8; 32]>,
    pub room_id: [u8; 16],
    pub psk: Zeroizing<[u8; 32]>,
    pub mode: Mode,
    pub expires_min: u64,
}

impl LinkKeys {
    /// The rendezvous topic for a UTC day.
    #[must_use]
    pub fn topic(&self, day: u32) -> [u8; 32] {
        expand32(&self.root, b"hush/v1/link/topic", &[&day.to_be_bytes()])
    }

    /// The handshake prologue: binds the mode, room and expiry, so none can be altered in transit.
    #[must_use]
    pub fn prologue(&self) -> Vec<u8> {
        let mut p = b"hush/v1/link/prologue".to_vec();
        p.push(self.mode.code() as u8);
        p.extend_from_slice(&self.room_id);
        p.extend_from_slice(&self.expires_min.to_be_bytes());
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHEAP: KdfParams = KdfParams { m_kib: 64, t: 1, p: 1 };

    #[test]
    fn format_and_parse() {
        let mut rng = crate::test_rng::rng();
        let link = Link::generate(Mode::Direct, 1_700_000_000_000, 60, false, &mut rng);
        let url = link.url("https://maxgfr.github.io/hush/");
        assert!(url.starts_with("https://maxgfr.github.io/hush/#2.s."));
        let parsed = Link::parse(&url).unwrap();
        assert_eq!(parsed.fragment(), link.fragment());
        assert_eq!(parsed.expires_min, 1_700_000_000_000 / 60_000 + 60);
        assert!(!parsed.is_expired(1_700_000_000_000));
        assert!(parsed.is_expired(1_700_000_000_000 + 60 * 60_000));

        let g = Link::generate(Mode::Group, 0, 5, true, &mut rng);
        let p = Link::parse(&g.fragment()).unwrap();
        assert_eq!(p.mode, Mode::Group);
        assert!(p.passphrase);
    }

    #[test]
    fn rejects_garbage() {
        for bad in [
            "",
            "#",
            "#2.s.AAAA.1",
            "2.x.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA.1",
            "2.s.short.1",
            "2.s.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA.1.q",
            "2.s.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA.01",
        ] {
            assert!(Link::parse(bad).is_err(), "{bad}");
        }
        assert_eq!(
            Link::parse("https://maxgfr.github.io/hush/#1.s.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA.1").err(),
            Some(Error::Outdated)
        );
    }

    #[test]
    fn drop_links() {
        let mut rng = crate::test_rng::rng();
        let mut link = Link::generate(Mode::Drop, 0, 60, true, &mut rng);
        assert!(link.fragment().starts_with("2.d."));
        assert_eq!(Link::parse(&link.fragment()).unwrap().locator, None);
        let locator = Locator::new([9; 32], vec!["nostr.download".into(), "127.0.0.1:8080".into()]).unwrap();
        link.locator = Some(locator.clone());
        let f = link.fragment();
        assert!(f.ends_with(".snostr.download~127.0.0.1:8080.p"), "{f}");
        let parsed = Link::parse(&f).unwrap();
        assert_eq!(parsed.locator.as_ref(), Some(&locator));
        assert!(parsed.passphrase);
        assert_eq!(parsed.fragment(), f);

        let sealed = locator.sealed_url("https://maxgfr.github.io/hush/");
        assert_eq!(Locator::parse_sealed(&sealed).unwrap(), locator);
        assert!(Link::parse(&sealed).is_err());
        assert!(Locator::parse_sealed(&f).is_err());
        assert_eq!(Locator::server_url("127.0.0.1:8080"), "http://127.0.0.1:8080");
        assert_eq!(Locator::server_url("nostr.download"), "https://nostr.download");

        // A locator only belongs to drop links, and hosts are plain names.
        let s = Link::generate(Mode::Direct, 0, 60, false, &mut rng).fragment();
        assert!(Link::parse(&format!("{s}.h{}.sa.b", b64::encode(&[0; 32]))).is_err());
        for bad in ["", "Evil.com", "a/b", "a b", "-a", "a@b", "a.p", "a~b"] {
            assert!(Locator::new([0; 32], vec![bad.into()]).is_err(), "{bad}");
        }
        assert!(Locator::new([0; 32], vec![]).is_err());
        assert!(Locator::new([0; 32], vec!["a.b".into(); MAX_SERVERS + 1]).is_err());

        let a = link.drop_root_with(Some("pass"), CHEAP).unwrap();
        let b = link.drop_root_with(Some("Pass"), CHEAP).unwrap();
        assert_ne!(*a, *b);
        assert!(link.drop_root_with(None, CHEAP).is_err());
        // The drop root never equals a session key derived from the same link.
        assert_ne!(*a, *link.keys_with(Some("pass"), CHEAP).unwrap().psk);
    }

    #[test]
    fn passphrase_changes_psk_but_not_topic() {
        let mut rng = crate::test_rng::rng();
        let link = Link::generate(Mode::Direct, 0, 5, true, &mut rng);
        let a = link.keys_with(Some("correct horse"), CHEAP).unwrap();
        let b = link.keys_with(Some("wrong horse"), CHEAP).unwrap();
        assert_eq!(a.topic(3), b.topic(3));
        assert_ne!(*a.psk, *b.psk);
        // NFKC: a full-width letter is the same passphrase.
        #[cfg(feature = "nfkc")]
        {
            let c = link.keys_with(Some("ｃorrect horse"), CHEAP).unwrap();
            assert_eq!(*a.psk, *c.psk);
        }
        assert!(link.keys_with(None, CHEAP).is_err());
        assert_ne!(a.topic(3), a.topic(4));
    }
}
