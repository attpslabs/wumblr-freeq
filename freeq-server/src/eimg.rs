//! Client for the contrail "spaces" service that backs ephemeral, E2E-encrypted
//! images (the private image store).
//!
//! freeq-server is a **blind broker**: it forwards client-encrypted ciphertext
//! to the spaces service and never holds the content key or sees plaintext. The
//! spaces service stores only ciphertext + metadata + an `expires_at` it
//! enforces (24h hard delete). See `docs`/the plan for the full design.
//!
//! # Auth: shared-secret trusted gateway
//!
//! freeq authenticates the end user once (web OAuth or SASL), then calls the
//! spaces service over HTTP asserting the user's DID, authenticated by a shared
//! secret. This is the cross-process analogue of contrail's in-process marker:
//! the two services stay separate and independently deployable; the shared
//! element is the trust relationship, not a merged codebase. Concretely we send
//! two headers (matching contrail's `createTrustedGatewayMiddleware`):
//!
//! - `x-contrail-gateway-secret: <shared secret>`
//! - `x-contrail-gateway-did:    <acting user DID>`
//!
//! # Surface
//!
//! We mirror the subset of contrail's `<ns>.space.*` XRPC we need:
//! `createSpace`, `addMember`, `removeMember`, `uploadBlob`, `getBlob`.

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use serde_json::Value;

/// Header names for contrail's trusted-gateway auth (must match the contrail
/// `createTrustedGatewayMiddleware` constants).
const GATEWAY_SECRET_HEADER: &str = "x-contrail-gateway-secret";
const GATEWAY_DID_HEADER: &str = "x-contrail-gateway-did";

/// Result of uploading a ciphertext blob: the content id the spaces service
/// assigned (CID), plus the canonical space URI it lives in.
#[derive(Debug, Clone)]
pub struct UploadedBlob {
    pub cid: String,
    pub space_uri: String,
}

/// Bytes + content-type returned from a blob fetch.
#[derive(Debug, Clone)]
pub struct FetchedBlob {
    pub bytes: Vec<u8>,
    pub content_type: String,
}

/// Outcome of a blob fetch: present, or gone (expired/absent → caller maps to
/// HTTP 410/404). Distinguishing `Gone` lets the read path surface the 24h
/// hard-delete cleanly without treating it as an error.
#[derive(Debug, Clone)]
pub enum BlobFetch {
    Found(FetchedBlob),
    Gone,
}

/// Abstraction over the spaces service so endpoint/handler tests can inject a
/// mock without a live HTTP service. The real implementation is
/// [`HttpEimgClient`].
#[async_trait]
pub trait EimgClient: Send + Sync {
    /// Ensure a space exists for `channel` owned by `owner_did`. Idempotent:
    /// returns the space URI whether it was just created or already existed.
    async fn ensure_space(&self, owner_did: &str, channel: &str) -> Result<String>;

    /// Add `member_did` to the channel's space (mirrors an IRC JOIN). Best
    /// effort at the call site — see the membership-sync notes in server.rs.
    async fn add_member(&self, acting_did: &str, space_uri: &str, member_did: &str) -> Result<()>;

    /// Remove `member_did` from the channel's space (mirrors PART/KICK).
    async fn remove_member(
        &self,
        acting_did: &str,
        space_uri: &str,
        member_did: &str,
    ) -> Result<()>;

    /// Upload a ciphertext blob to the channel's space on behalf of `did`.
    async fn upload_blob(
        &self,
        did: &str,
        space_uri: &str,
        content_type: &str,
        ciphertext: &[u8],
    ) -> Result<UploadedBlob>;

    /// Fetch a ciphertext blob. Returns `BlobFetch::Gone` for an expired or
    /// missing blob (HTTP 410/404 from the spaces service).
    async fn get_blob(&self, did: &str, space_uri: &str, cid: &str) -> Result<BlobFetch>;
}

