//! Client for the ePDS (self.surf) headless endpoints the issuer needs:
//!  - POST /_internal/account/create  → provision a community account
//!  - com.atproto.repo.putRecord      → write records to a community's repo
//!
//! Account creation is authenticated with the issuer's ePDS API key
//! (x-api-key, the key minted with --can-create-directly). Record writes
//! authenticate as the community itself, using the Bearer accessJwt the
//! issuer custodies for that community.

use anyhow::{Context, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone)]
pub struct EpdsClient {
    http: reqwest::Client,
    /// e.g. https://auth.self.surf — the ePDS auth-service base URL.
    auth_base: String,
    /// e.g. https://self.surf — the PDS (pds-core) base URL for XRPC.
    pds_base: String,
    /// x-api-key for /_internal/account/create.
    api_key: String,
}

/// Result of provisioning a community account.
#[derive(Debug, Clone)]
pub struct CreatedAccount {
    pub did: String,
    pub handle: String,
    pub access_jwt: String,
    pub refresh_jwt: String,
}

#[derive(Serialize)]
struct CreateAccountReq<'a> {
    handle: &'a str,
    email: &'a str,
}

#[derive(Deserialize)]
struct CreateAccountRes {
    did: String,
    handle: String,
    #[serde(rename = "accessJwt")]
    access_jwt: String,
    #[serde(rename = "refreshJwt")]
    refresh_jwt: String,
}

impl EpdsClient {
    pub fn from_env() -> anyhow::Result<Self> {
        let auth_base = std::env::var("WUMBLR_ISSUER_EPDS_AUTH_URL")
            .unwrap_or_else(|_| "https://auth.self.surf".to_string());
        let pds_base = std::env::var("WUMBLR_ISSUER_EPDS_PDS_URL")
            .unwrap_or_else(|_| "https://self.surf".to_string());
        let api_key = std::env::var("WUMBLR_ISSUER_EPDS_API_KEY")
            .context("WUMBLR_ISSUER_EPDS_API_KEY not set — refusing to start")?;
        Ok(Self {
            http: reqwest::Client::new(),
            auth_base: auth_base.trim_end_matches('/').to_string(),
            pds_base: pds_base.trim_end_matches('/').to_string(),
            api_key,
        })
    }

    /// Create a community account on ePDS. `handle` is the local part only
    /// (e.g. `musicx7f9k2`); ePDS appends the handle domain. `email` is a
    /// synthetic, never-delivered address that satisfies ePDS's uniqueness
    /// check.
    pub async fn create_account(
        &self,
        handle: &str,
        email: &str,
    ) -> anyhow::Result<CreatedAccount> {
        let url = format!("{}/_internal/account/create", self.auth_base);
        let res = self
            .http
            .post(&url)
            .header("x-api-key", &self.api_key)
            .json(&CreateAccountReq { handle, email })
            .send()
            .await
            .context("ePDS account/create request failed")?;

        let status = res.status();
        if !status.is_success() {
            let body = res.text().await.unwrap_or_default();
            return Err(anyhow!("ePDS account/create returned {status}: {body}"));
        }

        let parsed: CreateAccountRes = res
            .json()
            .await
            .context("ePDS account/create returned unparseable body")?;
        Ok(CreatedAccount {
            did: parsed.did,
            handle: parsed.handle,
            access_jwt: parsed.access_jwt,
            refresh_jwt: parsed.refresh_jwt,
        })
    }

    /// Write (create-or-replace) a record in a community's repo via
    /// com.atproto.repo.putRecord, authenticating as the community with its
    /// accessJwt. `repo` is the community DID.
    pub async fn put_record(
        &self,
        access_jwt: &str,
        repo: &str,
        collection: &str,
        rkey: &str,
        record: Value,
    ) -> anyhow::Result<()> {
        let url = format!("{}/xrpc/com.atproto.repo.putRecord", self.pds_base);
        let body = serde_json::json!({
            "repo": repo,
            "collection": collection,
            "rkey": rkey,
            "record": record,
        });
        let res = self
            .http
            .post(&url)
            .bearer_auth(access_jwt)
            .json(&body)
            .send()
            .await
            .context("ePDS putRecord request failed")?;

        let status = res.status();
        if !status.is_success() {
            let body = res.text().await.unwrap_or_default();
            return Err(anyhow!("ePDS putRecord returned {status}: {body}"));
        }
        Ok(())
    }
}
