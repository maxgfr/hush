//! The two handshakes. Both are three messages, carried over signaling envelopes, so the WebRTC
//! offer and answer (which reveal IP addresses) only ever travel encrypted to an authenticated
//! peer. Neither depends on the transport underneath: the same bytes work over WebRTC, LAN TCP,
//! Bluetooth or a QR code.
//!
//! **Link handshake** `hush/v1/link: NNpsk0 + X-Wing` (J = whoever opens the link, C = its creator)
//! ```text
//! prologue, psk = link keys
//! HS1  J → C   e_J (X-Wing ek)                      MixHash(e_J)
//!              AEAD(commit(N_J) || payload1)        (psk-keyed: proves J holds the link)
//! HS2  C → J   ct = Encap(e_J)                      MixHash(ct), MixKey(ss)
//!              AEAD(N_C || payload2)                (payload2 = SDP offer)
//! HS3  J → C   AEAD(N_J || payload3)                (payload3 = SDP answer); C checks commit(N_J)
//! SAS = f(h, N_C, N_J)                              each nonce fixed before the other is known
//! ```
//!
//! **Contact handshake** `hush/v1/contact: KKpsk0 + X-Wing` (I = initiator, R = responder), between
//! two identities that already hold each other's signed prekeys (`spk`):
//! ```text
//! prologue = ids, psk = contact secret (or the lookup key for a first request)
//! C1  I → R   epoch_R || e_I || ct1 = Encap(spk_R)  MixHash(e_I), MixHash(ct1), MixKey(ss1)
//!             AEAD(payload1)
//! C2  R → I   epoch_I || ct2 = Encap(e_I) || ct3 = Encap(spk_I)   MixKey(ss2), MixKey(ss3)
//!             AEAD(payload2)
//! C3  I → R   AEAD(payload3)
//! ```
//! Only R can read C1 (needs `spk_R`), only I can read C2 (needs `spk_I` and `e_I`), and C3 proves
//! I read C2: mutual authentication without signatures, so transcripts are deniable. `e_I` gives
//! forward secrecy.

use zeroize::Zeroizing;

use crate::identity::Identity;
use crate::kem::{CT_LEN, EK_LEN, KemSecret, encapsulate};
use crate::link::LinkKeys;
use crate::noise::SymmetricState;
use crate::sas::{self, SAS_LEN};
use crate::{Error, Result, Rng};

const LINK_PROTOCOL: &[u8] = b"hush/v1/link:NNpsk0+XWing+SHA256+ChaChaPoly";
const CONTACT_PROTOCOL: &[u8] = b"hush/v1/contact:KKpsk0+XWing+SHA256+ChaChaPoly";

/// The outcome of a handshake: the root of the session's ratchet.
pub struct Established {
    pub root: Zeroizing<[u8; 32]>,
    /// The final transcript hash (binds contact cards to this session).
    pub hash: [u8; 32],
    /// Emoji indices to compare, for link handshakes.
    pub sas: Option<[u8; SAS_LEN]>,
    /// Whether this side sent the first handshake message.
    pub initiator: bool,
}

impl core::fmt::Debug for Established {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Established")
            .field("sas", &self.sas)
            .field("initiator", &self.initiator)
            .finish_non_exhaustive()
    }
}

fn link_state(keys: &LinkKeys) -> SymmetricState {
    let mut ss = SymmetricState::new(LINK_PROTOCOL);
    ss.mix_hash(&keys.prologue());
    ss.mix_key(&*keys.psk);
    ss
}

fn split_nonce(pt: &[u8]) -> Result<([u8; 32], &[u8])> {
    if pt.len() < 32 {
        return Err(Error::Malformed);
    }
    let (n, rest) = pt.split_at(32);
    Ok((n.try_into().map_err(|_| Error::Malformed)?, rest))
}

// ---------------------------------------------------------------------------------------------
// Link handshake

/// Joiner, after sending HS1. Cloneable so a group joiner can complete one handshake per member
/// answering the same HS1.
#[derive(Clone)]
pub struct LinkJoiner {
    ss: SymmetricStateClone,
    kem: KemSecret,
    nonce: Zeroizing<[u8; 32]>,
}

/// Joiner, after reading HS2.
pub struct LinkJoinerReady {
    ss: SymmetricState,
    nonce_c: [u8; 32],
    nonce: Zeroizing<[u8; 32]>,
}

