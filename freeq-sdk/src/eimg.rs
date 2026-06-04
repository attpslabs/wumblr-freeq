//! Client for freeq's **ephemeral encrypted image** store (`/api/v1/eimg`).
//!
//! Images are encrypted client-side with the channel's ENC2 [`GroupKey`] before
//! they ever leave the device, so the freeq server (a blind broker) and the
//! backing spaces service only ever hold ciphertext — never the key or the
//! plaintext. Images are hard-deleted 24h after upload.
//!
//! # Flow
//!
//! - **Upload:** derive the channel's `GroupKey` from the current member DIDs +
//!   epoch, [`encrypt_bytes`](GroupKey::encrypt_bytes) the image (12-byte nonce
//!   prepended to the AES-GCM ciphertext), then multipart-POST the blob to
//!   `/api/v1/eimg`. The server returns an opaque `image_id` + `expires_at`.
//! - **Fetch:** GET `/api/v1/eimg/{image_id}`, then `decrypt_bytes` with the
//!   same `GroupKey`. A `410 Gone` response means the image has expired.
//!
//! # Caller responsibilities
//!
//! This module is deliberately pure: it does **not** resolve channel membership
//! or mint auth tokens. The caller must supply:
//!
//! - `members` — the channel's member **DIDs** (not nicks). DIDs are the durable
//!   identity that survives nick changes, so the key is derived from them. The
//!   SDK does not yet maintain a nick→DID membership cache; that is a planned
//!   follow-up. Both sender and recipient must derive the key from the *same*
//!   member set + `epoch`, or decryption fails.
//! - `epoch` — the channel's current key epoch (increments on membership change).
//! - `upload_token` — `Some(token)` to authenticate via a broker-issued
//!   `x-upload-token`, or `None` to rely on an active WebSocket session for
//!   `did` on the server. (The token-mint endpoint is broker-only, so clients
//!   cannot mint one themselves.)

use anyhow::{Context, Result, anyhow, bail};

use crate::e2ee_did::GroupKey;

/// Result of an encrypted-image upload.
#[derive(Debug, Clone)]
pub struct EimgUploadResult {
    /// Opaque server-assigned id used to fetch the image back.
    pub image_id: String,
    /// Unix timestamp (seconds) at which the image becomes inaccessible (410).
    pub expires_at: u64,
}

/// Outcome of a fetch: the decrypted image bytes, or `Gone` if the image has
/// expired / been deleted (HTTP 410/404).
#[derive(Debug, Clone)]
pub enum EimgFetch {
    Found(Vec<u8>),
    Gone,
}

/// Build the eimg endpoint URL from a web base (e.g. `https://irc.freeq.at`).
fn upload_url(web_base: &str) -> String {
    format!("{}/api/v1/eimg", web_base.trim_end_matches('/'))
}

/// Path URL for a fetch. `image_id` is a server-minted ULID (always
/// `[0-9A-Z]`, URL-safe), so the path segment needs no encoding; the `did`
/// query param is added via reqwest's query builder (which encodes it).
fn fetch_url(web_base: &str, image_id: &str) -> String {
    format!(
        "{}/api/v1/eimg/{}",
        web_base.trim_end_matches('/'),
        image_id
    )
}

/// Encrypt `image_bytes` with the channel's group key and upload the ciphertext.
///
/// `content_type` is the image's MIME type (e.g. `image/png`); it's stored
/// server-side and returned on fetch. See the module docs for `members`,
/// `epoch`, and `upload_token`.
#[allow(clippy::too_many_arguments)]
pub async fn upload_encrypted_image(
    client: &reqwest::Client,
    web_base: &str,
    did: &str,
    channel: &str,
    members: &[String],
    epoch: u64,
    upload_token: Option<&str>,
    content_type: &str,
    image_bytes: &[u8],
) -> Result<EimgUploadResult> {
    let key = GroupKey::derive(channel, members, epoch);
    let ciphertext = key
        .encrypt_bytes(image_bytes)
        .map_err(|e| anyhow!("image encryption failed: {e}"))?;

    let part = reqwest::multipart::Part::bytes(ciphertext)
        .file_name("eimg")
        .mime_str(content_type)
        .context("building eimg multipart part")?;
    let form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("did", did.to_string())
        .text("channel", channel.to_string());

    let mut req = client.post(upload_url(web_base)).multipart(form);
    if let Some(token) = upload_token {
        req = req.header("x-upload-token", token);
    }
    let resp = req.send().await.context("eimg upload request failed")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        bail!("eimg upload returned {status}: {body}");
    }
    let v: serde_json::Value = resp.json().await.context("eimg upload: unparseable body")?;
    let image_id = v
        .get("image_id")
        .and_then(|s| s.as_str())
        .ok_or_else(|| anyhow!("eimg upload: no image_id in response"))?
        .to_string();
    let expires_at = v
        .get("expires_at")
        .and_then(|n| n.as_u64())
        .ok_or_else(|| anyhow!("eimg upload: no expires_at in response"))?;
    Ok(EimgUploadResult {
        image_id,
        expires_at,
    })
}

/// Fetch and decrypt an encrypted image. Returns [`EimgFetch::Gone`] if the
/// image has expired (HTTP 410) or is absent (404).
#[allow(clippy::too_many_arguments)]
pub async fn fetch_encrypted_image(
    client: &reqwest::Client,
    web_base: &str,
    image_id: &str,
    did: &str,
    channel: &str,
    members: &[String],
    epoch: u64,
    upload_token: Option<&str>,
) -> Result<EimgFetch> {
    let mut req = client
        .get(fetch_url(web_base, image_id))
        .query(&[("did", did)]);
    if let Some(token) = upload_token {
        req = req.header("x-upload-token", token);
    }
    let resp = req.send().await.context("eimg fetch request failed")?;

    let status = resp.status();
    if status.as_u16() == 410 || status.as_u16() == 404 {
        return Ok(EimgFetch::Gone);
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        bail!("eimg fetch returned {status}: {body}");
    }
    let ciphertext = resp
        .bytes()
        .await
        .context("eimg fetch: reading body failed")?;

    let key = GroupKey::derive(channel, members, epoch);
    let plaintext = key
        .decrypt_bytes(&ciphertext)
        .map_err(|e| anyhow!("image decryption failed: {e}"))?;
    Ok(EimgFetch::Found(plaintext))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_builders_trim_and_encode() {
        assert_eq!(
            upload_url("https://irc.freeq.at/"),
            "https://irc.freeq.at/api/v1/eimg"
        );
        assert_eq!(
            fetch_url("https://irc.freeq.at", "abc123"),
            "https://irc.freeq.at/api/v1/eimg/abc123"
        );
    }

    #[test]
    fn encrypt_then_decrypt_via_groupkey_roundtrips_the_blob() {
        // The crypto contract the upload/fetch fns rely on: a blob produced by
        // the sender's key decrypts under the same channel/members/epoch.
        let members = vec!["did:plc:alice".to_string(), "did:plc:bob".to_string()];
        let sender = GroupKey::derive("#secret", &members, 3);
        let recipient = GroupKey::derive("#secret", &members, 3);

        let img = b"\x89PNG\r\n\x1a\n fake image bytes";
        let blob = sender.encrypt_bytes(img).unwrap();
        let got = recipient.decrypt_bytes(&blob).unwrap();
        assert_eq!(got, img);
    }
}
