//! zsol-mcp — MCP gateway for the zsol proof-of-funds attestation protocol.
//!
//! Transports (T-32/T-33):
//!   stdio (default): line-delimited JSON-RPC 2.0 on stdin/stdout — diagnostics stderr-only.
//!   streamable HTTP: bind ZSOL_MCP_HTTP_BIND (e.g. 127.0.0.1:8099), POST /mcp.
//! Both can run concurrently by binding HTTP while stdio is active.
//!
//! Config (all env): ZEBRA_RPC_URL, ZALLET_RPC_URL(+auth), ZSOL_REGISTRY_URL,
//! ZSOL_MCP_DRY_RUN (default 1), ZSOL_SHARE_ACCOUNTS, ZSOL_ATTEST_ACCOUNT,
//! ZSOL_RPC_METHOD_* pin overrides, ZSOL_ANCHOR_MAX_RETRIES.

use std::sync::Arc;

mod http;
mod mcp;
mod ops;
mod proto;
mod stdio;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr) // stdout is stdio-protocol-reserved
        .init();

    let ctx = ops::ToolCtx::from_env()
        .map_err(|e| anyhow::anyhow!("config: {e}"))?;
    tracing::info!(
        dry_run = ctx.dry_run,
        registry = %ctx.registry_url,
        "zsol-mcp starting (stdio MCP gateway)"
    );

    let http_bind = std::env::var("ZSOL_MCP_HTTP_BIND").ok().filter(|s| !s.is_empty());
    if let Some(bind) = http_bind {
        let app = http::router(Arc::clone(&ctx));
        let listener = tokio::net::TcpListener::bind(&bind).await?;
        tracing::info!(%bind, "zsol-mcp streamable HTTP listening on /mcp");
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                tracing::error!(err = %e, "http transport exited");
            }
        });
    }

    if std::env::var("ZSOL_MCP_NO_STDIO").map(|v| v == "1").unwrap_or(false) {
        // HTTP-only mode (daemon): idle forever.
        tracing::info!("http-only mode (ZSOL_MCP_NO_STDIO=1)");
        tokio::signal::ctrl_c().await?;
        return Ok(());
    }
    stdio::run(ctx).await?;
    Ok(())
}
