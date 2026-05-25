use axum::Json;
use serde::Serialize;

#[derive(Serialize)]
pub struct Health {
    pub ok: bool,
    pub version: &'static str,
}

pub async fn health() -> Json<Health> {
    Json(Health {
        ok: true,
        version: env!("CARGO_PKG_VERSION"),
    })
}
