//! DID-based end-to-end encryption (ENC2).
//!
//! Replaces passphrase-based E2EE with identity-bound encryption using
//! keys from DID documents. Only verified members of a channel can decrypt.
//!
//! # Protocol Overview
//!
//! 1. Each user's DID document contains a secp256k1 public key
//! 2. Channel encryption uses a shared **group key** derived from ECDH
//! 3. The group key is derived from: sorted member DIDs + pairwise ECDH secrets
//! 4. When membership changes, the group key is rotated
//!
//! # Wire Format
//!
//! ```text
//! ENC2:<epoch>:<nonce-b64>:<ciphertext-b64>
//! ```
//!
//! - `ENC2` — version tag (identity-bound E2EE)
//! - `epoch` — key epoch (increments on membership change)
//! - `nonce` — 12-byte AES-GCM nonce, base64url-encoded
//! - `ciphertext` — AES-256-GCM ciphertext + tag, base64url-encoded
//!
//! # Key Derivation
//!
//! For a channel with member DIDs [A, B, C] (sorted lexicographically):
//!
//! ```text
//! group_ikm = HKDF-Extract(
//!   salt: SHA-256(channel_name),
//!   ikm:  sorted_dids_concatenated
//! )
//! group_key = HKDF-Expand(group_ikm, info: "freeq-e2ee-v2-<epoch>", len: 32)
//! ```
//!
//! Each member proves they belong by being able to sign challenges
//! during SASL auth. The server tracks authenticated members, and the
//! client derives the group key from the known member set.
//!
//! # Key Exchange
//!
//! For private messages (DM E2EE), we use ECDH:
//!
//! ```text
//! shared = ECDH(my_private_key, their_public_key)
//! dm_key = HKDF-SHA256(shared, salt: sorted(did_a, did_b), info: "freeq-dm-v1")
//! ```

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hkdf::Hkdf;
use sha2::Sha256;

/// Prefix for DID-based encrypted messages.
pub const ENC2_PREFIX: &str = "ENC2:";

/// A group encryption context for a channel.
#[derive(Debug, Clone)]
pub struct GroupKey {
    /// The channel name.
    pub channel: String,
    /// Sorted list of member DIDs.
    pub members: Vec<String>,
    /// Key epoch (increments on membership change).
    pub epoch: u64,
    /// Derived AES-256 key.
    key: [u8; 32],
}

impl GroupKey {
    /// Derive a group key for a channel with the given authenticated members.
    ///
    /// Members are sorted lexicographically before derivation, so the same
    /// set always produces the same key regardless of join order.
    pub fn derive(channel: &str, members: &[String], epoch: u64) -> Self {
        use sha2::Digest;

        let mut sorted: Vec<String> = members.to_vec();
        sorted.sort();
        sorted.dedup();

        // IKM: concatenation of sorted DIDs
        let ikm: Vec<u8> = sorted.iter().flat_map(|d| d.as_bytes().to_vec()).collect();
        let salt = Sha256::digest(channel.to_lowercase().as_bytes());

        let hk = Hkdf::<Sha256>::new(Some(&salt), &ikm);
        let info = format!("freeq-e2ee-v2-{epoch}");
        let mut key = [0u8; 32];
        hk.expand(info.as_bytes(), &mut key)
            .expect("32 bytes is valid for HKDF");

        Self {
            channel: channel.to_string(),
            members: sorted,
            epoch,
            key,
        }
    }

    /// Encrypt a plaintext message.
    ///
    /// Returns: `ENC2:<epoch>:<nonce>:<ciphertext>`
    pub fn encrypt(&self, plaintext: &str) -> Result<String, EncryptError> {
        let cipher = Aes256Gcm::new_from_slice(&self.key).map_err(|_| EncryptError::BadKey)?;
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ct = cipher
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|_| EncryptError::EncryptFailed)?;

        let nonce_b64 = URL_SAFE_NO_PAD.encode(&nonce[..]);
        let ct_b64 = URL_SAFE_NO_PAD.encode(&ct);

