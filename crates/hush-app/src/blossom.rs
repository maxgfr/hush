//! Blossom blob servers (BUD-01, BUD-02): where sealed drops wait for their recipients.
//!
//! A blob is addressed by the SHA-256 of its bytes, so a server cannot change what it serves
//! without being caught, and every blob is checked when it comes back. Uploads and deletions are
//! authorized by a Nostr event (kind 24242) signed with the drop's throwaway key: no account, and
//! nothing ties two drops together. Servers only ever see ciphertext.

use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use k256::schnorr::SigningKey;
use serde_json::{Value, json};

use crate::util::{now_ms, random};
use hush_core::link::Locator;

/// Servers that accept anonymous uploads of opaque blobs, with CORS for the web app (checked
/// October 2026: upload, fetch and delete with a throwaway key).
pub const DEFAULT_SERVERS: [&str; 3] = ["nostr.download", "blossom.yakihonne.com", "files.sovbit.host"];
/// How many servers keep a copy of each blob.
pub const COPIES: usize = 2;
const AUTH_KIND: u64 = 24_242;
const TIMEOUT: Duration = Duration::from_secs(120);
const TRIES: usize = 3;

/// A Blossom client.
#[derive(Clone)]
pub struct Blossom {
    http: reqwest::Client,
}

impl Default for Blossom {
    fn default() -> Self {
        Self::new()
    }
}

/// The throwaway signing key of a drop, from the 32 bytes its manifest keeps.
///
/// # Errors
/// When the bytes are not a valid secp256k1 secret (probability about 2⁻¹²⁸).
pub fn signing_key(secret: &[u8; 32]) -> Result<SigningKey> {
    SigningKey::from_bytes(secret).map_err(|_| anyhow!("invalid drop key"))
}

/// A fresh throwaway key, as the 32 bytes to keep in a manifest.
#[must_use]
pub fn new_key() -> [u8; 32] {
    loop {
        let k = random::<32>();
        if SigningKey::from_bytes(&k).is_ok() {
            return k;
        }
    }
}

/// `Authorization: Nostr <base64(event)>` for one verb on one blob.
fn auth(sk: &SigningKey, verb: &str, sha: &str) -> String {
    let pubkey = hex::encode(sk.verifying_key().to_bytes());
    let created_at = now_ms() / 1000;
    let tags = json!([["t", verb], ["x", sha], ["expiration", (created_at + 3600).to_string()]]);
    let serial = json!([0, pubkey, created_at, AUTH_KIND, tags, verb]).to_string();
    let id = hush_core::kdf::sha256(&[serial.as_bytes()]);
    let sig = sk.sign_raw(&id, &random::<32>()).expect("schnorr signing");
    let event = json!({"id": hex::encode(id), "pubkey": pubkey, "created_at": created_at, "kind": AUTH_KIND, "tags": tags, "content": verb, "sig": hex::encode(sig.to_bytes())});
    format!("Nostr {}", base64_std(event.to_string().as_bytes()))
}

fn base64_std(data: &[u8]) -> String {
    // Standard alphabet with padding, as the header expects (the core only does base64url).
    hush_core::b64::encode(data).replace('-', "+").replace('_', "/") + ["", "==", "="][data.len() % 3]
}

fn reason(resp: &reqwest::Response) -> String {
    resp.headers()
        .get("x-reason")
        .and_then(|v| v.to_str().ok())
        .map(|r| format!(": {}", r.chars().take(120).collect::<String>()))
        .unwrap_or_default()
}

