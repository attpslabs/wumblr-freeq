use std::sync::Arc;

use axum::{
    Router,
    http::{Method, header},
    routing::{get, post},
};
use tower_http::{
    cors::{AllowOrigin, CorsLayer},
    trace::TraceLayer,
};

use crate::{
    broker::{BrokerClient, MockBroker},
    config::Config,
    routes::{health, session, well_known},
};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub broker: Arc<dyn BrokerClient>,
}

pub fn router(config: Config) -> Router {
    let state = AppState {
        config: Arc::new(config),
        // M1 step 4: in-memory mock. Real HttpBroker swaps in at M2.
        broker: Arc::new(MockBroker::new()),
    };

    // Permissive CORS for M1 dev. Tightens at M5 deploy.
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::mirror_request())
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
        ])
        .allow_credentials(false);

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
        // Session endpoints (M1 step 4)
        .route("/session/register", post(session::register_session))
        .route("/me", get(session::me))
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}