/// Creator, after reading HS1.
pub struct LinkCreatorPending {
    ss: SymmetricState,
    commit_j: [u8; 32],
    ek_j: Vec<u8>,
}

/// Creator, after sending HS2.
pub struct LinkCreator {
    ss: SymmetricState,
    commit_j: [u8; 32],
    nonce_c: [u8; 32],
}

impl LinkJoiner {
    /// Builds HS1.
    pub fn start<R: Rng + ?Sized>(keys: &LinkKeys, payload: &[u8], rng: &mut R) -> (Self, Vec<u8>) {
        let mut ss = link_state(keys);
        let kem = KemSecret::generate(rng);
        let nonce = Zeroizing::new(crate::random::<32, R>(rng));
        let ek = kem.public();
        ss.mix_hash(&ek);
        let mut pt = sas::commit(&nonce).to_vec();
        pt.extend_from_slice(payload);
        let ct = ss.encrypt_and_hash(&pt).expect("keyed by the psk");
        let mut msg = ek;
        msg.extend_from_slice(&ct);
        (
            Self {
                ss: SymmetricStateClone(ss),
                kem,
                nonce,
            },
            msg,
        )
    }

    /// Reads one creator's HS2; returns its payload (the SDP offer).
    ///
    /// # Errors
    /// [`Error::Decrypt`] when HS2 was not made for this HS1 by someone holding the link.
    pub fn read_hs2(&self, hs2: &[u8]) -> Result<(LinkJoinerReady, Vec<u8>)> {
        if hs2.len() < CT_LEN {
            return Err(Error::Malformed);
        }
        let mut ss = self.ss.clone().0;
        let (ct, rest) = hs2.split_at(CT_LEN);
        ss.mix_hash(ct);
        let shared = self.kem.decapsulate(ct)?;
        ss.mix_key(&*shared);
        let pt = ss.decrypt_and_hash(rest)?;
        let (nonce_c, payload) = split_nonce(&pt)?;
        Ok((
            LinkJoinerReady {
                ss,
                nonce_c,
                nonce: self.nonce.clone(),
            },
            payload.to_vec(),
        ))
    }
}

impl LinkJoinerReady {
    /// Builds HS3 (carrying the SDP answer) and finishes.
    #[must_use]
    pub fn write_hs3(mut self, payload: &[u8]) -> (Vec<u8>, Established) {
        let mut pt = self.nonce.to_vec();
        pt.extend_from_slice(payload);
        let msg = self.ss.encrypt_and_hash(&pt).expect("keyed");
        let hash = self.ss.handshake_hash();
        let est = Established {
            root: Zeroizing::new(self.ss.split()),
            hash,
            sas: Some(sas::derive(&hash, &self.nonce_c, &self.nonce)),
            initiator: true,
        };
        (msg, est)
    }
}

impl LinkCreatorPending {
    /// Reads HS1; returns the joiner's first payload.
    ///
    /// # Errors
    /// [`Error::Decrypt`] when the sender does not hold the link (or its passphrase).
    pub fn read_hs1(keys: &LinkKeys, hs1: &[u8]) -> Result<(Self, Vec<u8>)> {
        if hs1.len() < EK_LEN {
            return Err(Error::Malformed);
        }
        let mut ss = link_state(keys);
        let (ek, rest) = hs1.split_at(EK_LEN);
        ss.mix_hash(ek);
        let pt = ss.decrypt_and_hash(rest)?;
        let (commit_j, payload) = split_nonce(&pt)?;
        Ok((
            Self {
                ss,
                commit_j,
                ek_j: ek.to_vec(),
            },
            payload.to_vec(),
        ))
    }

    /// Builds HS2 (carrying the SDP offer).
    ///
    /// # Errors
    /// [`Error::Malformed`] when the joiner's key is invalid.
    pub fn write_hs2<R: Rng + ?Sized>(mut self, payload: &[u8], rng: &mut R) -> Result<(LinkCreator, Vec<u8>)> {
        let (ct, shared) = encapsulate(&self.ek_j, rng)?;
        self.ss.mix_hash(&ct);
        self.ss.mix_key(&*shared);
        let nonce_c: [u8; 32] = crate::random(rng);
        let mut pt = nonce_c.to_vec();
        pt.extend_from_slice(payload);
        let body = self.ss.encrypt_and_hash(&pt)?;
        let mut msg = ct;
        msg.extend_from_slice(&body);
        Ok((
            LinkCreator {
                ss: self.ss,
                commit_j: self.commit_j,
                nonce_c,
            },
            msg,
        ))
    }
}

