//! VerifiableCredential signing.
//!
//! Field shape MUST match freeq-server's `policy::types::VerifiableCredential`
//! byte-for-byte after JCS canonicalization — otherwise signatures won't verify.
//! See parity_test in tests/.

use ed25519_dalek::{Signer, SigningKey};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerifiableCredential {
    #[serde(rename = "type")]
    pub credential_type_tag: String,
    pub issuer: String,
    pub subject: String,
    pub credential_type: String,
    pub claims: serde_json::Value,
    pub issued_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    pub signature: String,
}

pub fn sign(
    issuer_did: &str,
    subject_did: &str,
    credential_type: &str,
    claims: serde_json::Value,
    ttl_secs: i64,
    signing_key: &SigningKey,
) -> anyhow::Result<VerifiableCredential> {
    let now = chrono::Utc::now();
    let mut cred = VerifiableCredential {
        credential_type_tag: "FreeqCredential/v1".to_string(),
        issuer: issuer_did.to_string(),
        subject: subject_did.to_string(),
        credential_type: credential_type.to_string(),
        claims,
        issued_at: now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        expires_at: Some(
            (now + chrono::Duration::seconds(ttl_secs))
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        ),
        signature: String::new(),
    };
    let canonical = freeq_sdk::canonical::canonicalize(&cred)?;
    let sig = signing_key.sign(canonical.as_bytes());
    use base64::Engine;
    cred.signature = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sig.to_bytes());
    Ok(cred)
}
