//! Session endpoints.
//!
//! `POST /session/register` — RN client posts the OAuth session blob
//!   (returned by `@atproto/oauth-client-expo.signIn()`) here. Backend hands
//!   it to the broker, gets back an opaque `session_id`, returns it to client.
//!
//! `GET /me` — client presents `Authorization: Bearer <session_id>`. Backend
//!   asks broker `/whoami` and returns the DID.

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::info;

use crate::app::AppState;

#[derive(Debug, Deserialize)]
pub struct RegisterSessionReq {
    pub did: String,
    /// The serialized OAuth session blob from `@atproto/oauth-client-expo`.
    /// We don't introspect it in M1 — broker owns this shape.
    pub session: Value,
}

#[derive(Debug, Serialize)]
pub struct RegisterSessionRes {
    pub session_id: String,
}

pub async fn register_session(
    State(state): State<AppState>,
    Json(body): Json<RegisterSessionReq>,
) -> Result<Json<RegisterSessionRes>, (StatusCode, String)> {
    let session_id = state
        .broker
        .register_session(&body.did, body.session)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    info!(did = %body.did, "registered session");
    Ok(Json(RegisterSessionRes { session_id }))
}

#[derive(Debug, Serialize)]
pub struct MeRes {
    pub did: String,
    pub handle: Option<String>,
}

pub async fn me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<MeRes>, (StatusCode, String)> {
    let bearer = extract_bearer(&headers).ok_or((
        StatusCode::UNAUTHORIZED,
        "missing Authorization: Bearer header".to_string(),
    ))?;

    let who = state
        .broker
        .whoami(bearer)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::UNAUTHORIZED, "unknown session".to_string()))?;

    Ok(Json(MeRes {
        did: who.did,
        handle: who.handle,
    }))
}

fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    let h = headers.get(AUTHORIZATION)?.to_str().ok()?;
    h.strip_prefix("Bearer ")
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn extract_bearer_works() {
        let mut h = HeaderMap::new();
        h.insert(AUTHORIZATION, HeaderValue::from_static("Bearer abc-123"));
        assert_eq!(extract_bearer(&h), Some("abc-123"));
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
}