impl LinkCreator {
    /// Reads HS3 and finishes; returns the joiner's last payload (the SDP answer).
    ///
    /// # Errors
    /// [`Error::Decrypt`] when HS3 does not match, [`Error::BadSignature`] when the revealed nonce
    /// does not match its commitment.
    pub fn read_hs3(mut self, hs3: &[u8]) -> Result<(Vec<u8>, Established)> {
        let pt = self.ss.decrypt_and_hash(hs3)?;
        let (nonce_j, payload) = split_nonce(&pt)?;
        if !crate::kdf::ct_eq(&sas::commit(&nonce_j), &self.commit_j) {
            return Err(Error::BadSignature);
        }
        let hash = self.ss.handshake_hash();
        let est = Established {
            root: Zeroizing::new(self.ss.split()),
            hash,
            sas: Some(sas::derive(&hash, &self.nonce_c, &nonce_j)),
            initiator: false,
        };
        Ok((payload.to_vec(), est))
    }
}

/// `SymmetricState` is deliberately not `Clone` everywhere; the joiner needs it to fork.
struct SymmetricStateClone(SymmetricState);

impl Clone for SymmetricStateClone {
    fn clone(&self) -> Self {
        Self(self.0.fork())
    }
}

// ---------------------------------------------------------------------------------------------
// Contact handshake

/// Static inputs of a contact handshake.
pub struct ContactParams<'a> {
    /// Contact secret, or the lookup key for a first request to an identifier.
    pub psk: &'a [u8; 32],
    /// Both identifier hashes, in any order (sorted internally).
    pub ids: (&'a [u8; 32], &'a [u8; 32]),
}

fn contact_state(p: &ContactParams<'_>) -> SymmetricState {
    let (a, b) = if p.ids.0 <= p.ids.1 {
        (p.ids.0, p.ids.1)
    } else {
        (p.ids.1, p.ids.0)
    };
    let mut ss = SymmetricState::new(CONTACT_PROTOCOL);
    let mut prologue = b"hush/v1/contact/prologue".to_vec();
    prologue.extend_from_slice(a);
    prologue.extend_from_slice(b);
    ss.mix_hash(&prologue);
    ss.mix_key(p.psk);
    ss
}

fn read_epoch(b: &[u8]) -> Result<u32> {
    Ok(u32::from_be_bytes(
        b.get(..4)
            .ok_or(Error::Malformed)?
            .try_into()
            .map_err(|_| Error::Malformed)?,
    ))
}

/// Initiator, after sending C1.
pub struct ContactInitiator {
    ss: SymmetricState,
    e: KemSecret,
}

/// Initiator, after reading C2.
pub struct ContactInitiatorReady {
    ss: SymmetricState,
}

/// Responder, after reading C1.
pub struct ContactResponderPending {
    ss: SymmetricState,
    ek_i: Vec<u8>,
}

/// Responder, after sending C2.
pub struct ContactResponder {
    ss: SymmetricState,
}

impl ContactInitiator {
    /// Builds C1 to a responder whose prekey for `peer_epoch` is `peer_spk`.
    ///
    /// # Errors
    /// [`Error::Malformed`] when the prekey is invalid.
    pub fn start<R: Rng + ?Sized>(
        params: &ContactParams<'_>,
        peer_spk: &[u8],
        peer_epoch: u32,
        payload: &[u8],
        rng: &mut R,
    ) -> Result<(Self, Vec<u8>)> {
        let mut ss = contact_state(params);
        let e = KemSecret::generate(rng);
        let epoch = peer_epoch.to_be_bytes();
        ss.mix_hash(&epoch);
        let ek = e.public();
        ss.mix_hash(&ek);
        let (ct1, ss1) = encapsulate(peer_spk, rng)?;
        ss.mix_hash(&ct1);
        ss.mix_key(&*ss1);
        let body = ss.encrypt_and_hash(payload)?;
        let mut msg = epoch.to_vec();
        msg.extend_from_slice(&ek);
        msg.extend_from_slice(&ct1);
        msg.extend_from_slice(&body);
        Ok((Self { ss, e }, msg))
    }

