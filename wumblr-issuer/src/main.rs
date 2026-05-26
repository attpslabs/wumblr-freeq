//! wumblr-issuer — VerifiableCredential issuer for wumblr communities.
//!
//! Signs `wumblr_member:<community>` credentials over PDS-public membership
//! state. Lives in attpslabs/wumblr-freeq (public, MIT) because the
//! credential shape and signing logic is the contract anyone running a
//! wumblr-compatible deployment needs.
//!
//! Trust model: wumblr-backend (the closed product API) authenticates incoming
//! credential-issuance requests with an HMAC-SHA256 over `ts={unix}\n{body}`,
//! shared via WUMBLR_ISSUER_SHARED_SECRET. The issuer doesn't talk to PDSes
//! itself — wumblr-backend has already established membership truth from the
//! firehose / appview when it asks for a credential.

use std::sync::Arc;

use anyhow::Context;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing_subscriber::{EnvFilter, fmt};

use wumblr_issuer::{credentials, did_doc, keys};

use crate::keys::IssuerKeys;

#[derive(Clone)]
struct AppState {
    keys: Arc<IssuerKeys>,
    shared_secret: Arc<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("wumblr_issuer=info,info")),
        )
        .init();

    let did = std::env::var("WUMBLR_ISSUER_DID")
        .unwrap_or_else(|_| "did:web:wumblr.com:verify".to_string());
    let listen = std::env::var("WUMBLR_ISSUER_LISTEN")
        .unwrap_or_else(|_| "0.0.0.0:3090".to_string());
    let shared_secret = std::env::var("WUMBLR_ISSUER_SHARED_SECRET")
        .context("WUMBLR_ISSUER_SHARED_SECRET not set — refusing to start")?;
    if shared_secret.len() < 32 {
        anyhow::bail!("WUMBLR_ISSUER_SHARED_SECRET too short (< 32 chars) — refusing to start");
    }

    let keys = IssuerKeys::from_env(&did)?;
    tracing::info!(
        did = %keys.did,
        pubkey = %keys.pubkey_multibase,
        "wumblr-issuer keys loaded"
    );

    let state = AppState {
        keys: Arc::new(keys),
        shared_secret: Arc::new(shared_secret),
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/verify/.well-known/did.json", get(did_document))
        .route("/credentials/issue", post(issue_credential))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&listen).await?;
    tracing::info!(%listen, "wumblr-issuer listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true, "service": "wumblr-issuer" }))
}

async fn did_document(State(state): State<AppState>) -> Json<Value> {
    Json(did_doc::build(&state.keys))
}

#[derive(Debug, Deserialize)]
struct IssueRequest {
    subject_did: String,
    community: String,
    /// Optional ttl in seconds. Defaults to 24h.
    #[serde(default)]
    ttl_secs: Option<i64>,
}

#[derive(Debug, Serialize)]
struct IssueResponse {
    credential: credentials::VerifiableCredential,
}

async fn issue_credential(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<IssueResponse>, (StatusCode, String)> {
    // HMAC auth — same scheme as broker↔freeq-server.
    verify_hmac(&headers, &body, state.shared_secret.as_bytes())
        .map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;

    let req: IssueRequest = serde_json::from_slice(&body)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("invalid body: {e}")))?;

    if !req.subject_did.starts_with("did:") {
        return Err((StatusCode::BAD_REQUEST, "subject_did must be a DID".into()));
    }
    if req.community.is_empty() || !req.community.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err((
            StatusCode::BAD_REQUEST,
            "community must be a non-empty alphanumeric/-/_ string".into(),
        ));
    }

    let credential_type = format!("wumblr_member:{}", req.community);
    let claims = json!({ "community": req.community });
    let ttl = req.ttl_secs.unwrap_or(86400).clamp(60, 7 * 86400);

    let cred = credentials::sign(
        &state.keys.did,
        &req.subject_did,
        &credential_type,
        claims,
        ttl,
        &state.keys.signing,
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!(
        subject = %req.subject_did,
        credential_type = %credential_type,
        ttl_secs = ttl,
        "issued credential"
    );

    Ok(Json(IssueResponse { credential: cred }))
}

fn verify_hmac(headers: &HeaderMap, body: &[u8], secret: &[u8]) -> anyhow::Result<()> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let sig = headers
        .get("x-wumblr-signature")
        .and_then(|v| v.to_str().ok())
        .context("missing X-Wumblr-Signature")?;
    let ts = headers
        .get("x-wumblr-timestamp")
        .and_then(|v| v.to_str().ok())
        .context("missing X-Wumblr-Timestamp")?;

    let ts_int: i64 = ts.parse().context("bad timestamp")?;
    let now = chrono::Utc::now().timestamp();
    if (now - ts_int).abs() > 60 {
        anyhow::bail!("timestamp outside replay window");
    }

    let mut signing_input = format!("ts={ts}\n").into_bytes();
    signing_input.extend_from_slice(body);

    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("hmac key length valid");
    mac.update(&signing_input);
    let expected = mac.finalize().into_bytes();
    use base64::Engine;
    let expected_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(expected);

    if !constant_time_eq(sig.as_bytes(), expected_b64.as_bytes()) {
        anyhow::bail!("bad signature");
    }
    Ok(())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut acc: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        acc |= x ^ y;
    }
    acc == 0
}
