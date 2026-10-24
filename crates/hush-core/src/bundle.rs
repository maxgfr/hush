//! Signed key bundles and contact cards.
//!
//! ```text
//! bundle = v(1) || ed_pk(32) || mldsa_pk(1952) || spk_epoch(u32) || spk(1216)
//!          || created_ms(u64) || expires_ms(u64) || name_len(1) || name(≤ 64 bytes UTF-8)
//!          || hybrid_sig("bundle", everything before)
//! card   = bundle_len(u16) || bundle || hybrid_sig("card", session_hash || SHA-256(bundle))
//! ```
//! A bundle proves "the owner of identifier X currently uses prekey P". A card additionally proves
//! that owner is the peer of *this* session (it signs the session's transcript hash), so a card
//! cannot be replayed into another conversation.

use crate::identity::{ED_PK_LEN, Identity, MLDSA_PK_LEN, PublicIdentity, SIG_LEN};
use crate::kdf::sha256;
use crate::kem::EK_LEN;
use crate::{Error, Result, Rng, VERSION};

/// Longest display name, in bytes.
pub const MAX_NAME: usize = 64;
/// Bundles are valid for two prekey periods.
pub const BUNDLE_TTL_MS: u64 = 14 * 86_400_000;

const FIXED_LEN: usize = 1 + ED_PK_LEN + MLDSA_PK_LEN + 4 + EK_LEN + 8 + 8 + 1;

/// A verified bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    pub identity: PublicIdentity,
    pub spk_epoch: u32,
    pub spk: Vec<u8>,
    pub created_ms: u64,
    pub expires_ms: u64,
    pub name: String,
    /// The exact signed encoding.
    pub bytes: Vec<u8>,
}

/// Zero-width and bidirectional-override characters that could make one name pass for another.
pub(crate) const SPOOFING: [char; 15] = [
    '\u{200b}', '\u{200c}', '\u{200d}', '\u{200e}', '\u{200f}', '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}',
    '\u{202e}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}', '\u{feff}',
];

/// A peer-chosen name made safe to display: no control characters (terminal escapes included),
/// no zero-width or bidi-override characters, no surrounding spaces.
#[must_use]
pub fn display_name(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_control() && !SPOOFING.contains(c))
        .collect::<String>()
        .trim()
        .to_string()
}

fn clamp_name(name: &str) -> &str {
    let mut end = name.len().min(MAX_NAME);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    &name[..end]
}

impl Bundle {
    /// Builds and signs our bundle for the current prekey epoch.
    pub fn create<R: Rng + ?Sized>(me: &Identity, name: &str, now_ms: u64, rng: &mut R) -> Self {
        let epoch = Identity::spk_epoch(crate::utc_day(now_ms));
        let spk = me.spk(epoch).public();
        let name = clamp_name(name);
        let mut b = Vec::with_capacity(FIXED_LEN + name.len() + SIG_LEN);
        b.push(VERSION);
        b.extend_from_slice(me.public().ed_pk());
        b.extend_from_slice(me.public().mldsa_pk());
        b.extend_from_slice(&epoch.to_be_bytes());
        b.extend_from_slice(&spk);
        b.extend_from_slice(&now_ms.to_be_bytes());
        b.extend_from_slice(&(now_ms + BUNDLE_TTL_MS).to_be_bytes());
        b.push(u8::try_from(name.len()).expect("clamped"));
        b.extend_from_slice(name.as_bytes());
        let sig = me.sign(b"bundle", &b, rng);
        b.extend_from_slice(&sig);
        Self::parse(&b, now_ms).expect("freshly signed bundle verifies")
    }

    /// Parses and verifies a bundle.
    ///
    /// # Errors
    /// [`Error::Malformed`], [`Error::BadSignature`], or [`Error::Expired`] once past its expiry
    /// (or when it claims a lifetime longer than [`BUNDLE_TTL_MS`]).
    pub fn parse(b: &[u8], now_ms: u64) -> Result<Self> {
        let bundle = Self::parse_stored(b)?;
        if now_ms >= bundle.expires_ms
            || bundle.created_ms > now_ms + 86_400_000
            || bundle.expires_ms.saturating_sub(bundle.created_ms) > BUNDLE_TTL_MS
        {
            return Err(Error::Expired);
        }
        Ok(bundle)
    }

    /// Parses and verifies a bundle we stored earlier (a saved contact's), ignoring its expiry:
    /// prekeys are derived from the identity seed, so an old one still reaches its owner.
    ///
    /// # Errors
    /// [`Error::Malformed`] or [`Error::BadSignature`].
    pub fn parse_stored(b: &[u8]) -> Result<Self> {
        if b.len() < FIXED_LEN + SIG_LEN || b[0] != VERSION {
            return Err(Error::Malformed);
        }
        let mut at = 1;
        let mut take = |n: usize| -> Result<&[u8]> {
            let s = b.get(at..at + n).ok_or(Error::Malformed)?;
            at += n;
            Ok(s)
        };
        let ed_pk = take(ED_PK_LEN)?;
        let mldsa_pk = take(MLDSA_PK_LEN)?;
        let spk_epoch = u32::from_be_bytes(take(4)?.try_into().map_err(|_| Error::Malformed)?);
        let spk = take(EK_LEN)?.to_vec();
        let created_ms = u64::from_be_bytes(take(8)?.try_into().map_err(|_| Error::Malformed)?);
        let expires_ms = u64::from_be_bytes(take(8)?.try_into().map_err(|_| Error::Malformed)?);
        let name_len = usize::from(take(1)?[0]);
        if name_len > MAX_NAME {
            return Err(Error::Malformed);
        }
        // The signed bytes keep the name as sent; what callers display is cleaned.
        let name = display_name(core::str::from_utf8(take(name_len)?).map_err(|_| Error::Malformed)?);
        let signed_len = FIXED_LEN + name_len;
        if b.len() != signed_len + SIG_LEN {
            return Err(Error::Malformed);
        }
        let sig = &b[signed_len..];
        let identity = PublicIdentity::from_keys(ed_pk, mldsa_pk)?;
        identity.verify(b"bundle", &b[..signed_len], sig)?;
        Ok(Self {
            identity,
            spk_epoch,
            spk,
            created_ms,
            expires_ms,
            name,
            bytes: b.to_vec(),
        })
    }

