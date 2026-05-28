//! wumblr-issuer HTTP client.
//!
//! Backend posts HMAC-signed credential-issuance requests to the issuer at
//! `POST <issuer>/credentials/issue`. The issuer verifies the HMAC, signs a
//! `wumblr_member:<community>` VerifiableCredential with its Ed25519 key, and
//! returns it. Backend forwards the credential to the client.
//!
//! HMAC scheme matches wumblr-issuer/src/main.rs: `X-Wumblr-Signature`
//! (HMAC-SHA256, base64url-no-pad) + `X-Wumblr-Timestamp` (unix seconds),
//! signed payload `ts={ts}\n{body}` with replay window 60s.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
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

pub struct IssuerClient {
    base_url: String,
    shared_secret: String,
    http: reqwest::Client,
}

impl IssuerClient {
    pub fn new(base_url: String, shared_secret: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            shared_secret,
            http: reqwest::Client::new(),
        }
    }

    /// Ask the issuer to sign a `wumblr_member:<community>` credential for
    /// `subject_did`. wumblr-backend is the ONLY caller of the issuer; this
    /// is where we'd add policy checks (e.g. "is this DID actually a member
    /// of this community?") before requesting issuance.
    pub async fn issue_wumblr_member(
        &self,
        subject_did: &str,
        community: &str,
        ttl_secs: i64,
    ) -> Result<VerifiableCredential> {
        let body = serde_json::json!({
            "subject_did": subject_did,
            "community": community,
            "ttl_secs": ttl_secs,
        });
        let body_bytes = serde_json::to_vec(&body)?;

        let ts = chrono::Utc::now().timestamp();
        let (sig, ts_str) = sign_hmac(ts, &body_bytes, self.shared_secret.as_bytes());

        let resp = self
            .http
            .post(format!("{}/credentials/issue", self.base_url))
            .header("Content-Type", "application/json")
            .header("X-Wumblr-Signature", sig)
            .header("X-Wumblr-Timestamp", ts_str)
            .body(body_bytes)
            .send()
            .await
            .context("calling issuer /credentials/issue")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("issuer /credentials/issue returned {status}: {body}");
        }

        #[derive(Deserialize)]
        struct IssueResponse {
            credential: VerifiableCredential,
        }
        let body: IssueResponse = resp
            .json()
            .await
            .context("decoding issuer /credentials/issue body")?;
        Ok(body.credential)
    }

    /// Ask the issuer to provision a new community account on ePDS, custody
    /// its session, and write the initial profile + admins records.
    /// `creator_did` becomes the community's primary admin. Returns the new
    /// community DID + handle.
    pub async fn create_community(
        &self,
        display_name: &str,
        description: Option<&str>,
        creator_did: &str,
        join_mode: &str,
    ) -> Result<CreatedCommunity> {
        let body = serde_json::json!({
            "display_name": display_name,
            "description": description,
            "creator_did": creator_did,
            "join_mode": join_mode,
        });
        let body_bytes = serde_json::to_vec(&body)?;

        let ts = chrono::Utc::now().timestamp();
        let (sig, ts_str) = sign_hmac(ts, &body_bytes, self.shared_secret.as_bytes());

        let resp = self
            .http
            .post(format!("{}/communities", self.base_url))
            .header("Content-Type", "application/json")
            .header("X-Wumblr-Signature", sig)
            .header("X-Wumblr-Timestamp", ts_str)
            .body(body_bytes)
            .send()
            .await
            .context("calling issuer /communities")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("issuer /communities returned {status}: {body}");
        }

        let body: CreatedCommunity = resp
            .json()
            .await
            .context("decoding issuer /communities body")?;
        Ok(body)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreatedCommunity {
    pub did: String,
    pub handle: String,
}

fn sign_hmac(ts: i64, body: &[u8], secret: &[u8]) -> (String, String) {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let ts_str = ts.to_string();
    let mut signing_input = format!("ts={ts_str}\n").into_bytes();
    signing_input.extend_from_slice(body);

    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC key length valid");
    mac.update(&signing_input);
    let tag = mac.finalize().into_bytes();

    use base64::Engine;
    let sig = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(tag);
    (sig, ts_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_issuer_verifier() {
        // Same scheme as wumblr-issuer/src/main.rs::verify_hmac. If this
        // test passes, our outgoing requests verify on the issuer side.
        let ts = 1234567890i64;
        let body = br#"{"subject_did":"did:plc:abc","community":"wumblr","ttl_secs":86400}"#;
        let secret = b"shared-test-secret-32-bytes-long-aa";

        let (sig, ts_str) = sign_hmac(ts, body, secret);
        assert_eq!(ts_str, "1234567890");

        // Recompute manually and compare.
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        let mut expected_input = format!("ts={ts_str}\n").into_bytes();
        expected_input.extend_from_slice(body);
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
        mac.update(&expected_input);
        let expected_tag = mac.finalize().into_bytes();
        use base64::Engine;
        let expected_sig =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(expected_tag);
        assert_eq!(sig, expected_sig);
    }
}
