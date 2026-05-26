//! Credential issuance proxy.
//!
//! `GET /credentials/wumblr_member?community=<rkey>` — requires a signed-in
//! wumblr bearer. Backend resolves the bearer to a DID via SessionStore,
//! asks wumblr-issuer to sign a `wumblr_member:<community>` VerifiableCredential
//! for that DID, and returns it to the client. The client presents the
//! credential to freeq-server's `POST /api/v1/credentials/present` before
//! attempting JOIN.
//!
//! For M2 the `community` is hardcoded to `wumblr` (open membership). When
//! M4 brings real communities, this route adds a membership check (does the
//! two-record handshake exist for this DID in this community?) before
//! requesting issuance.

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::{app::AppState, issuer::VerifiableCredential};

#[derive(Debug, Deserialize)]
pub struct WumblrMemberQuery {
    pub community: String,
}

#[derive(Debug, Serialize)]
pub struct WumblrMemberRes {
    pub credential: VerifiableCredential,
}

pub async fn wumblr_member(
    State(state): State<AppState>,
    Query(q): Query<WumblrMemberQuery>,
    headers: HeaderMap,
) -> Result<Json<WumblrMemberRes>, (StatusCode, String)> {
    let bearer = extract_bearer(&headers).ok_or((
        StatusCode::UNAUTHORIZED,
        "missing Authorization: Bearer header".into(),
    ))?;
    let session = state.sessions.get(bearer).ok_or((
        StatusCode::UNAUTHORIZED,
        "unknown session".into(),
    ))?;

    if q.community != "wumblr" {
        // M2 only supports the open-membership `wumblr` community. M4 expands
        // this to check the two-record handshake for arbitrary community rkeys.
        return Err((
            StatusCode::FORBIDDEN,
            format!(
                "community `{}` not supported in M2 (only `wumblr` is open-membership)",
                q.community
            ),
        ));
    }

    let credential = state
        .issuer
        .issue_wumblr_member(&session.did, &q.community, 24 * 3600)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                format!("issuer request failed: {e}"),
            )
        })?;

    info!(
        did = %session.did,
        community = %q.community,
        "issued credential"
    );

    Ok(Json(WumblrMemberRes { credential }))
}

fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    let h = headers.get(AUTHORIZATION)?.to_str().ok()?;
    h.strip_prefix("Bearer ")
}
