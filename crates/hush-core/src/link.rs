//! Invitation links: `https://…/hush/#2.<mode>.<secret>.<expiry>[.p]`.
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
}

impl Mode {
    #[must_use]
    pub fn code(self) -> char {
        match self {
            Mode::Direct => 's',
            Mode::Group => 'g',
        }
    }

    fn from_code(c: &str) -> Result<Self> {
        match c {
            "s" => Ok(Mode::Direct),
            "g" => Ok(Mode::Group),
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
}

impl core::fmt::Debug for Link {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Link")
            .field("mode", &self.mode)
            .field("expires_min", &self.expires_min)
            .field("passphrase", &self.passphrase)
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
        let parts: Vec<&str> = fragment.split('.').collect();
        let (version, mode, secret, expiry, flags) = match parts.as_slice() {
            [v, m, s, e] => (*v, *m, *s, *e, None),
            [v, m, s, e, f] => (*v, *m, *s, *e, Some(*f)),
            _ => return Err(Error::Malformed),
        };
        match version {
            LINK_VERSION => {}
            "1" => return Err(Error::Outdated),
            _ => return Err(Error::Malformed),
        }
        let passphrase = match flags {
            None => false,
            Some("p") => true,
            Some(_) => return Err(Error::Malformed),
        };
        Ok(Self {
            mode: Mode::from_code(mode)?,
            secret: Zeroizing::new(b64::decode_array(secret)?),
            expires_min: parse_base36(expiry)?,
            passphrase,
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
