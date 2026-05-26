use axum::{Json, extract::State};
use serde_json::{Value, json};

use crate::app::AppState;

/// ATProto OAuth 2.0 client metadata for wumblr.
///
/// Both `@atproto/oauth-client-browser` (web) and `@atproto/oauth-client-expo`
/// (native) dereference this URL to discover redirect URIs, grant types, and
/// the JWKS endpoint. Hosting one document for both platforms is intentional —
/// freeq-auth-broker also references the same `client_id`, so all three
/// services agree on one OAuth identity.
///
/// Spec: https://atproto.com/specs/oauth
pub async fn oauth_client_metadata(State(state): State<AppState>) -> Json<Value> {
    let cfg = &state.config;
    Json(json!({
        "client_id": cfg.oauth_client_id(),
        "client_name": "wumblr",
        "client_uri": cfg.public_origin,
        // application_type "native" matches the Expo OAuth client and lets
        // the same metadata serve web (RN-Web) and native (iOS/Android).
        // ATProto OAuth accepts either application_type as long as redirects
        // and auth-method align — "native" + token_endpoint_auth_method:"none"
        // is what `@atproto/oauth-client-expo` expects.
        "application_type": "native",
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "scope": "atproto transition:generic",
        "redirect_uris": [
            // Production web: where deployed Cloudflare Pages will land users.
            cfg.web_redirect_uri(),
            // Local-dev web. ATProto OAuth allows http://127.0.0.1:<port>
            // (loopback) as a redirect URI for development clients. The web
            // OAuth library picks whichever declared redirect matches the
            // current document origin. From `npx expo start --web` (default
            // 8081) the library uses this one; from production it uses the
            // https one above. Remove or restrict before public launch.
            "http://127.0.0.1:8081/auth/callback",
            // Native custom-scheme redirect.
            //
            // ATProto OAuth spec: "Any custom scheme must match the client_id
            // hostname in reverse-domain order." Our client_id host is
            // `api.wumblr.com`, so the scheme is `com.wumblr.api`. One trailing
            // slash after the colon (not two — `com.wumblr.api://` is invalid
            // per the spec).
            //
            // If we ever change the host (e.g. to wumblr.com), this string
            // changes to `com.wumblr:/auth/callback`. Keep in sync with
            // apps/mobile/assets/oauth-client-metadata.json and the
            // `scheme` field in apps/mobile/app.json.
            "com.wumblr.api:/auth/callback",
        ],
        // Public client; native and SPA flows don't have a stable secret.
        "token_endpoint_auth_method": "none",
        "dpop_bound_access_tokens": true,
    }))
}

/// JWKS endpoint. Returns the public keys whose private counterparts are
/// used to sign `client_assertion` JWTs during OAuth token exchange.
///
/// **M1 step 2 placeholder:** returns an empty key set. Real key material
/// is generated and persisted in M1 step 4 when OAuth is wired up end-to-end.
/// Hitting this endpoint in current state will work but no OAuth flow can
/// complete yet.
pub async fn jwks() -> Json<Value> {
    Json(json!({ "keys": [] }))
}

/// `did:web:wumblr.com` DID document. Identifies the wumblr service itself
/// (used as the `client_id` host and referenced by the OAuth metadata).
///
/// In M1 we serve a minimal valid DID document. Service entries (PDS, appview,
/// freeq endpoints) get added in M3/M5 as services come online.
pub async fn did_document(State(state): State<AppState>) -> Json<Value> {
    let did = derive_did_web(&state.config.public_origin);
    Json(json!({
        "@context": ["https://www.w3.org/ns/did/v1"],
        "id": did,
        "service": [],
    }))
}

/// `did:web:wumblr.com:verify` — the issuer DID used for freeq policy
/// credentials. wumblr-freeq verifies `wumblr_member:<rkey>` credential
/// signatures against this DID's verification method.
///
/// **M1 step 2 placeholder:** no verification method yet. Real key material
/// lands in M2 when the freeq credential signer is wired up.
pub async fn verify_did_document(State(state): State<AppState>) -> Json<Value> {
    let mut did = derive_did_web(&state.config.public_origin);
    did.push_str(":verify");
    Json(json!({
        "@context": ["https://www.w3.org/ns/did/v1"],
        "id": did,
        "verificationMethod": [],
    }))
}

/// Convert a public origin like `https://wumblr.com` → `did:web:wumblr.com`.
/// For `http://127.0.0.1:8787` we emit `did:web:127.0.0.1%3A8787` (per spec).
fn derive_did_web(origin: &str) -> String {
    let host = origin
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .replace(':', "%3A");
    format!("did:web:{host}")
}

#[cfg(test)]
mod tests {
    use super::derive_did_web;

    #[test]
    fn did_web_from_https() {
        assert_eq!(derive_did_web("https://wumblr.com"), "did:web:wumblr.com");
    }

    #[test]
    fn did_web_from_localhost() {
        assert_eq!(
            derive_did_web("http://127.0.0.1:8787"),
            "did:web:127.0.0.1%3A8787"
        );
    }

    #[test]
    fn did_web_strips_trailing_slash() {
        assert_eq!(derive_did_web("https://wumblr.com/"), "did:web:wumblr.com");
    }
}
