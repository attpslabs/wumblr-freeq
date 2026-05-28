//! Community creation.
//!
//! `POST /communities` — requires a signed-in wumblr bearer. Backend resolves
//! the bearer to the caller's DID, then asks wumblr-issuer to provision a new
//! community account on ePDS (creating the DID + handle, custodying the PDS
//! session, and writing the initial profile + admins records with the caller
//! as primary admin). Returns the new community's DID + handle.
//!
//! The issuer does the privileged work; the backend's job here is auth
//! (resolve bearer → DID) and input validation.

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::app::AppState;

#[derive(Debug, Deserialize)]
pub struct CreateCommunityReq {
    /// Human-readable name (1–64 chars). Stored verbatim as displayName.
    pub display_name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// "open" | "invite". Only "open" works end-to-end in M3; the field is
    /// accepted so the frontend can send it, but "invite" isn't enforced yet.
    #[serde(default = "default_join_mode")]
    pub join_mode: String,
}

fn default_join_mode() -> String {
    "open".to_string()
}

#[derive(Debug, Serialize)]
pub struct CreateCommunityRes {
    pub did: String,
    pub handle: String,
}

pub async fn create_community(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateCommunityReq>,
) -> Result<Json<CreateCommunityRes>, (StatusCode, String)> {
    let bearer = extract_bearer(&headers).ok_or((
        StatusCode::UNAUTHORIZED,
        "missing Authorization: Bearer header".into(),
    ))?;
    let session = state
        .sessions
        .get(bearer)
        .ok_or((StatusCode::UNAUTHORIZED, "unknown session".into()))?;

    let display_name = req.display_name.trim();
    if display_name.is_empty() || display_name.chars().count() > 64 {
        return Err((
            StatusCode::BAD_REQUEST,
            "display_name must be 1–64 chars".into(),
        ));
    }
    if req.join_mode != "open" && req.join_mode != "invite" {
        return Err((
            StatusCode::BAD_REQUEST,
            "join_mode must be 'open' or 'invite'".into(),
        ));
    }

    let created = state
        .issuer
        .create_community(
            display_name,
            req.description.as_deref(),
            &session.did,
            &req.join_mode,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                format!("community provisioning failed: {e}"),
            )
        })?;

    info!(
        did = %created.did,
        handle = %created.handle,
        creator = %session.did,
        "created community"
    );

    Ok(Json(CreateCommunityRes {
        did: created.did,
        handle: created.handle,
    }))
}

fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    let h = headers.get(AUTHORIZATION)?.to_str().ok()?;
    h.strip_prefix("Bearer ")
}
