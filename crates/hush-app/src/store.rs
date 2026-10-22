//! The vault on disk, as `web/src/app/store.ts` keeps it in IndexedDB: nothing is written unless
//! the user creates one, and then only records sealed by the core under opaque HMAC names. The
//! records and backups are the same as the web app's, so a backup moves freely between the web
//! app, the CLI and the phones.
//!
//! ```text
//! <dir>/vault/header        the data key, wrapped by Argon2id(passphrase)
//! <dir>/vault/<index>       one sealed record each (identity, contact index, contacts, history)
//! ```

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use hush_core::bundle::Bundle;
use hush_core::identity::Identity;
use hush_core::vault::{KdfParams, Vault};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::text::clean_name;
use crate::util::{b64u, from_b64u, now_ms, rng};

/// Kept history per contact, at most.
pub const MAX_HISTORY: usize = 2000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contact {
    /// `hush1…`
    pub id: String,
    /// Hex of the identifier hash.
    pub key: String,
    pub name: String,
    /// Latest signed bundle (base64url).
    pub bundle: String,
    /// Contact secret from the session where cards were exchanged (base64url).
    pub secret: String,
    pub added: u64,
    pub history: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    pub mine: bool,
    pub ts: u64,
    pub text: String,
    #[serde(rename = "expiresAt", skip_serializing_if = "Option::is_none", default)]
    pub expires_at: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredIdentity {
    seed: String,
    name: String,
}

/// What a backup holds (the web app's `Snapshot`).
#[derive(Debug, Serialize, Deserialize)]
struct Snapshot {
    v: u8,
    identity: Option<StoredIdentity>,
    contacts: Vec<Contact>,
    history: std::collections::BTreeMap<String, Vec<HistoryEntry>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VaultState {
    /// No directory to keep anything in, or no vault created.
    None,
    Locked,
    Unlocked,
}

pub struct Store {
    dir: Option<PathBuf>,
    vault: Option<Vault>,
    identity: Option<Identity>,
    contacts: Vec<Contact>,
}

fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        let mut f = opts.open(&tmp)?;
        std::io::Write::write_all(&mut f, data)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

impl Store {
    /// A store kept under `dir`, or in memory only (`None`: nothing can be saved).
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self {
            dir,
            vault: None,
            identity: None,
            contacts: Vec::new(),
        }
    }

    fn vault_dir(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join("vault"))
    }

    pub fn state(&self) -> VaultState {
        if self.vault.is_some() {
            VaultState::Unlocked
        } else if self.vault_dir().is_some_and(|d| d.join("header").exists()) {
            VaultState::Locked
        } else {
            VaultState::None
        }
    }

    pub fn is_unlocked(&self) -> bool {
        self.vault.is_some()
    }

    fn need(&self) -> Result<&Vault> {
        self.vault.as_ref().ok_or_else(|| anyhow!("the vault is locked"))
    }

    fn record_path(&self, kind: &str, id: &str) -> Result<PathBuf> {
        let dir = self.vault_dir().ok_or_else(|| anyhow!("no storage"))?;
        Ok(dir.join(self.need()?.index(kind, id)))
    }

    fn read_record<T: DeserializeOwned>(&self, kind: &str, id: &str) -> Result<Option<T>> {
        let path = self.record_path(kind, id)?;
        let blob = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let plain = self.need()?.open(kind, id, &blob).map_err(|e| anyhow!("{e}"))?;
        Ok(Some(serde_json::from_slice(&plain)?))
    }

    fn write_record<T: Serialize>(&self, kind: &str, id: &str, value: &T) -> Result<()> {
        let path = self.record_path(kind, id)?;
        let sealed = self.need()?.seal(kind, id, &serde_json::to_vec(value)?, &mut rng());
        write_private(&path, &sealed)
    }

    fn delete_record(&self, kind: &str, id: &str) -> Result<()> {
        match std::fs::remove_file(self.record_path(kind, id)?) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    pub fn create(&mut self, passphrase: &str) -> Result<()> {
        let dir = self.vault_dir().ok_or_else(|| anyhow!("no storage"))?;
        if dir.join("header").exists() {
            bail!("a vault already exists");
        }
        std::fs::create_dir_all(&dir)?;
        let (vault, header) = Vault::create(passphrase, KdfParams::DEFAULT, &mut rng()).map_err(|e| anyhow!("{e}"))?;
        write_private(&dir.join("header"), &header)?;
        self.vault = Some(vault);
        self.write_record("index", "contacts", &Vec::<String>::new())?;
        self.load_all()
    }

    /// Fails with "Decrypt" for a wrong passphrase.
    pub fn unlock(&mut self, passphrase: &str) -> Result<()> {
        let dir = self.vault_dir().ok_or_else(|| anyhow!("no storage"))?;
        let header = std::fs::read(dir.join("header")).context("no vault")?;
        self.vault = Some(Vault::unlock(passphrase, &header).map_err(|e| anyhow!("{e:?}"))?);
        self.load_all()
    }

    pub fn lock(&mut self) {
        self.vault = None;
        self.identity = None;
        self.contacts.clear();
    }

    pub fn erase(&mut self) -> Result<()> {
        self.lock();
        if let Some(dir) = self.vault_dir()
            && dir.exists()
        {
            std::fs::remove_dir_all(dir)?;
        }
        Ok(())
    }

    pub fn change_passphrase(&self, passphrase: &str) -> Result<()> {
        let header = self
            .need()?
            .header(passphrase, KdfParams::DEFAULT, &mut rng())
            .map_err(|e| anyhow!("{e}"))?;
        let dir = self.vault_dir().ok_or_else(|| anyhow!("no storage"))?;
        write_private(&dir.join("header"), &header)
    }

    fn load_all(&mut self) -> Result<()> {
        self.identity = self
            .read_record::<StoredIdentity>("identity", "me")?
            .and_then(|me| from_b64u(&me.seed))
            .and_then(|s| <[u8; 32]>::try_from(s.as_slice()).ok())
            .map(Identity::from_seed);
        let keys: Vec<String> = self.read_record("index", "contacts")?.unwrap_or_default();
        self.contacts = keys
            .iter()
            .filter_map(|k| self.read_record::<Contact>("contact", k).ok().flatten())
            .collect();
        Ok(())
    }

    // ── identity and contacts ───────────────────────────────────────────────

    pub fn identity(&self) -> Option<&Identity> {
        self.identity.as_ref().filter(|_| self.vault.is_some())
    }

    /// Our identity, created on first use (needs the vault).
    pub fn ensure_identity(&mut self, name: &str) -> Result<&Identity> {
        if self.identity.is_none() {
            let id = Identity::generate(&mut rng());
            self.write_record(
                "identity",
                "me",
                &StoredIdentity {
                    seed: b64u(id.seed()),
                    name: name.to_string(),
                },
            )?;
            self.identity = Some(id);
        }
        Ok(self.identity.as_ref().expect("just set"))
    }

    pub fn contacts(&self) -> &[Contact] {
        &self.contacts
    }

    pub fn contact(&self, key: &str) -> Option<&Contact> {
        self.contacts.iter().find(|c| c.key == key)
    }

    fn write_index(&self) -> Result<()> {
        let keys: Vec<&str> = self.contacts.iter().map(|c| c.key.as_str()).collect();
        self.write_record("index", "contacts", &keys)
    }

    pub fn save_contact(&mut self, c: Contact) -> Result<()> {
        self.write_record("contact", &c.key, &c)?;
        self.contacts.retain(|x| x.key != c.key);
        self.contacts.push(c);
        self.write_index()
    }

    pub fn remove_contact(&mut self, key: &str) -> Result<()> {
        self.delete_record("contact", key)?;
        self.delete_record("history", key)?;
        self.contacts.retain(|x| x.key != key);
        self.write_index()
    }

    // ── history ─────────────────────────────────────────────────────────────

    pub fn history(&self, key: &str) -> Result<Vec<HistoryEntry>> {
        let now = now_ms();
        Ok(self
            .read_record::<Vec<HistoryEntry>>("history", key)?
            .unwrap_or_default()
            .into_iter()
            .filter(|e| e.expires_at.is_none_or(|t| t > now))
            .collect())
    }

    pub fn append_history(&self, key: &str, entry: HistoryEntry) -> Result<()> {
        let mut list = self.history(key)?;
        list.push(entry);
        let start = list.len().saturating_sub(MAX_HISTORY);
        self.write_record("history", key, &list[start..].to_vec())
    }

    pub fn remove_history(&self, key: &str, id: &str) -> Result<()> {
        let list = self.history(key)?;
        let kept: Vec<HistoryEntry> = list.iter().filter(|e| e.id != id).cloned().collect();
        if kept.len() != list.len() {
            self.write_record("history", key, &kept)?;
        }
        Ok(())
    }

    pub fn clear_history(&self, key: &str) -> Result<()> {
        self.delete_record("history", key)
    }

    // ── backups ─────────────────────────────────────────────────────────────

    pub fn export_backup(&self, passphrase: &str) -> Result<Vec<u8>> {
        let me: Option<StoredIdentity> = self.read_record("identity", "me")?;
        let mut history = std::collections::BTreeMap::new();
        for c in &self.contacts {
            let h = self.history(&c.key)?;
            if !h.is_empty() {
                history.insert(c.key.clone(), h);
            }
        }
        let snap = Snapshot {
            v: 1,
            identity: me,
            contacts: self.contacts.clone(),
            history,
        };
        hush_core::backup::export(passphrase, KdfParams::DEFAULT, &serde_json::to_vec(&snap)?, &mut rng())
            .map_err(|e| anyhow!("{e}"))
    }

    /// Merges a backup into the unlocked vault; returns how many contacts it restored. Fails
    /// with "Decrypt" for a wrong passphrase.
    pub fn import_backup(&mut self, file: &[u8], passphrase: &str) -> Result<usize> {
        self.need()?;
        let plain = hush_core::backup::import(passphrase, file).map_err(|e| anyhow!("{e:?}"))?;
        let snap: Snapshot = serde_json::from_slice(&plain).map_err(|_| anyhow!("Malformed"))?;
        if snap.v != 1 {
            bail!("Malformed");
        }
        if let Some(me) = &snap.identity
            && self.identity.is_none()
            && let Some(seed) = from_b64u(&me.seed).and_then(|s| <[u8; 32]>::try_from(s.as_slice()).ok())
        {
            self.write_record("identity", "me", me)?;
            self.identity = Some(Identity::from_seed(seed));
        }
        let mut n = 0;
        for c in snap.contacts {
            // A backup is untrusted input like any other: every bundle is verified again.
            let ok = from_b64u(&c.bundle)
                .and_then(|b| Bundle::parse_stored(&b).ok())
                .is_some_and(|b| b.identity.id() == c.id && hex::encode(b.id_hash()) == c.key)
                && from_b64u(&c.secret).is_some_and(|s| s.len() == 32);
            if !ok {
                continue;
            }
            let key = c.key.clone();
            self.save_contact(Contact {
                name: clean_name(&c.name),
                ..c
            })?;
            if let Some(h) = snap.history.get(&key) {
                let start = h.len().saturating_sub(MAX_HISTORY);
                self.write_record("history", &key, &h[start..].to_vec())?;
            }
            n += 1;
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(Some(dir.path().to_path_buf()));
        (dir, s)
    }

    #[test]
    fn vault_lifecycle_and_records() {
        let (_d, mut s) = store();
        assert_eq!(s.state(), VaultState::None);
        s.create("a long passphrase").unwrap();
        assert_eq!(s.state(), VaultState::Unlocked);
        let id = s.ensure_identity("Alice").unwrap().public().id();
        s.save_contact(Contact {
            id: "x".into(),
            key: "ab".into(),
            name: "Bob".into(),
            bundle: String::new(),
            secret: String::new(),
            added: 1,
            history: true,
        })
        .unwrap();
        s.append_history(
            "ab",
            HistoryEntry {
                id: "m1".into(),
                mine: true,
                ts: 1,
                text: "hi".into(),
                expires_at: None,
            },
        )
        .unwrap();
        s.lock();
        assert_eq!(s.state(), VaultState::Locked);
        assert!(s.unlock("wrong").is_err());
        s.unlock("a long passphrase").unwrap();
        assert_eq!(s.identity().unwrap().public().id(), id);
        assert_eq!(s.contacts()[0].name, "Bob");
        assert_eq!(s.history("ab").unwrap()[0].text, "hi");
        s.remove_history("ab", "m1").unwrap();
        assert!(s.history("ab").unwrap().is_empty());
        s.change_passphrase("another passphrase").unwrap();
        s.lock();
        s.unlock("another passphrase").unwrap();
        // Nothing on disk shows a contact key or a name.
        for f in std::fs::read_dir(s.vault_dir().unwrap()).unwrap() {
            let name = f.unwrap().file_name().into_string().unwrap();
            assert!(name == "header" || name.len() == 32, "{name}");
        }
        s.erase().unwrap();
        assert_eq!(s.state(), VaultState::None);
    }

    #[test]
    fn backups_round_trip_and_drop_bad_contacts() {
        let (_d, mut a) = store();
        a.create("pass one").unwrap();
        let bob = Identity::generate(&mut rng());
        let bundle = Bundle::create(&bob, "Bob\u{202e}", now_ms(), &mut rng());
        let good = Contact {
            id: bob.public().id(),
            key: hex::encode(bob.public().id_hash()),
            name: "Bob\u{202e}".into(),
            bundle: b64u(&bundle.bytes),
            secret: b64u(&[7u8; 32]),
            added: 1,
            history: false,
        };
        a.save_contact(good.clone()).unwrap();
        a.save_contact(Contact {
            key: "00".into(),
            ..good.clone()
        })
        .unwrap();
        let file = a.export_backup("backup pass").unwrap();

        let (_e, mut b) = store();
        b.create("pass two").unwrap();
        assert!(b.import_backup(&file, "nope").is_err());
        assert_eq!(
            b.import_backup(&file, "backup pass").unwrap(),
            1,
            "the forged key is dropped"
        );
        assert_eq!(b.contacts()[0].name, "Bob");
    }
}