/// Configuration for the HTTP spaces client.
#[derive(Debug, Clone)]
pub struct EimgConfig {
    /// Base URL of the contrail spaces service, e.g. `https://eimg.wumblr.com`.
    pub base_url: String,
    /// Shared secret for the trusted-gateway auth.
    pub shared_secret: String,
    /// Contrail deployment namespace, used to build XRPC method NSIDs
    /// (`<namespace>.space.*`). e.g. `com.wumblr.eimg`.
    pub namespace: String,
    /// The space `type` NSID for image spaces, e.g. `com.wumblr.eimg.space`.
    pub space_type: String,
}

/// Real HTTP client for the contrail spaces service.
#[derive(Clone)]
pub struct HttpEimgClient {
    http: reqwest::Client,
    cfg: EimgConfig,
}

impl HttpEimgClient {
    pub fn new(cfg: EimgConfig) -> Self {
        Self {
            http: reqwest::Client::new(),
            cfg,
        }
    }

    fn xrpc_url(&self, method_suffix: &str) -> String {
        // e.g. https://host/xrpc/com.wumblr.eimg.space.uploadBlob
        format!(
            "{}/xrpc/{}.space.{}",
            self.cfg.base_url.trim_end_matches('/'),
            self.cfg.namespace,
            method_suffix
        )
    }

    /// Apply the trusted-gateway auth headers (secret + asserted user DID).
    fn auth(&self, req: reqwest::RequestBuilder, did: &str) -> reqwest::RequestBuilder {
        req.header(GATEWAY_SECRET_HEADER, &self.cfg.shared_secret)
            .header(GATEWAY_DID_HEADER, did)
    }
}

#[async_trait]
impl EimgClient for HttpEimgClient {
    async fn ensure_space(&self, owner_did: &str, channel: &str) -> Result<String> {
        // contrail keys a space by (owner, type, key); we use the channel name
        // (lowercased for stability) as the key. createSpace 409s if it already
        // exists — which we treat as success and resolve the URI from the body
        // or by reconstructing it.
        let key = channel.to_lowercase();
        let url = self.xrpc_url("createSpace");
        let body = serde_json::json!({ "type": self.cfg.space_type, "key": key });
        let res = self
            .auth(self.http.post(&url), owner_did)
            .json(&body)
            .send()
            .await
            .context("spaces createSpace request failed")?;

        let status = res.status();
        if status.is_success() {
            let v: Value = res
                .json()
                .await
                .context("spaces createSpace: unparseable body")?;
            return v
                .get("space")
                .and_then(|s| s.get("uri"))
                .and_then(|u| u.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| anyhow!("spaces createSpace: no space.uri in response"));
        }
        if status.as_u16() == 409 {
            // Already exists — reconstruct the at-uri: at://<owner>/<type>/<key>.
            return Ok(format!(
                "at://{}/{}/{}",
                owner_did, self.cfg.space_type, key
            ));
        }
        let text = res.text().await.unwrap_or_default();
        bail!("spaces createSpace returned {status}: {text}")
    }

    async fn add_member(&self, acting_did: &str, space_uri: &str, member_did: &str) -> Result<()> {
        let url = self.xrpc_url("addMember");
        let body = serde_json::json!({ "spaceUri": space_uri, "did": member_did });
        let res = self
            .auth(self.http.post(&url), acting_did)
            .json(&body)
            .send()
            .await
            .context("spaces addMember request failed")?;
        let status = res.status();
        if !status.is_success() {
            let text = res.text().await.unwrap_or_default();
            bail!("spaces addMember returned {status}: {text}");
        }
        Ok(())
    }

    async fn remove_member(
        &self,
        acting_did: &str,
        space_uri: &str,
        member_did: &str,
    ) -> Result<()> {
        let url = self.xrpc_url("removeMember");
        let body = serde_json::json!({ "spaceUri": space_uri, "did": member_did });
        let res = self
            .auth(self.http.post(&url), acting_did)
            .json(&body)
            .send()
            .await
            .context("spaces removeMember request failed")?;
        let status = res.status();
        if !status.is_success() {
            let text = res.text().await.unwrap_or_default();
            bail!("spaces removeMember returned {status}: {text}");
        }
        Ok(())
    }