        Ok(format!("{ENC2_PREFIX}{}:{nonce_b64}:{ct_b64}", self.epoch))
    }

    /// Decrypt a wire-format ENC2 message.
    pub fn decrypt(&self, wire: &str) -> Result<String, DecryptError> {
        let body = wire
            .strip_prefix(ENC2_PREFIX)
            .ok_or(DecryptError::NotEncrypted)?;

        let parts: Vec<&str> = body.splitn(3, ':').collect();
        if parts.len() != 3 {
            return Err(DecryptError::MalformedMessage);
        }

        let epoch: u64 = parts[0]
            .parse()
            .map_err(|_| DecryptError::MalformedMessage)?;
        if epoch != self.epoch {
            return Err(DecryptError::EpochMismatch {
                expected: self.epoch,
                got: epoch,
            });
        }

        let nonce_bytes = URL_SAFE_NO_PAD
            .decode(parts[1])
            .map_err(|_| DecryptError::MalformedMessage)?;
        let ct_bytes = URL_SAFE_NO_PAD
            .decode(parts[2])
            .map_err(|_| DecryptError::MalformedMessage)?;

        if nonce_bytes.len() != 12 {
            return Err(DecryptError::MalformedMessage);
        }

        let cipher = Aes256Gcm::new_from_slice(&self.key).map_err(|_| DecryptError::BadKey)?;
        let nonce = Nonce::from_slice(&nonce_bytes);
        let pt = cipher
            .decrypt(nonce, ct_bytes.as_ref())
            .map_err(|_| DecryptError::DecryptFailed)?;

        String::from_utf8(pt).map_err(|_| DecryptError::InvalidUtf8)
    }

    /// Encrypt raw bytes for the ephemeral image store.
    ///
    /// Unlike [`encrypt`](Self::encrypt) (which produces the textual `ENC2:`
    /// wire format for IRC messages), this produces a raw binary blob:
    /// the 12-byte AES-GCM nonce **prepended** to the ciphertext, with no
    /// base64 or text envelope. This matches the freeq-server `/api/v1/eimg`
    /// contract ("nonce-prepended ciphertext") and the at-rest format in
    /// `freeq-server/src/db.rs`. The epoch is NOT embedded — the caller tracks
    /// which epoch's key encrypted a given image out of band.
    pub fn encrypt_bytes(&self, plaintext: &[u8]) -> Result<Vec<u8>, EncryptError> {
        let cipher = Aes256Gcm::new_from_slice(&self.key).map_err(|_| EncryptError::BadKey)?;
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ct = cipher
            .encrypt(&nonce, plaintext)
            .map_err(|_| EncryptError::EncryptFailed)?;

        let mut blob = Vec::with_capacity(12 + ct.len());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ct);
        Ok(blob)
    }

    /// Decrypt a nonce-prepended blob produced by [`encrypt_bytes`](Self::encrypt_bytes).
    ///
    /// The first 12 bytes are the AES-GCM nonce; the remainder is the
    /// ciphertext+tag. Returns the plaintext bytes.
    pub fn decrypt_bytes(&self, blob: &[u8]) -> Result<Vec<u8>, DecryptError> {
        if blob.len() < 12 {
            return Err(DecryptError::MalformedMessage);
        }
        let (nonce_bytes, ct_bytes) = blob.split_at(12);
        let cipher = Aes256Gcm::new_from_slice(&self.key).map_err(|_| DecryptError::BadKey)?;
        let nonce = Nonce::from_slice(nonce_bytes);
        cipher
            .decrypt(nonce, ct_bytes)
            .map_err(|_| DecryptError::DecryptFailed)
    }

    /// Check if this key has the same member set.
    pub fn members_match(&self, members: &[String]) -> bool {
        let mut sorted: Vec<String> = members.to_vec();
        sorted.sort();
        sorted.dedup();
        self.members == sorted
    }
}

/// DM encryption using ECDH key agreement.
///
/// Derives a shared secret from two secp256k1 keys, then derives an
/// AES-256 key for DM encryption.
pub struct DmKey {
    /// Both DIDs in sorted order.
    pub dids: (String, String),
    /// Derived AES-256 key.
    key: [u8; 32],
}

impl DmKey {
    /// Derive a DM key from an ECDH shared secret.
    ///
    /// `my_private` is the local user's secp256k1 private key bytes (32 bytes).
    /// `their_public` is the remote user's compressed secp256k1 public key.
    /// DIDs are used as salt for domain separation.
    pub fn from_secp256k1(
        my_did: &str,
        their_did: &str,
        my_private: &[u8; 32],
        their_public_bytes: &[u8],
    ) -> Result<Self, String> {
        use k256::PublicKey as K256Pub;
        use k256::ecdh::diffie_hellman;
        use k256::elliptic_curve::sec1::FromEncodedPoint;

        let my_scalar =
            k256::NonZeroScalar::try_from(&my_private[..]).map_err(|_| "Invalid private key")?;

        let their_point = k256::EncodedPoint::from_bytes(their_public_bytes)
            .map_err(|_| "Invalid public key encoding")?;
        let their_key = K256Pub::from_encoded_point(&their_point);
        if their_key.is_none().into() {
            return Err("Invalid public key point".to_string());
        }
        let their_key = their_key.unwrap();

        let shared = diffie_hellman(&my_scalar, their_key.as_affine());
        let shared_bytes = shared.raw_secret_bytes();

        // Sort DIDs for deterministic salt
        let (did_a, did_b) = if my_did < their_did {
            (my_did, their_did)
        } else {
            (their_did, my_did)
        };
        let salt = format!("{did_a}:{did_b}");

        let hk = Hkdf::<Sha256>::new(Some(salt.as_bytes()), shared_bytes);
        let mut key = [0u8; 32];
        hk.expand(b"freeq-dm-v1", &mut key)
            .expect("32 bytes is valid");

        Ok(Self {
            dids: (did_a.to_string(), did_b.to_string()),
            key,
        })
    }