impl Blossom {
    #[must_use]
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .connect_timeout(Duration::from_secs(15))
            .user_agent(concat!("hush/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("HTTP client");
        Self { http }
    }

    async fn put_once(&self, host: &str, data: &[u8], sha: &str, sk: &SigningKey) -> Result<()> {
        let resp = self
            .http
            .put(format!("{}/upload", Locator::server_url(host)))
            .header("Authorization", auth(sk, "upload", sha))
            .header("Content-Type", "application/octet-stream")
            .header("X-SHA-256", sha)
            .body(data.to_vec())
            .send()
            .await?;
        if !resp.status().is_success() {
            bail!(
                "{host} refused the upload ({}{})",
                resp.status().as_u16(),
                reason(&resp)
            );
        }
        // The descriptor must name the blob we sent.
        let body: Value = resp
            .bytes()
            .await
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(Value::Null);
        if body["sha256"].as_str().is_some_and(|s| s != sha) {
            bail!("{host} stored something else");
        }
        Ok(())
    }

    /// Uploads a blob to one server, retrying.
    ///
    /// # Errors
    /// The last failure.
    pub async fn put(&self, host: &str, data: &[u8], sk: &SigningKey) -> Result<()> {
        let sha = hex::encode(hush_core::kdf::sha256(&[data]));
        let mut last = anyhow!("no attempt");
        for attempt in 0..TRIES {
            match self.put_once(host, data, &sha, sk).await {
                Ok(()) => return Ok(()),
                Err(e) => last = e,
            }
            tokio::time::sleep(Duration::from_millis(500 << attempt)).await;
        }
        Err(last)
    }

    /// Uploads a blob to `want` servers among `hosts`, in order; returns those that took it.
    ///
    /// # Errors
    /// When no server took it.
    pub async fn put_any(&self, hosts: &[String], want: usize, data: &[u8], sk: &SigningKey) -> Result<Vec<String>> {
        let mut ok = Vec::new();
        let mut errors = Vec::new();
        for h in hosts {
            if ok.len() >= want {
                break;
            }
            match self.put(h, data, sk).await {
                Ok(()) => ok.push(h.clone()),
                Err(e) => errors.push(e.to_string()),
            }
        }
        if ok.is_empty() {
            bail!("no server took the blob: {}", errors.join("; "));
        }
        Ok(ok)
    }

    /// Fetches a blob from the first of `hosts` that has it, checking its SHA-256.
    ///
    /// # Errors
    /// When no server returns the right bytes.
    pub async fn get(&self, hosts: &[String], sha: &[u8; 32], max: usize) -> Result<Vec<u8>> {
        let hex_sha = hex::encode(sha);
        let mut errors = Vec::new();
        for attempt in 0..TRIES {
            for h in hosts {
                match self.get_once(h, &hex_sha, sha, max).await {
                    Ok(b) => return Ok(b),
                    Err(e) => errors.push(e.to_string()),
                }
            }
            tokio::time::sleep(Duration::from_millis(500 << attempt)).await;
        }
        errors.dedup();
        bail!("{}", errors.join("; "))
    }

    async fn get_once(&self, host: &str, hex_sha: &str, sha: &[u8; 32], max: usize) -> Result<Vec<u8>> {
        let mut resp = self
            .http
            .get(format!("{}/{hex_sha}", Locator::server_url(host)))
            .send()
            .await?;
        if !resp.status().is_success() {
            bail!("{host}: {}{}", resp.status().as_u16(), reason(&resp));
        }
        if resp.content_length().is_some_and(|n| n > max as u64) {
            bail!("{host}: blob too large");
        }
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await? {
            if body.len() + chunk.len() > max {
                bail!("{host}: blob too large");
            }
            body.extend_from_slice(&chunk);
        }
        if hush_core::kdf::sha256(&[&body]) != *sha {
            bail!("{host} returned other bytes");
        }
        Ok(body)
    }

    /// Asks every server to delete a blob. Best effort: returns how many confirmed.
    pub async fn delete(&self, hosts: &[String], sha: &[u8; 32], sk: &SigningKey) -> usize {
        let hex_sha = hex::encode(sha);
        let mut n = 0;
        for h in hosts {
            let resp = self
                .http
                .delete(format!("{}/{hex_sha}", Locator::server_url(h)))
                .header("Authorization", auth(sk, "delete", &hex_sha))
                .send()
                .await;
            if resp.is_ok_and(|r| r.status().is_success() || r.status().as_u16() == 404) {
                n += 1;
            }
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard_alphabet() {
        assert_eq!(base64_std(b""), "");
        assert_eq!(base64_std(b"f"), "Zg==");
        assert_eq!(base64_std(b"fo"), "Zm8=");
        assert_eq!(base64_std(b"foo"), "Zm9v");
        assert_eq!(base64_std(&[0xfb, 0xff]), "+/8=");
    }

    #[test]
    fn auth_event_is_signed_for_its_blob() {
        let sk = signing_key(&new_key()).unwrap();
        let header = auth(&sk, "upload", &"ab".repeat(32));
        let json = header.strip_prefix("Nostr ").unwrap();
        let std = json.replace('+', "-").replace('/', "_");
        let raw = hush_core::b64::decode(std.trim_end_matches('=')).unwrap();
        let ev: Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(ev["kind"], 24_242);
        assert_eq!(ev["tags"][1][1], "ab".repeat(32));
        let serial = json!([0, ev["pubkey"], ev["created_at"], 24_242, ev["tags"], "upload"]).to_string();
        assert_eq!(ev["id"], hex::encode(hush_core::kdf::sha256(&[serial.as_bytes()])));
    }
}
