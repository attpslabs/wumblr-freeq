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
    broker::{BrokerClient, HttpBroker, MockBroker},
    config::Config,
    issuer::IssuerClient,
    routes::{auth, credentials, health, session, well_known},
};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub broker: Arc<dyn BrokerClient>,
    pub sessions: Arc<session::SessionStore>,
    pub issuer: Arc<IssuerClient>,
}

pub fn router(config: Config) -> Router {
    // Broker selection: an explicit WUMBLR_DEV_MOCK_BROKER=1 forces the
    // in-memory mock for unit-style local dev. Otherwise we point at the
    // real broker at WUMBLR_BROKER_URL.
    let broker: Arc<dyn BrokerClient> = if std::env::var("WUMBLR_DEV_MOCK_BROKER").is_ok() {
        tracing::warn!("WUMBLR_DEV_MOCK_BROKER set — using in-memory MockBroker");
        Arc::new(MockBroker::new())
    } else {
        Arc::new(HttpBroker::new(config.broker_url.clone()))
    };

    let issuer = Arc::new(IssuerClient::new(
        config.issuer_url.clone(),
        config.issuer_shared_secret.clone(),
    ));

    let state = AppState {
        config: Arc::new(config),
        broker,
        sessions: Arc::new(session::SessionStore::new()),
        issuer,
    };

    // Permissive CORS for M1/M2 dev. Tightens at M5 deploy.
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::mirror_request())
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
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
        // did:web identity for wumblr itself. (The issuer DID
        // `did:web:api.wumblr.com:verify` is published by wumblr-issuer,
        // routed at the same hostname via nginx.)
        .route("/.well-known/did.json", get(well_known::did_document))
        // Session exchange — frontend posts broker_token, gets back a wumblr
        // bearer + identity + freeq web-token.
        .route("/session/exchange", post(session::exchange))
        .route("/me", get(session::me))
        // Credential proxy — backend HMAC-calls the issuer on the user's behalf.
        .route(
            "/credentials/wumblr_member",
            get(credentials::wumblr_member),
        )
        // Dev-only OAuth callback bridge — see routes/auth.rs.
        .route("/auth/callback", get(auth::callback))
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}
