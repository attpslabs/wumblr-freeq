//! Session endpoints.
//!
//! Flow:
//!  1. The browser does the OAuth dance against the broker (it lands on
//!     `https://auth.wumblr.com/auth/login`, gets bounced through bsky.social,
//!     comes back to the broker's `/auth/callback` which redirects to the
//!     frontend's `return_to` with `#oauth=<base64-encoded-token>` in the URL
//!     fragment).
//!  2. The frontend extracts the `broker_token` from that payload and calls
//!     `POST /session/exchange` here.
//!  3. Backend forwards the token to broker's `POST /session` to resolve
//!     `{did, handle, freeq_web_token, nick}`, mints a local opaque bearer,
//!     stashes the mapping in process memory, returns the bearer + identity
//!     to the frontend.
//!  4. Frontend stores the wumblr bearer (e.g. `wb-abc123…`) and presents it
//!     on `GET /me` plus any future authenticated routes.
//!
//! Restarting wumblr-backend invalidates every bearer. The frontend must
//! re-run the exchange (the broker_token in `sessionStorage` should still
//! be valid until the broker DB drops it). M5 may move this to a persistent
//! store; for M2 it's in-memory.

use std::sync::Mutex;

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::{app::AppState, broker::ResolvedSession};

#[derive(Debug, Deserialize)]
pub struct ExchangeReq {
    pub broker_token: String,
}

#[derive(Debug, Serialize)]
pub struct ExchangeRes {
    pub bearer: String,
    pub did: String,
    pub handle: String,
    /// freeq web-token, returned so the frontend can present it on the IRC
    /// WebSocket SASL handshake. Opaque from our perspective.
    pub freeq_web_token: String,
    pub nick: String,
}

pub async fn exchange(
    State(state): State<AppState>,
    Json(body): Json<ExchangeReq>,
) -> Result<Json<ExchangeRes>, (StatusCode, String)> {
    let resolved = state
        .broker
        .resolve_broker_token(&body.broker_token)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("broker exchange failed: {e}")))?
        .ok_or((StatusCode::UNAUTHORIZED, "invalid broker token".into()))?;

    let bearer = mint_bearer();
    state.sessions.insert(bearer.clone(), resolved.clone());

    info!(did = %resolved.did, handle = %resolved.handle, "session exchanged");

    Ok(Json(ExchangeRes {
        bearer,
        did: resolved.did,
        handle: resolved.handle,
        freeq_web_token: resolved.freeq_web_token,
        nick: resolved.nick,
    }))
}

#[derive(Debug, Serialize)]
pub struct MeRes {
    pub did: String,
    pub handle: String,
    pub nick: String,
}

pub async fn me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<MeRes>, (StatusCode, String)> {
    let bearer = extract_bearer(&headers).ok_or((
        StatusCode::UNAUTHORIZED,
        "missing Authorization: Bearer header".into(),
    ))?;

    let resolved = state
        .sessions
        .get(bearer)
        .ok_or((StatusCode::UNAUTHORIZED, "unknown session".into()))?;

    Ok(Json(MeRes {
        did: resolved.did,
        handle: resolved.handle,
        nick: resolved.nick,
    }))
}

/// Opaque, URL-safe wumblr-backend bearer. `wb-` prefix lets us tell it
/// apart from broker tokens in logs/error messages.
fn mint_bearer() -> String {
    let mut buf = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut buf);
    use base64::Engine;
    let suffix = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf);
    format!("wb-{suffix}")
}

fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    let h = headers.get(AUTHORIZATION)?.to_str().ok()?;
    h.strip_prefix("Bearer ")
}

/// Process-local session store. Maps wumblr-backend bearer →
/// `ResolvedSession`. Lost on restart; M2 is fine with that.
#[derive(Default)]
pub struct SessionStore {
    inner: Mutex<std::collections::HashMap<String, ResolvedSession>>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, bearer: String, session: ResolvedSession) {
        self.inner.lock().unwrap().insert(bearer, session);
    }

    pub fn get(&self, bearer: &str) -> Option<ResolvedSession> {
        self.inner.lock().unwrap().get(bearer).cloned()
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn extract_bearer_works() {
        let mut h = HeaderMap::new();
        h.insert(AUTHORIZATION, HeaderValue::from_static("Bearer wb-abc123"));
        assert_eq!(extract_bearer(&h), Some("wb-abc123"));
    }

    #[test]
    fn extract_bearer_missing() {
        let h = HeaderMap::new();
        assert_eq!(extract_bearer(&h), None);
    }

    #[test]
    fn extract_bearer_wrong_scheme() {
        let mut h = HeaderMap::new();
        h.insert(AUTHORIZATION, HeaderValue::from_static("Basic xyz"));
        assert_eq!(extract_bearer(&h), None);
    }

    #[test]
    fn mint_bearer_prefix_and_length() {
        let b = mint_bearer();
        assert!(b.starts_with("wb-"));
        // 24 bytes base64url-no-pad = 32 chars
        assert_eq!(b.len(), 3 + 32);
    }

    #[test]
    fn session_store_roundtrip() {
        let store = SessionStore::new();
        let s = ResolvedSession {
            did: "did:plc:abc".into(),
            handle: "alice.bsky.social".into(),
            freeq_web_token: "ft-xyz".into(),
            nick: "alice".into(),
        };
        store.insert("wb-1".into(), s.clone());
        assert_eq!(store.get("wb-1").unwrap().did, s.did);
        assert!(store.get("wb-missing").is_none());
    }
}
