//! Load the Ed25519 issuer keypair from WUMBLR_ISSUER_PRIVKEY_B64 env.
//!
//! The 32-byte seed is base64url-encoded (no padding). The pubkey is derived
//! and published in /verify/.well-known/did.json as a multibase
//! Ed25519VerificationKey2020 (z + base58btc(0xed01 || pubkey)).

use anyhow::Context;
use ed25519_dalek::SigningKey;

pub struct IssuerKeys {
    pub signing: SigningKey,
    pub did: String,
    pub pubkey_multibase: String,
}

impl IssuerKeys {
    pub fn from_env(did: &str) -> anyhow::Result<Self> {
        use base64::Engine;
        let seed_b64 = std::env::var("WUMBLR_ISSUER_PRIVKEY_B64")
            .context("WUMBLR_ISSUER_PRIVKEY_B64 not set — refusing to start")?;
        let seed_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(seed_b64.trim())
            .context("WUMBLR_ISSUER_PRIVKEY_B64 not valid base64url")?;
        let seed: [u8; 32] = seed_bytes
            .try_into()
            .map_err(|v: Vec<u8>| anyhow::anyhow!("expected 32-byte seed, got {}", v.len()))?;
        let signing = SigningKey::from_bytes(&seed);
        let pubkey = signing.verifying_key().to_bytes();

        // Multibase: 'z' prefix + base58btc(0xed 0x01 || pubkey)
        let mut prefixed = Vec::with_capacity(34);
        prefixed.push(0xed);
        prefixed.push(0x01);
        prefixed.extend_from_slice(&pubkey);
        let pubkey_multibase = format!("z{}", bs58::encode(prefixed).into_string());

        Ok(Self {
            signing,
            did: did.to_string(),
            pubkey_multibase,
        })
    }
}