    #[must_use]
    pub fn id_hash(&self) -> &[u8; 32] {
        self.identity.id_hash()
    }
}

/// Builds our contact card for a session.
pub fn card<R: Rng + ?Sized>(me: &Identity, name: &str, session_hash: &[u8; 32], now_ms: u64, rng: &mut R) -> Vec<u8> {
    let bundle = Bundle::create(me, name, now_ms, rng);
    let mut out = u16::try_from(bundle.bytes.len())
        .expect("bundle under 64 KiB")
        .to_be_bytes()
        .to_vec();
    out.extend_from_slice(&bundle.bytes);
    let digest = sha256(&[&bundle.bytes]);
    out.extend_from_slice(&me.sign(b"card", &[session_hash.as_slice(), &digest].concat(), rng));
    out
}

/// Verifies a peer's card for this session.
///
/// # Errors
/// [`Error::BadSignature`] if the card was made for another session or forged.
pub fn verify_card(card: &[u8], session_hash: &[u8; 32], now_ms: u64) -> Result<Bundle> {
    let len = usize::from(u16::from_be_bytes(
        card.get(..2)
            .ok_or(Error::Malformed)?
            .try_into()
            .map_err(|_| Error::Malformed)?,
    ));
    let bundle_bytes = card.get(2..2 + len).ok_or(Error::Malformed)?;
    let sig = card.get(2 + len..).ok_or(Error::Malformed)?;
    let bundle = Bundle::parse(bundle_bytes, now_ms)?;
    let digest = sha256(&[bundle_bytes]);
    bundle
        .identity
        .verify(b"card", &[session_hash.as_slice(), &digest].concat(), sig)?;
    Ok(bundle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_round_trip_and_checks() {
        let mut rng = crate::test_rng::rng();
        let me = Identity::generate(&mut rng);
        let now = 1_750_000_000_000;
        let b = Bundle::create(&me, "Alice", now, &mut rng);
        assert_eq!(b.name, "Alice");
        assert_eq!(b.id_hash(), me.public().id_hash());
        assert_eq!(Bundle::parse(&b.bytes, now).unwrap(), b);
        assert_eq!(
            Bundle::parse(&b.bytes, now + BUNDLE_TTL_MS).unwrap_err(),
            Error::Expired
        );
        let mut t = b.bytes.clone();
        t[40] ^= 1;
        assert!(Bundle::parse(&t, now).is_err());
        let long = Bundle::create(&me, &"é".repeat(40), now, &mut rng);
        assert!(long.name.len() <= MAX_NAME);
    }

    #[test]
    fn overlong_lifetime_is_refused() {
        let mut rng = crate::test_rng::rng();
        let me = Identity::generate(&mut rng);
        let now = 1_750_000_000_000;
        let b = Bundle::create(&me, "", now, &mut rng);
        let mut signed = b.bytes[..b.bytes.len() - SIG_LEN].to_vec();
        let expires_at = signed.len() - 1 - 8;
        signed[expires_at..expires_at + 8].copy_from_slice(&(now + 10 * BUNDLE_TTL_MS).to_be_bytes());
        let sig = me.sign(b"bundle", &signed, &mut rng);
        signed.extend_from_slice(&sig);
        assert!(Bundle::parse_stored(&signed).is_ok());
        assert_eq!(Bundle::parse(&signed, now).unwrap_err(), Error::Expired);
    }

    #[test]
    fn names_are_cleaned_for_display() {
        assert_eq!(display_name(" Bob\u{202e}kcilc\u{1b}]8;;x\u{7}\n "), "Bobkcilc]8;;x");
        let mut rng = crate::test_rng::rng();
        let me = Identity::generate(&mut rng);
        let b = Bundle::create(&me, "A\u{1b}[2Jlice", 1_750_000_000_000, &mut rng);
        assert_eq!(b.name, "A[2Jlice");
    }

    #[test]
    fn card_is_bound_to_its_session() {
        let mut rng = crate::test_rng::rng();
        let me = Identity::generate(&mut rng);
        let now = 1_750_000_000_000;
        let c = card(&me, "Bob", &[1; 32], now, &mut rng);
        assert_eq!(verify_card(&c, &[1; 32], now).unwrap().name, "Bob");
        assert_eq!(verify_card(&c, &[2; 32], now).unwrap_err(), Error::BadSignature);
    }
}
