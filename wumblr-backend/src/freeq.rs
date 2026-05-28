//! freeq-server HTTP client.
//!
//! Backend posts broker-HMAC-signed requests to freeq-server. Currently used
//! for `POST /api/v1/communities/member`, which writes a `com.wumblr.member`
//! record to a user's PDS — freeq custodies the user's PDS session (pushed by
//! the broker) and performs the write on the user's behalf.
//!
//! HMAC scheme matches freeq-server's `verify_broker_signature_raw`:
//! `X-Broker-Signature` (HMAC-SHA256, base64, over `ts={ts}\n{body}`) +
//! `X-Broker-Timestamp` (unix seconds), 60s replay window.

use anyhow::{Context, Result};

pub struct FreeqClient {
    base_url: String,
    broker_secret: String,
    http: reqwest::Client,
}

impl FreeqClient {
    pub fn new(base_url: String, broker_secret: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            broker_secret,
            http: reqwest::Client::new(),
        }
    }

    /// Ask freeq to write `com.wumblr.member` to `did`'s PDS, marking them a
    /// member of `community_did`. Requires the user to have an active freeq
    /// Login session (i.e. they're signed in).
    pub async fn write_member(&self, did: &str, community_did: &str) -> Result<()> {
        let body = serde_json::json!({
            "did": did,
            "community_did": community_did,
        });
        let body_bytes = serde_json::to_vec(&body)?;

        let ts = chrono::Utc::now().timestamp();
        let (sig, ts_str) = sign_broker_hmac(ts, &body_bytes, self.broker_secret.as_bytes());

        let resp = self
            .http
            .post(format!("{}/api/v1/communities/member", self.base_url))
            .header("Content-Type", "application/json")
            .header("X-Broker-Signature", sig)
            .header("X-Broker-Timestamp", ts_str)
            .body(body_bytes)
            .send()
            .await
            .context("calling freeq /api/v1/communities/member")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("freeq /api/v1/communities/member returned {status}: {body}");
        }
        Ok(())
    }
}

/// HMAC-SHA256 over `ts={ts}\n` then `body`, base64url-no-pad — matching
/// freeq-server's `verify_broker_signature_raw`. Returns (signature, ts).
fn sign_broker_hmac(ts: i64, body: &[u8], secret: &[u8]) -> (String, String) {
    use base64::Engine;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let ts_str = ts.to_string();
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC key length valid");
    mac.update(format!("ts={ts_str}\n").as_bytes());
    mac.update(body);
    let tag = mac.finalize().into_bytes();
    let sig = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(tag);
    (sig, ts_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_freeq_verifier() {
        // Mirror freeq-server::verify_broker_signature_raw exactly.
        let ts = 1234567890i64;
        let body = br#"{"did":"did:plc:abc","community_did":"did:plc:xyz"}"#;
        let secret = b"broker-shared-secret-32-bytes-long";

        let (sig, ts_str) = sign_broker_hmac(ts, body, secret);

        use base64::Engine;
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
        mac.update(format!("ts={ts_str}\n").as_bytes());
        mac.update(body);
        let expected =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        assert_eq!(sig, expected);
    }
}
