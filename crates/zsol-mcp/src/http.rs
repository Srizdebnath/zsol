//! Streamable HTTP transport (FR-4.2): POST /mcp JSON-RPC 2.0 (single or batch);
//! single JSON response (this server never streams results); GET/DELETE sessions are
//! honest 405s (no server-initiated streams in v0).

use std::sync::Arc;

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use serde_json::Value;

use crate::ops::ToolCtx;
use crate::{mcp, proto};

pub fn router(ctx: Arc<ToolCtx>) -> Router {
    Router::new()
        .route("/mcp", get(no_get).delete(no_delete).post(post_mcp))
        .with_state(ctx)
}

async fn no_get() -> impl IntoResponse {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        [(header::ALLOW, "POST, DELETE")],
        "server does not stream in v0",
    )
}

async fn no_delete() -> impl IntoResponse {
    (StatusCode::OK, "session-less server: nothing to clear")
}

async fn post_mcp(State(ctx): State<Arc<ToolCtx>>, body: String) -> impl IntoResponse {
    let values = match proto::envelope(&body) {
        Ok(vs) => vs,
        Err((id, code, msg)) => {
            return (
                StatusCode::OK,
                serde_json::to_string(&proto::error(&id, code, &msg)).unwrap_or_default(),
            )
        }
    };
    let mut responses = Vec::new();
    for v in &values {
        responses.extend(mcp::dispatch_value(&ctx, v).await);
    }
    let out = match responses.len() {
        0 => Value::Null,
        1 => responses.into_iter().next().unwrap_or(Value::Null),
        _ => Value::Array(responses),
    };
    (StatusCode::OK, serde_json::to_string(&out).unwrap_or_default())
}
