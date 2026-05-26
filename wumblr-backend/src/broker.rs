//! freeq-auth-broker client.
//!
//! The broker is the OAuth orchestrator: it runs `/auth/login` →
//! bsky.social → `/auth/callback` itself, stores encrypted sessions in its
//! own SQLite, and exposes a single client-facing endpoint `POST /session`
//! that exchanges a `broker_token` (received by the browser as part of
//! the broker's `return_to` redirect) for the user's `{did, handle}` plus
//! a freeq web-token usable for SASL on the chat WebSocket.
//!
//! See freeq-auth-broker/src/main.rs:942-1021 for the canonical shape.
//!
//! There is no `/whoami` or `/sessions` ingest endpoint; the previous
//! M1 trait that accepted an OAuth-blob from the client is gone. The
//! frontend now redirects directly to the broker and presents the
//! returned `broker_token` to this backend, which forwards it.

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Result of resolving a broker token. The fields mirror the broker's
/// `BrokerSessionResponse` (see freeq-auth-broker/src/main.rs:294-299).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedSession {
    pub did: String,
    pub handle: String,
    /// Opaque freeq web-token; the client passes this on the WebSocket
    /// SASL handshake to freeq-server (which trusts it because the broker
    /// minted it via an HMAC-authed `/auth/broker/web-token` push).
    pub freeq_web_token: String,
    pub nick: String,
}

#[async_trait::async_trait]
pub trait BrokerClient: Send + Sync {
    /// Exchange a one-time `broker_token` (delivered to the client by the
    /// broker's OAuth-callback redirect) for the session it represents.
    /// Returns `None` if the token is unknown or expired.
    async fn resolve_broker_token(&self, broker_token: &str) -> Result<Option<ResolvedSession>>;
}

/// In-memory test/dev impl. Maps pre-seeded tokens → sessions. Used by
/// unit tests; the runtime binding is `HttpBroker` in `app.rs`.
#[derive(Default)]
pub struct MockBroker {
    inner: Arc<Mutex<MockState>>,
}

#[derive(Default)]
struct MockState {
    sessions: std::collections::HashMap<String, ResolvedSession>,
}

impl MockBroker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-seed a token → session mapping for tests.
    pub fn seed(&self, broker_token: &str, session: ResolvedSession) {
        self.inner
            .lock()
            .unwrap()
            .sessions
            .insert(broker_token.to_string(), session);
    }
}

#[async_trait::async_trait]
impl BrokerClient for MockBroker {
    async fn resolve_broker_token(&self, broker_token: &str) -> Result<Option<ResolvedSession>> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .sessions
            .get(broker_token)
            .cloned())
    }
}

/// HTTP client against a real freeq-auth-broker. Used in production.
pub struct HttpBroker {
    base_url: String,
    http: reqwest::Client,
}

impl HttpBroker {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http: reqwest::Client::new(),
        }
    }
}

#[derive(Serialize)]
struct SessionRequest<'a> {
    broker_token: &'a str,
}

#[derive(Deserialize)]
struct SessionResponse {
    token: String,
    nick: String,
    did: String,
    handle: String,
}

#[async_trait::async_trait]
impl BrokerClient for HttpBroker {
    async fn resolve_broker_token(&self, broker_token: &str) -> Result<Option<ResolvedSession>> {
        let resp = self
            .http
            .post(format!("{}/session", self.base_url))
            .json(&SessionRequest { broker_token })
            .send()
            .await
            .context("calling broker /session")?;

        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Ok(None);
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("broker /session returned {status}: {body}");
        }

        let body: SessionResponse = resp.json().await.context("decoding broker /session body")?;
        Ok(Some(ResolvedSession {
            did: body.did,
            handle: body.handle,
            freeq_web_token: body.token,
            nick: body.nick,
        }))
    }
}