    /// Reads C2, decapsulating with our own prekey for the epoch R chose.
    ///
    /// # Errors
    /// [`Error::Decrypt`] when C2 is not from the expected responder.
    pub fn read_c2(
        mut self,
        me: &Identity,
        accept_epoch: impl Fn(u32) -> bool,
        c2: &[u8],
    ) -> Result<(ContactInitiatorReady, Vec<u8>)> {
        if c2.len() < 4 + 2 * CT_LEN {
            return Err(Error::Malformed);
        }
        let epoch = read_epoch(c2)?;
        if !accept_epoch(epoch) {
            return Err(Error::Expired);
        }
        self.ss.mix_hash(&c2[..4]);
        let ct2 = &c2[4..4 + CT_LEN];
        let ct3 = &c2[4 + CT_LEN..4 + 2 * CT_LEN];
        self.ss.mix_hash(ct2);
        self.ss.mix_key(&*self.e.decapsulate(ct2)?);
        self.ss.mix_hash(ct3);
        self.ss.mix_key(&*me.spk(epoch).decapsulate(ct3)?);
        let payload = self.ss.decrypt_and_hash(&c2[4 + 2 * CT_LEN..])?;
        Ok((ContactInitiatorReady { ss: self.ss }, payload))
    }
}

impl ContactInitiatorReady {
    /// Builds C3 and finishes.
    #[must_use]
    pub fn write_c3(mut self, payload: &[u8]) -> (Vec<u8>, Established) {
        let msg = self.ss.encrypt_and_hash(payload).expect("keyed");
        let est = Established {
            root: Zeroizing::new(self.ss.split()),
            hash: self.ss.handshake_hash(),
            sas: None,
            initiator: true,
        };
        (msg, est)
    }
}

impl ContactResponderPending {
    /// Reads C1 with our prekey for the epoch the initiator used.
    ///
    /// # Errors
    /// [`Error::Expired`] for an epoch the caller refuses, [`Error::Decrypt`] when C1 was not made
    /// for us with the right secret.
    pub fn read_c1(
        params: &ContactParams<'_>,
        me: &Identity,
        accept_epoch: impl Fn(u32) -> bool,
        c1: &[u8],
    ) -> Result<(Self, Vec<u8>)> {
        if c1.len() < 4 + EK_LEN + CT_LEN {
            return Err(Error::Malformed);
        }
        let epoch = read_epoch(c1)?;
        if !accept_epoch(epoch) {
            return Err(Error::Expired);
        }
        let mut ss = contact_state(params);
        ss.mix_hash(&c1[..4]);
        let ek = &c1[4..4 + EK_LEN];
        ss.mix_hash(ek);
        let ct1 = &c1[4 + EK_LEN..4 + EK_LEN + CT_LEN];
        ss.mix_hash(ct1);
        ss.mix_key(&*me.spk(epoch).decapsulate(ct1)?);
        let payload = ss.decrypt_and_hash(&c1[4 + EK_LEN + CT_LEN..])?;
        Ok((Self { ss, ek_i: ek.to_vec() }, payload))
    }

    /// Builds C2 to an initiator whose prekey for `peer_epoch` is `peer_spk`.
    ///
    /// # Errors
    /// [`Error::Malformed`] when a key is invalid.
    pub fn write_c2<R: Rng + ?Sized>(
        mut self,
        peer_spk: &[u8],
        peer_epoch: u32,
        payload: &[u8],
        rng: &mut R,
    ) -> Result<(ContactResponder, Vec<u8>)> {
        let epoch = peer_epoch.to_be_bytes();
        self.ss.mix_hash(&epoch);
        let (ct2, ss2) = encapsulate(&self.ek_i, rng)?;
        self.ss.mix_hash(&ct2);
        self.ss.mix_key(&*ss2);
        let (ct3, ss3) = encapsulate(peer_spk, rng)?;
        self.ss.mix_hash(&ct3);
        self.ss.mix_key(&*ss3);
        let body = self.ss.encrypt_and_hash(payload)?;
        let mut msg = epoch.to_vec();
        msg.extend_from_slice(&ct2);
        msg.extend_from_slice(&ct3);
        msg.extend_from_slice(&body);
        Ok((ContactResponder { ss: self.ss }, msg))
    }
}