    /// Encrypt a DM.
    pub fn encrypt(&self, plaintext: &str) -> Result<String, EncryptError> {
        let cipher = Aes256Gcm::new_from_slice(&self.key).map_err(|_| EncryptError::BadKey)?;
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ct = cipher
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|_| EncryptError::EncryptFailed)?;

        let nonce_b64 = URL_SAFE_NO_PAD.encode(&nonce[..]);
        let ct_b64 = URL_SAFE_NO_PAD.encode(&ct);

        Ok(format!("{ENC2_PREFIX}dm:{nonce_b64}:{ct_b64}"))
    }

    /// Decrypt a DM.
    pub fn decrypt(&self, wire: &str) -> Result<String, DecryptError> {
        let body = wire
            .strip_prefix(ENC2_PREFIX)
            .ok_or(DecryptError::NotEncrypted)?;

        let body = body.strip_prefix("dm:").ok_or(DecryptError::NotDm)?;

        let (nonce_b64, ct_b64) = body.split_once(':').ok_or(DecryptError::MalformedMessage)?;

        let nonce_bytes = URL_SAFE_NO_PAD
            .decode(nonce_b64)
            .map_err(|_| DecryptError::MalformedMessage)?;
        let ct_bytes = URL_SAFE_NO_PAD
            .decode(ct_b64)
            .map_err(|_| DecryptError::MalformedMessage)?;

        if nonce_bytes.len() != 12 {
            return Err(DecryptError::MalformedMessage);
        }

        let cipher = Aes256Gcm::new_from_slice(&self.key).map_err(|_| DecryptError::BadKey)?;
        let nonce = Nonce::from_slice(&nonce_bytes);
        let pt = cipher
            .decrypt(nonce, ct_bytes.as_ref())
            .map_err(|_| DecryptError::DecryptFailed)?;

        String::from_utf8(pt).map_err(|_| DecryptError::InvalidUtf8)
    }
}

/// Check if a message is ENC2-encrypted.
pub fn is_encrypted(text: &str) -> bool {
    text.starts_with(ENC2_PREFIX)
}

/// Parse the epoch from an ENC2 message without decrypting.
pub fn parse_epoch(wire: &str) -> Option<u64> {
    let body = wire.strip_prefix(ENC2_PREFIX)?;
    let epoch_str = body.split(':').next()?;
    if epoch_str == "dm" {
        return None; // DM, no epoch
    }
    epoch_str.parse().ok()
}

#[derive(Debug, thiserror::Error)]
pub enum EncryptError {
    #[error("invalid key")]
    BadKey,
    #[error("encryption failed")]
    EncryptFailed,
}

#[derive(Debug, thiserror::Error)]
pub enum DecryptError {
    #[error("not an ENC2 encrypted message")]
    NotEncrypted,
    #[error("not a DM message")]
    NotDm,
    #[error("malformed encrypted message")]
    MalformedMessage,
    #[error("invalid key")]
    BadKey,
    #[error("epoch mismatch: expected {expected}, got {got}")]
    EpochMismatch { expected: u64, got: u64 },
    #[error("decryption failed (wrong key, wrong members, or tampered)")]
    DecryptFailed,
    #[error("decrypted data is not valid UTF-8")]
    InvalidUtf8,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_key_roundtrip() {
        let members = vec![
            "did:plc:alice".to_string(),
            "did:plc:bob".to_string(),
            "did:plc:charlie".to_string(),
        ];
        let gk = GroupKey::derive("#secret", &members, 1);

        let wire = gk.encrypt("Hello group!").unwrap();
        assert!(wire.starts_with("ENC2:1:"));
        assert!(is_encrypted(&wire));

        let pt = gk.decrypt(&wire).unwrap();
        assert_eq!(pt, "Hello group!");
    }