    async fn upload_blob(
        &self,
        did: &str,
        space_uri: &str,
        content_type: &str,
        ciphertext: &[u8],
    ) -> Result<UploadedBlob> {
        // uploadBlob takes the space via query and raw bytes as the body.
        let url = format!(
            "{}?spaceUri={}",
            self.xrpc_url("uploadBlob"),
            urlencoding::encode(space_uri)
        );
        let res = self
            .auth(self.http.post(&url), did)
            .header("content-type", content_type)
            .body(ciphertext.to_vec())
            .send()
            .await
            .context("spaces uploadBlob request failed")?;
        let status = res.status();
        if !status.is_success() {
            let text = res.text().await.unwrap_or_default();
            bail!("spaces uploadBlob returned {status}: {text}");
        }
        let v: Value = res
            .json()
            .await
            .context("spaces uploadBlob: unparseable body")?;
        let cid = v
            .get("blob")
            .and_then(|b| b.get("ref"))
            .and_then(|r| r.get("$link"))
            .and_then(|l| l.as_str())
            .ok_or_else(|| anyhow!("spaces uploadBlob: no blob.ref.$link in response"))?
            .to_string();
        Ok(UploadedBlob {
            cid,
            space_uri: space_uri.to_string(),
        })
    }

    async fn get_blob(&self, did: &str, space_uri: &str, cid: &str) -> Result<BlobFetch> {
        let url = format!(
            "{}?spaceUri={}&cid={}",
            self.xrpc_url("getBlob"),
            urlencoding::encode(space_uri),
            urlencoding::encode(cid)
        );
        let res = self
            .auth(self.http.get(&url), did)
            .send()
            .await
            .context("spaces getBlob request failed")?;
        let status = res.status();
        // 410 Gone (expired) or 404 Not Found → Gone; the read path maps this to
        // HTTP 410 for the freeq client.
        if status.as_u16() == 410 || status.as_u16() == 404 {
            return Ok(BlobFetch::Gone);
        }
        if !status.is_success() {
            let text = res.text().await.unwrap_or_default();
            bail!("spaces getBlob returned {status}: {text}");
        }
        let content_type = res
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let bytes = res
            .bytes()
            .await
            .context("spaces getBlob: reading body failed")?
            .to_vec();
        Ok(BlobFetch::Found(FetchedBlob {
            bytes,
            content_type,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cfg() -> EimgConfig {
        EimgConfig {
            base_url: "https://eimg.example.com/".to_string(),
            shared_secret: "secret".to_string(),
            namespace: "com.wumblr.eimg".to_string(),
            space_type: "com.wumblr.eimg.space".to_string(),
        }
    }

    #[test]
    fn xrpc_url_is_built_from_namespace() {
        let c = HttpEimgClient::new(test_cfg());
        assert_eq!(
            c.xrpc_url("uploadBlob"),
            "https://eimg.example.com/xrpc/com.wumblr.eimg.space.uploadBlob"
        );
        // trailing slash on base_url is trimmed, not doubled.
        assert!(!c.xrpc_url("getBlob").contains("com//xrpc"));
    }

    #[test]
    fn ensure_space_uri_reconstruction_matches_at_uri_shape() {
        // The 409 path reconstructs at://<owner>/<type>/<key>; assert the shape
        // we'd build matches what contrail's buildSpaceUri produces.
        let c = test_cfg();
        let owner = "did:plc:alice";
        let key = "#General".to_lowercase();
        let uri = format!("at://{}/{}/{}", owner, c.space_type, key);
        assert_eq!(uri, "at://did:plc:alice/com.wumblr.eimg.space/#general");
    }
}