impl ContactResponder {
    /// Reads C3 and finishes.
    ///
    /// # Errors
    /// [`Error::Decrypt`] when the initiator could not read C2.
    pub fn read_c3(mut self, c3: &[u8]) -> Result<(Vec<u8>, Established)> {
        let payload = self.ss.decrypt_and_hash(c3)?;
        let est = Established {
            root: Zeroizing::new(self.ss.split()),
            hash: self.ss.handshake_hash(),
            sas: None,
            initiator: false,
        };
        Ok((payload, est))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::{Link, Mode};
    use crate::vault::KdfParams;

    fn link_keys() -> (Link, LinkKeys) {
        let mut rng = crate::test_rng::rng();
        let link = Link::generate(Mode::Direct, 0, 60, false, &mut rng);
        let keys = link.keys(None).unwrap();
        (link, keys)
    }

    #[test]
    fn link_handshake_agrees() {
        let mut rng = crate::test_rng::rng();
        let (link, kc) = link_keys();
        let kj = link.keys(None).unwrap();
        let (joiner, hs1) = LinkJoiner::start(&kj, b"nick:bob", &mut rng);
        let (pending, p1) = LinkCreatorPending::read_hs1(&kc, &hs1).unwrap();
        assert_eq!(p1, b"nick:bob");
        let (creator, hs2) = pending.write_hs2(b"offer", &mut rng).unwrap();
        let (ready, p2) = joiner.read_hs2(&hs2).unwrap();
        assert_eq!(p2, b"offer");
        let (hs3, ej) = ready.write_hs3(b"answer");
        let (p3, ec) = creator.read_hs3(&hs3).unwrap();
        assert_eq!(p3, b"answer");
        assert_eq!(*ej.root, *ec.root);
        assert_eq!(ej.hash, ec.hash);
        assert_eq!(ej.sas, ec.sas);
        assert!(ej.initiator && !ec.initiator);
    }

    #[test]
    fn wrong_link_or_passphrase_fails() {
        let mut rng = crate::test_rng::rng();
        let (_, kc) = link_keys();
        let (_, other) = link_keys();
        let (_, hs1) = LinkJoiner::start(&other, b"", &mut rng);
        assert_eq!(LinkCreatorPending::read_hs1(&kc, &hs1).err(), Some(Error::Decrypt));

        let link = Link::generate(Mode::Direct, 0, 60, true, &mut rng);
        let cheap = KdfParams { m_kib: 64, t: 1, p: 1 };
        let good = link.keys_with(Some("pass"), cheap).unwrap();
        let bad = link.keys_with(Some("Pass"), cheap).unwrap();
        let (_, hs1) = LinkJoiner::start(&bad, b"", &mut rng);
        assert!(LinkCreatorPending::read_hs1(&good, &hs1).is_err());
    }

    #[test]
    fn man_in_the_middle_gets_different_sas() {
        // Mallory holds the link and sits between Joiner and Creator.
        let mut rng = crate::test_rng::rng();
        let (link, kc) = link_keys();
        let kj = link.keys(None).unwrap();
        let km = link.keys(None).unwrap();

        let (joiner, hs1_j) = LinkJoiner::start(&kj, b"", &mut rng);
        // Mallory answers the joiner as a creator...
        let (m_pending, _) = LinkCreatorPending::read_hs1(&km, &hs1_j).unwrap();
        let (m_creator, hs2_m) = m_pending.write_hs2(b"", &mut rng).unwrap();
        let (ready, _) = joiner.read_hs2(&hs2_m).unwrap();
        let (hs3_j, e_j) = ready.write_hs3(b"");
        let (_, e_mj) = m_creator.read_hs3(&hs3_j).unwrap();
        // ...and joins the creator as a joiner.
        let (m_joiner, hs1_m) = LinkJoiner::start(&km, b"", &mut rng);
        let (c_pending, _) = LinkCreatorPending::read_hs1(&kc, &hs1_m).unwrap();
        let (creator, hs2_c) = c_pending.write_hs2(b"", &mut rng).unwrap();
        let (m_ready, _) = m_joiner.read_hs2(&hs2_c).unwrap();
        let (hs3_m, e_mc) = m_ready.write_hs3(b"");
        let (_, e_c) = creator.read_hs3(&hs3_m).unwrap();

        assert_eq!(e_j.sas, e_mj.sas);
        assert_eq!(e_c.sas, e_mc.sas);
        assert_ne!(
            e_j.sas, e_c.sas,
            "the two people see different emojis (1 in 2^30 chance otherwise)"
        );
        assert_ne!(*e_j.root, *e_c.root);
    }

    #[test]
    fn tampered_messages_fail() {
        let mut rng = crate::test_rng::rng();
        let (link, kc) = link_keys();
        let kj = link.keys(None).unwrap();
        let (joiner, hs1) = LinkJoiner::start(&kj, b"", &mut rng);
        let (pending, _) = LinkCreatorPending::read_hs1(&kc, &hs1).unwrap();
        let (creator, mut hs2) = pending.write_hs2(b"offer", &mut rng).unwrap();
        hs2[5] ^= 1;
        assert!(joiner.read_hs2(&hs2).is_err());
        hs2[5] ^= 1;
        let (ready, _) = joiner.read_hs2(&hs2).unwrap();
        let (mut hs3, _) = ready.write_hs3(b"");
        hs3[0] ^= 1;
        assert!(creator.read_hs3(&hs3).is_err());
    }

    #[test]
    fn group_joiner_forks_per_member() {
        let mut rng = crate::test_rng::rng();
        let (link, kc) = link_keys();
        let kj = link.keys(None).unwrap();
        let (joiner, hs1) = LinkJoiner::start(&kj, b"", &mut rng);
        let mut roots = Vec::new();
        for _ in 0..3 {
            let (pending, _) = LinkCreatorPending::read_hs1(&kc, &hs1).unwrap();
            let (creator, hs2) = pending.write_hs2(b"", &mut rng).unwrap();
            let (ready, _) = joiner.read_hs2(&hs2).unwrap();
            let (hs3, ej) = ready.write_hs3(b"");
            let (_, ec) = creator.read_hs3(&hs3).unwrap();
            assert_eq!(*ej.root, *ec.root);
            roots.push(*ej.root);
        }
        assert_ne!(roots[0], roots[1]);
        assert_ne!(roots[1], roots[2]);
    }

    #[test]
    fn contact_handshake_authenticates_both() {
        let mut rng = crate::test_rng::rng();
        let alice = Identity::generate(&mut rng);
        let bob = Identity::generate(&mut rng);
        let eve = Identity::generate(&mut rng);
        let psk = [5u8; 32];
        let params = ContactParams {
            psk: &psk,
            ids: (alice.public().id_hash(), bob.public().id_hash()),
        };
        let any = |_: u32| true;

        let (init, c1) = ContactInitiator::start(&params, &bob.spk(10).public(), 10, b"hi", &mut rng).unwrap();
        assert!(
            ContactResponderPending::read_c1(&params, &eve, any, &c1).is_err(),
            "only Bob reads C1"
        );
        let (pending, p1) = ContactResponderPending::read_c1(&params, &bob, any, &c1).unwrap();
        assert_eq!(p1, b"hi");
        let (resp, c2) = pending
            .write_c2(&alice.spk(11).public(), 11, b"offer", &mut rng)
            .unwrap();
        let (ready, p2) = init.read_c2(&alice, any, &c2).unwrap();
        assert_eq!(p2, b"offer");
        let (c3, ea) = ready.write_c3(b"answer");
        let (p3, eb) = resp.read_c3(&c3).unwrap();
        assert_eq!(p3, b"answer");
        assert_eq!(*ea.root, *eb.root);

        // Bob encapsulating to Eve's prekey instead of Alice's: Alice cannot read C2.
        let (init, c1) = ContactInitiator::start(&params, &bob.spk(10).public(), 10, b"", &mut rng).unwrap();
        let (pending, _) = ContactResponderPending::read_c1(&params, &bob, any, &c1).unwrap();
        let (_, c2) = pending.write_c2(&eve.spk(11).public(), 11, b"", &mut rng).unwrap();
        assert!(init.read_c2(&alice, any, &c2).is_err());

        // Wrong contact secret.
        let other = [6u8; 32];
        let wrong = ContactParams {
            psk: &other,
            ids: params.ids,
        };
        let (_, c1) = ContactInitiator::start(&wrong, &bob.spk(10).public(), 10, b"", &mut rng).unwrap();
        assert!(ContactResponderPending::read_c1(&params, &bob, any, &c1).is_err());
        // Refused epoch.
        let (_, c1) = ContactInitiator::start(&params, &bob.spk(10).public(), 10, b"", &mut rng).unwrap();
        assert_eq!(
            ContactResponderPending::read_c1(&params, &bob, |e| e == 11, &c1).err(),
            Some(Error::Expired)
        );
    }
}