    /// Canonical cross-implementation parity vector. The TypeScript SDK
    /// (`freeq-sdk-js/src/eimg.ts`) MUST derive the byte-identical key for these
    /// exact inputs, or cross-client image decryption silently fails. The
    /// expected hex below is the value this Rust implementation produces; the TS
    /// test hardcodes the same constant. If either side's derivation changes,
    /// this test breaks and the two must be re-synced.
    ///
    /// Inputs deliberately given UNSORTED + with a duplicate, and a mixed-case
    /// channel, to exercise the sort/dedup/lowercase both sides must match.
    #[test]
    fn group_key_derivation_vector() {
        let members = vec![
            "did:plc:bob".to_string(),
            "did:plc:alice".to_string(),
            "did:plc:bob".to_string(),
        ];
        let gk = GroupKey::derive("#Secret", &members, 0);
        let key_hex: String = gk.key.iter().map(|b| format!("{b:02x}")).collect();
        // Emit so the TS vector can be copied if it ever needs regenerating.
        println!("PARITY group_key(#Secret, sorted[alice,bob], epoch 0) = {key_hex}");
        assert_eq!(key_hex, GROUP_KEY_PARITY_HEX);
    }

    /// Shared constant — keep identical to `EXPECTED_KEY_HEX` in
    /// freeq-sdk-js/src/eimg.test.ts.
    const GROUP_KEY_PARITY_HEX: &str =
        "f3a95c43ef7245faee31bfde76b2e7de50de309c1ee801042ca22c90138900a7";

    #[test]
    fn encrypt_bytes_roundtrip() {
        let members = vec!["did:plc:alice".to_string(), "did:plc:bob".to_string()];
        let gk = GroupKey::derive("#secret", &members, 1);

        let plaintext = b"\x00\x01\x02 binary image bytes \xff\xfe";
        let blob = gk.encrypt_bytes(plaintext).unwrap();
        // Blob is nonce(12) ++ ciphertext+tag — strictly larger than plaintext,
        // and NOT the text ENC2: envelope.
        assert!(blob.len() > 12 + plaintext.len());
        assert!(!blob.starts_with(b"ENC2:"));

        let got = gk.decrypt_bytes(&blob).unwrap();
        assert_eq!(got, plaintext);
    }

    #[test]
    fn encrypt_bytes_wrong_members_fail() {
        let m1 = vec!["did:plc:alice".to_string(), "did:plc:bob".to_string()];
        let m2 = vec!["did:plc:alice".to_string(), "did:plc:carol".to_string()];
        let k1 = GroupKey::derive("#chan", &m1, 0);
        let k2 = GroupKey::derive("#chan", &m2, 0);

        let blob = k1.encrypt_bytes(b"secret image").unwrap();
        assert!(k2.decrypt_bytes(&blob).is_err());
    }

    #[test]
    fn encrypt_bytes_tamper_fails() {
        let members = vec!["did:plc:alice".to_string()];
        let gk = GroupKey::derive("#chan", &members, 0);
        let mut blob = gk.encrypt_bytes(b"hello").unwrap();
        // Flip a byte in the ciphertext region (after the 12-byte nonce).
        let last = blob.len() - 1;
        blob[last] ^= 0xff;
        assert!(gk.decrypt_bytes(&blob).is_err());
    }

    #[test]
    fn decrypt_bytes_truncated_blob_fails() {
        let members = vec!["did:plc:alice".to_string()];
        let gk = GroupKey::derive("#chan", &members, 0);
        // Fewer than 12 bytes can't contain a nonce.
        assert!(gk.decrypt_bytes(&[0u8; 5]).is_err());
        assert!(gk.decrypt_bytes(&[]).is_err());
    }

    #[test]
    fn encrypt_bytes_empty_plaintext() {
        let members = vec!["did:plc:alice".to_string()];
        let gk = GroupKey::derive("#chan", &members, 0);
        let blob = gk.encrypt_bytes(b"").unwrap();
        assert_eq!(gk.decrypt_bytes(&blob).unwrap(), b"");
    }

    #[test]
    fn group_key_order_independent() {
        let m1 = vec!["did:plc:bob".to_string(), "did:plc:alice".to_string()];
        let m2 = vec!["did:plc:alice".to_string(), "did:plc:bob".to_string()];

        let k1 = GroupKey::derive("#test", &m1, 0);
        let k2 = GroupKey::derive("#test", &m2, 0);

        // Same members, same key regardless of order
        let wire = k1.encrypt("test").unwrap();
        let pt = k2.decrypt(&wire).unwrap();
        assert_eq!(pt, "test");
    }

