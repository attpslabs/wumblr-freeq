//! did:web DID document for the issuer.
//!
//! Served at `/verify/.well-known/did.json` (proxied through nginx as
//! `https://wumblr.com/verify/.well-known/did.json` so the DID resolves
//! to `did:web:wumblr.com:verify`).

use serde_json::{Value, json};

use crate::keys::IssuerKeys;

pub fn build(keys: &IssuerKeys) -> Value {
    let key_id = format!("{}#key-1", keys.did);
    json!({
        "@context": [
            "https://www.w3.org/ns/did/v1",
            "https://w3id.org/security/suites/ed25519-2020/v1",
        ],
        "id": keys.did,
        "verificationMethod": [{
            "id": key_id,
            "type": "Ed25519VerificationKey2020",
            "controller": keys.did,
            "publicKeyMultibase": keys.pubkey_multibase,
        }],
        "assertionMethod": [key_id],
    })
}
