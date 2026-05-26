use clap::Parser;

/// wumblr-backend configuration. Sourced from CLI flags or env vars
/// (`WUMBLR_*` prefix). Loaded once at startup and held in `AppState`.
#[derive(Debug, Clone, Parser)]
#[command(version, about)]
pub struct Config {
    /// Address to bind the HTTP server on.
    #[arg(long, env = "WUMBLR_LISTEN", default_value = "0.0.0.0:8787")]
    pub listen: String,

    /// Public origin where wumblr-backend is reachable.
    /// Used to build the OAuth `client_id` URL and well-known docs.
    /// Must be HTTPS in production; `http://127.0.0.1:8787` works for local dev
    /// because ATProto OAuth allows localhost.
    #[arg(long, env = "WUMBLR_PUBLIC_ORIGIN", default_value = "http://127.0.0.1:8787")]
    pub public_origin: String,

    /// freeq-auth-broker base URL (used for `/whoami` and DPoP-proxied PDS writes).
    /// Not yet contacted in M1 step 2 — wired in step 4.
    #[arg(long, env = "WUMBLR_BROKER_URL", default_value = "http://127.0.0.1:3080")]
    pub broker_url: String,

    /// Admin DIDs (comma-separated). DID-gates `/admin/*` endpoints.
    /// Empty in M1; populated when the approval queue lands in M5.
    #[arg(long, env = "WUMBLR_ADMIN_DIDS", default_value = "", value_delimiter = ',')]
    pub admin_dids: Vec<String>,

    /// Dev-only OAuth callback bridge target. When set, GET /auth/callback
    /// 302-redirects (or JS-redirects for fragment-bearing responses) to
    /// this URL + the original ?query and #fragment.
    ///
    /// Set to e.g. `http://127.0.0.1:8081/auth/callback` during local
    /// development. Leave unset in production; the frontend on
    /// Cloudflare Pages will own the URL directly.
    #[arg(long, env = "WUMBLR_DEV_CALLBACK_TARGET")]
    pub dev_callback_target: Option<String>,
}

impl Config {
    /// `client_id` URL ATProto OAuth uses to dereference the metadata JSON.
    pub fn oauth_client_id(&self) -> String {
        format!("{}/oauth-client-metadata.json", self.public_origin)
    }

    /// JWKS URL. Not referenced by the public OAuth client metadata in M1
    /// (`token_endpoint_auth_method: "none"` for the native+SPA flow) but
    /// kept around for future wumblr-owned PDS writes via `private_key_jwt`.
    #[allow(dead_code)]
    pub fn jwks_uri(&self) -> String {
        format!("{}/jwks.json", self.public_origin)
    }

    /// Web `redirect_uri` (used by `@atproto/oauth-client-browser`).
    pub fn web_redirect_uri(&self) -> String {
        format!("{}/auth/callback", self.public_origin)
    }
}
