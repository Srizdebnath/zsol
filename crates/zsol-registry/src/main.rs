//! zsol-registry server binary — binds ZSOL_REGISTRY_BIND (default 127.0.0.1:8088),
//! requires ZSOL_REGISTRY_DB (no silent default location for durable data).

use std::sync::{Arc, Mutex};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .json() // T-29: structured JSON logs (req_id added per request by middleware)
        .init();

    let bind = std::env::var("ZSOL_REGISTRY_BIND").unwrap_or_else(|_| "127.0.0.1:8088".to_string());
    let db_path = std::env::var("ZSOL_REGISTRY_DB")
        .map_err(|_| anyhow::anyhow!("ZSOL_REGISTRY_DB must be set (no silent db location)"))?;

    // mkdir -p the db parent when it's not flat
    if let Some(parent) = std::path::Path::new(&db_path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let mut conn: rusqlite::Connection = rusqlite::Connection::open(&db_path)?;
    zsol_registry::init_db(&mut conn)?;
    tracing::info!(%db_path, "registry db initialized (WAL, append-only triggers)");

    let state = zsol_registry::AppState {
        db: Arc::new(Mutex::new(conn)),
    };
    let limits = zsol_registry::Limits::from_env();
    tracing::info!(
        max_body = limits.max_body_bytes,
        rate_per_sec = limits.post_per_second,
        rate_burst = limits.post_burst,
        "limits (FR-3.4)"
    );
    let app = zsol_registry::router(state, limits);

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, "zsol-registry listening");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutdown signal received");
}
