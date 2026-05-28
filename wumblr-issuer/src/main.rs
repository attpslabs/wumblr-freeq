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
use tokio::sync::Mutex;
use tracing_subscriber::{EnvFilter, fmt};

use wumblr_issuer::{credentials, did_doc, epds, keys, sessions};

use crate::epds::EpdsClient;
use crate::keys::IssuerKeys;
use crate::sessions::{CommunitySession, SessionStore};

#[derive(Clone)]
struct AppState {
    keys: Arc<IssuerKeys>,
    shared_secret: Arc<String>,
    sessions: Arc<Mutex<SessionStore>>,
    epds: Arc<EpdsClient>,
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

    let db_path =
        std::env::var("WUMBLR_ISSUER_DB").unwrap_or_else(|_| "./data/issuer.db".to_string());
    let session_store = SessionStore::open(&db_path)
        .with_context(|| format!("opening community session store at {db_path}"))?;
    let epds_client = EpdsClient::from_env()?;
    tracing::info!(%db_path, "community session store ready; ePDS client configured");

    let state = AppState {
        keys: Arc::new(keys),
        shared_secret: Arc::new(shared_secret),
        sessions: Arc::new(Mutex::new(session_store)),
        epds: Arc::new(epds_client),
    };

    let app = Router::new()
        .route("/health", get(health))
        // did:web spec resolution: for a DID with path components
        // (did:web:api.wumblr.com:verify), the doc lives at /verify/did.json.
        // For a DID without path components (did:web:api.wumblr.com), it
        // lives at /.well-known/did.json. We serve both so resolvers that
        // disagree on the rule still find us.
        .route("/verify/did.json", get(did_document))
        .route("/verify/.well-known/did.json", get(did_document))
        .route("/credentials/issue", post(issue_credential))
        .route("/communities", post(create_community))
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

#[derive(Debug, Deserialize)]
struct CreateCommunityRequest {
    /// Human-readable community name as typed by the creator (stored verbatim
    /// as profile.displayName). Used, truncated, to seed the handle.
    display_name: String,
    /// Optional community description.
    #[serde(default)]
    description: Option<String>,
    /// DID of the creating user — seeded as the community's primary admin.
    creator_did: String,
    /// "open" | "invite". Only "open" is implemented end-to-end in M3.
    #[serde(default = "default_join_mode")]
    join_mode: String,
}

fn default_join_mode() -> String {
    "open".to_string()
}

#[derive(Debug, Serialize)]
struct CreateCommunityResponse {
    did: String,
    handle: String,
}

/// Provision a community ATProto account on ePDS, custody its session, and
/// write the initial profile + admins records. Called service-to-service by
/// wumblr-backend (HMAC-authed). The issuer is a dumb executor here — the
/// backend has already authorized the caller; the issuer just provisions.
async fn create_community(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<CreateCommunityResponse>, (StatusCode, String)> {
    verify_hmac(&headers, &body, state.shared_secret.as_bytes())
        .map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;

    let req: CreateCommunityRequest = serde_json::from_slice(&body)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("invalid body: {e}")))?;

    if !req.creator_did.starts_with("did:") {
        return Err((StatusCode::BAD_REQUEST, "creator_did must be a DID".into()));
    }
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

    // Derive the ePDS handle local-part: sanitized, truncated name + random
    // salt. The salt disambiguates same-named communities. It can't be
    // derived from the DID (the DID only exists after account creation), so
    // it's random — the DID remains the true unique key.
    let salt = random_salt();
    let local = handle_local_part(display_name, &salt);
    // Synthetic, never-delivered email — community accounts have no inbox.
    let email = format!("community-{salt}@wumblr.internal");

    let created = state
        .epds
        .create_account(&local, &email)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("ePDS create failed: {e}")))?;

    // Custody the session before any write — if a write fails we still have
    // the credentials to retry rather than orphaning the account.
    {
        let store = state.sessions.lock().await;
        store
            .insert(&CommunitySession {
                did: created.did.clone(),
                handle: created.handle.clone(),
                access_jwt: created.access_jwt.clone(),
                refresh_jwt: created.refresh_jwt.clone(),
            })
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("session store failed: {e}"),
                )
            })?;
    }

    let now = chrono::Utc::now().to_rfc3339();

    // Write profile (rkey "self") on the community's own PDS.
    let profile = json!({
        "$type": "com.wumblr.profile",
        "displayName": display_name,
        "description": req.description,
        "joinMode": req.join_mode,
        "createdAt": now,
    });
    state
        .epds
        .put_record(
            &created.access_jwt,
            &created.did,
            "com.wumblr.profile",
            "self",
            profile,
        )
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("profile write failed: {e}")))?;

    // Write admins (rkey "self") — creator is the primary admin (first entry).
    let admins = json!({
        "$type": "com.wumblr.admins",
        "admins": [ { "did": req.creator_did, "addedAt": now } ],
    });
    state
        .epds
        .put_record(
            &created.access_jwt,
            &created.did,
            "com.wumblr.admins",
            "self",
            admins,
        )
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("admins write failed: {e}")))?;

    tracing::info!(
        did = %created.did,
        handle = %created.handle,
        creator = %req.creator_did,
        "provisioned community"
    );

    Ok(Json(CreateCommunityResponse {
        did: created.did,
        handle: created.handle,
    }))
}

/// 6 random base32-ish chars (a–z, 2–7) used to disambiguate handles.
fn random_salt() -> String {
    use rand::Rng;
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut rng = rand::thread_rng();
    (0..6)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect()
}

/// Build the ePDS handle local-part from a display name + salt.
///
/// ePDS requires the local part to be 5–20 chars, single-label, ASCII
/// alphanumeric. We lowercase the name, strip everything non-alphanumeric,
/// truncate to 14 chars (so name 14 + salt 6 = 20 max), and append the salt.
/// If the sanitized name is empty (e.g. all-emoji name) we fall back to "c".
fn handle_local_part(display_name: &str, salt: &str) -> String {
    let mut name: String = display_name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    name.truncate(14);
    if name.is_empty() {
        name.push('c');
    }
    format!("{name}{salt}")
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
