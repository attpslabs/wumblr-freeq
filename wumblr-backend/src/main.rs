use std::net::SocketAddr;

use anyhow::{Context, Result};
use clap::Parser;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt};

mod app;
mod config;
mod routes;

use crate::config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();

    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("wumblr_backend=debug,tower_http=debug,info")),
        )
        .init();

    let config = Config::parse();
    let addr: SocketAddr = config
        .listen
        .parse()
        .with_context(|| format!("invalid listen address: {}", config.listen))?;

    info!(
        listen = %addr,
        public_origin = %config.public_origin,
        broker = %config.broker_url,
        "starting wumblr-backend"
    );

    let router = app::router(config);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind {addr}"))?;

    axum::serve(listener, router).await?;
    Ok(())
}
