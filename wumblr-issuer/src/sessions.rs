//! Community PDS session custody.
//!
//! When the issuer provisions a community account on ePDS, it gets back a
//! DID + handle + accessJwt + refreshJwt. The JWTs are the community's PDS
//! credentials — the issuer is the sole custodian, since a community account
//! has no human to log in. We persist them in a local SQLite file
//! (`issuer.db`) so they survive restarts, encrypting the tokens at rest
//! with AES-256-GCM.
//!
//! The encryption key is a single 32-byte secret from
//! WUMBLR_ISSUER_SESSION_KEY_B64 (base64url, no padding). One key, one
//! purpose; no HKDF derivation — there is exactly one kind of secret stored
//! here. Losing the key makes every stored session unrecoverable, same
//! severity as losing the issuer signing key.

use std::path::Path;

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use anyhow::Context;
use base64::Engine;
use rusqlite::Connection;

/// A community PDS account the issuer custodies.
#[derive(Debug, Clone)]
pub struct CommunitySession {
    pub did: String,
    pub handle: String,
    pub access_jwt: String,
    pub refresh_jwt: String,
}

pub struct SessionStore {
    db: Connection,
    key: [u8; 32],
}

impl SessionStore {
    /// Open (or create) the SQLite store at `path` and load the AES key from
    /// WUMBLR_ISSUER_SESSION_KEY_B64.
    pub fn open<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let key = load_key_from_env()?;
        let db = Connection::open(path).context("open issuer.db")?;
        init_db(&db)?;
        Ok(Self { db, key })
    }

    /// Persist a freshly-provisioned community session, encrypting the JWTs.
    pub fn insert(&self, s: &CommunitySession) -> anyhow::Result<()> {
        let access_enc = encrypt(&self.key, &s.access_jwt);
        let refresh_enc = encrypt(&self.key, &s.refresh_jwt);
        let now = chrono::Utc::now().timestamp();
        self.db.execute(
            "INSERT INTO community_sessions (did, handle, access_jwt_enc, refresh_jwt_enc, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(did) DO UPDATE SET
               handle = excluded.handle,
               access_jwt_enc = excluded.access_jwt_enc,
               refresh_jwt_enc = excluded.refresh_jwt_enc,
               updated_at = excluded.updated_at",
            rusqlite::params![s.did, s.handle, access_enc, refresh_enc, now],
        )?;
        Ok(())
    }

    /// Look up a stored session by community DID, decrypting the JWTs.
    pub fn get(&self, did: &str) -> anyhow::Result<Option<CommunitySession>> {
        let mut stmt = self.db.prepare(
            "SELECT did, handle, access_jwt_enc, refresh_jwt_enc
             FROM community_sessions WHERE did = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![did])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let did: String = row.get(0)?;
        let handle: String = row.get(1)?;
        let access_enc: String = row.get(2)?;
        let refresh_enc: String = row.get(3)?;
        Ok(Some(CommunitySession {
            did,
            handle,
            access_jwt: decrypt(&self.key, &access_enc)?,
            refresh_jwt: decrypt(&self.key, &refresh_enc)?,
        }))
    }

    /// Replace the stored JWTs for a community after a token refresh.
    pub fn update_tokens(
        &self,
        did: &str,
        access_jwt: &str,
        refresh_jwt: &str,
    ) -> anyhow::Result<()> {
        let access_enc = encrypt(&self.key, access_jwt);
        let refresh_enc = encrypt(&self.key, refresh_jwt);
        let now = chrono::Utc::now().timestamp();
        self.db.execute(
            "UPDATE community_sessions
             SET access_jwt_enc = ?2, refresh_jwt_enc = ?3, updated_at = ?4
             WHERE did = ?1",
            rusqlite::params![did, access_enc, refresh_enc, now],
        )?;
        Ok(())
    }
}