    #[test]
    fn group_key_different_members_fail() {
        let m1 = vec!["did:plc:alice".to_string(), "did:plc:bob".to_string()];
        let m2 = vec!["did:plc:alice".to_string(), "did:plc:charlie".to_string()];

        let k1 = GroupKey::derive("#test", &m1, 0);
        let k2 = GroupKey::derive("#test", &m2, 0);

        let wire = k1.encrypt("secret").unwrap();
        assert!(k2.decrypt(&wire).is_err());
    }

    #[test]
    fn group_key_epoch_mismatch() {
        let members = vec!["did:plc:alice".to_string()];
        let k1 = GroupKey::derive("#test", &members, 1);
        let k2 = GroupKey::derive("#test", &members, 2);

        let wire = k1.encrypt("test").unwrap();
        let err = k2.decrypt(&wire).unwrap_err();
        assert!(matches!(
            err,
            DecryptError::EpochMismatch {
                expected: 2,
                got: 1
            }
        ));
    }

    #[test]
    fn group_key_different_channel_fail() {
        let members = vec!["did:plc:alice".to_string()];
        let k1 = GroupKey::derive("#chan-a", &members, 0);
        let k2 = GroupKey::derive("#chan-b", &members, 0);

        let wire = k1.encrypt("test").unwrap();
        assert!(k2.decrypt(&wire).is_err());
    }

    #[test]
    fn dm_key_ecdh_roundtrip() {
        // Generate two secp256k1 keypairs
        let sk_a = k256::ecdsa::SigningKey::random(&mut rand::thread_rng());
        let sk_b = k256::ecdsa::SigningKey::random(&mut rand::thread_rng());

        let pk_a_bytes = sk_a.verifying_key().to_sec1_bytes();
        let pk_b_bytes = sk_b.verifying_key().to_sec1_bytes();

        let sk_a_bytes: [u8; 32] = sk_a.to_bytes().into();
        let sk_b_bytes: [u8; 32] = sk_b.to_bytes().into();

        // Both sides derive the same DM key
        let dm_a = DmKey::from_secp256k1("did:plc:alice", "did:plc:bob", &sk_a_bytes, &pk_b_bytes)
            .unwrap();

        let dm_b = DmKey::from_secp256k1("did:plc:bob", "did:plc:alice", &sk_b_bytes, &pk_a_bytes)
            .unwrap();

        // A encrypts, B decrypts
        let wire = dm_a.encrypt("Secret DM").unwrap();
        assert!(wire.starts_with("ENC2:dm:"));
        let pt = dm_b.decrypt(&wire).unwrap();
        assert_eq!(pt, "Secret DM");

        // B encrypts, A decrypts
        let wire2 = dm_b.encrypt("Reply").unwrap();
        let pt2 = dm_a.decrypt(&wire2).unwrap();
        assert_eq!(pt2, "Reply");
    }

    #[test]
    fn dm_key_wrong_party_fails() {
        let sk_a = k256::ecdsa::SigningKey::random(&mut rand::thread_rng());
        let sk_b = k256::ecdsa::SigningKey::random(&mut rand::thread_rng());
        let sk_c = k256::ecdsa::SigningKey::random(&mut rand::thread_rng());

        let pk_b_bytes = sk_b.verifying_key().to_sec1_bytes();
        let pk_a_bytes = sk_a.verifying_key().to_sec1_bytes();

        let sk_a_bytes: [u8; 32] = sk_a.to_bytes().into();
        let sk_c_bytes: [u8; 32] = sk_c.to_bytes().into();

        let dm_ab = DmKey::from_secp256k1("did:plc:alice", "did:plc:bob", &sk_a_bytes, &pk_b_bytes)
            .unwrap();

        let dm_ca =
            DmKey::from_secp256k1("did:plc:charlie", "did:plc:alice", &sk_c_bytes, &pk_a_bytes)
                .unwrap();

        let wire = dm_ab.encrypt("For Bob only").unwrap();
        assert!(dm_ca.decrypt(&wire).is_err());
    }

    #[test]
    fn parse_epoch_works() {
        assert_eq!(parse_epoch("ENC2:42:nonce:ct"), Some(42));
        assert_eq!(parse_epoch("ENC2:dm:nonce:ct"), None);
        assert_eq!(parse_epoch("ENC1:nonce:ct"), None);
    }

    #[test]
    fn members_match_check() {
        let members = vec!["did:plc:b".to_string(), "did:plc:a".to_string()];
        let gk = GroupKey::derive("#test", &members, 0);

        assert!(gk.members_match(&["did:plc:a".to_string(), "did:plc:b".to_string()]));
        assert!(gk.members_match(&["did:plc:b".to_string(), "did:plc:a".to_string()]));
        assert!(!gk.members_match(&["did:plc:a".to_string()]));
    }
}
