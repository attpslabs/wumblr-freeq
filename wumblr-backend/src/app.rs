use std::sync::Arc;

use axum::{Router, routing::get};
use tower_http::trace::TraceLayer;

use crate::{
    config::Config,
    routes::{health, well_known},
};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
}

pub fn router(config: Config) -> Router {
    let state = AppState {
        config: Arc::new(config),
    };

    Router::new()
        // health + version
        .route("/health", get(health::health))
        // OAuth client metadata + JWKS — one identity for web + native + broker
        .route(
            "/oauth-client-metadata.json",
            get(well_known::oauth_client_metadata),
        )
        .route("/jwks.json", get(well_known::jwks))
        // did:web identity for wumblr itself + the freeq credential issuer
        .route("/.well-known/did.json", get(well_known::did_document))
        .route(
            "/verify/.well-known/did.json",
            get(well_known::verify_did_document),
        )
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}