fn init_db(db: &Connection) -> Result<(), rusqlite::Error> {
    db.execute(
        "CREATE TABLE IF NOT EXISTS community_sessions (
            did             TEXT PRIMARY KEY,
            handle          TEXT NOT NULL,
            access_jwt_enc  TEXT NOT NULL,
            refresh_jwt_enc TEXT NOT NULL,
            created_at      INTEGER NOT NULL,
            updated_at      INTEGER NOT NULL
        )",
        [],
    )?;
    Ok(())
}

fn load_key_from_env() -> anyhow::Result<[u8; 32]> {
    let b64 = std::env::var("WUMBLR_ISSUER_SESSION_KEY_B64")
        .context("WUMBLR_ISSUER_SESSION_KEY_B64 not set — refusing to start")?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(b64.trim())
        .context("WUMBLR_ISSUER_SESSION_KEY_B64 not valid base64url")?;
    bytes
        .try_into()
        .map_err(|v: Vec<u8>| anyhow::anyhow!("session key must be 32 bytes, got {}", v.len()))
}

/// Encrypt a plaintext string with AES-256-GCM.
/// Returns base64url(nonce(12) || ciphertext).
fn encrypt(key: &[u8; 32], plaintext: &str) -> String {
    use rand::RngCore;
    let cipher = Aes256Gcm::new(key.into());
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .expect("AES-256-GCM encryption failed with a valid key");
    let mut combined = Vec::with_capacity(12 + ciphertext.len());
    combined.extend_from_slice(&nonce_bytes);
    combined.extend_from_slice(&ciphertext);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(combined)
}

/// Decrypt a value produced by `encrypt`.
fn decrypt(key: &[u8; 32], encoded: &str) -> anyhow::Result<String> {
    let combined = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .context("session blob not valid base64url")?;
    if combined.len() < 13 {
        anyhow::bail!("encrypted session too short");
    }
    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let cipher = Aes256Gcm::new(key.into());
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| anyhow::anyhow!("session decryption failed — wrong key?"))?;
    String::from_utf8(plaintext).context("decrypted session not valid UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> SessionStore {
        // 32-byte all-zero key for tests, set via env before open.
        let key_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([0u8; 32]);
        // SAFETY: tests are single-threaded per-process for env writes here.
        unsafe {
            std::env::set_var("WUMBLR_ISSUER_SESSION_KEY_B64", key_b64);
        }
        SessionStore {
            db: {
                let db = Connection::open_in_memory().unwrap();
                init_db(&db).unwrap();
                db
            },
            key: [0u8; 32],
        }
    }

    #[test]
    fn roundtrip_encrypt_decrypt() {
        let key = [7u8; 32];
        let enc = encrypt(&key, "super-secret-jwt");
        assert_ne!(enc, "super-secret-jwt");
        assert_eq!(decrypt(&key, &enc).unwrap(), "super-secret-jwt");
    }

    #[test]
    fn decrypt_with_wrong_key_fails() {
        let enc = encrypt(&[1u8; 32], "x");
        assert!(decrypt(&[2u8; 32], &enc).is_err());
    }

    #[test]
    fn insert_get_update() {
        let store = test_store();
        let s = CommunitySession {
            did: "did:plc:abc123".into(),
            handle: "musicx7f9k2.self.surf".into(),
            access_jwt: "access-1".into(),
            refresh_jwt: "refresh-1".into(),
        };
        store.insert(&s).unwrap();

        let got = store.get("did:plc:abc123").unwrap().unwrap();
        assert_eq!(got.handle, "musicx7f9k2.self.surf");
        assert_eq!(got.access_jwt, "access-1");
        assert_eq!(got.refresh_jwt, "refresh-1");

        store
            .update_tokens("did:plc:abc123", "access-2", "refresh-2")
            .unwrap();
        let got2 = store.get("did:plc:abc123").unwrap().unwrap();
        assert_eq!(got2.access_jwt, "access-2");
        assert_eq!(got2.refresh_jwt, "refresh-2");

        assert!(store.get("did:plc:missing").unwrap().is_none());
    }
}
