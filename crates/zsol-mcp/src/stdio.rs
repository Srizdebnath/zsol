//! stdio transport (FR-4.1): MCP over line-delimited JSON-RPC 2.0 on stdin/stdout.
//! Diagnostics go to stderr ONLY — stdout is protocol-reserved.

use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::io::stdout;
use serde_json::Value;

use crate::ops::ToolCtx;
use crate::{mcp, proto};

/// Diagnostics classification for debug logs; never writes to stdout.
pub fn is_diag(_tool: &str) -> bool {
    true
}

pub async fn run(ctx: Arc<ToolCtx>) -> std::io::Result<()> {
    let mut out = stdout();
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let batch = proto::envelope(&line).unwrap_or_else(|(id, code, msg)| {
            vec![serde_json::json!({"pending_response_marker": true, "id": id, "code": code, "msg": msg})]
        });
        let mut responses: Vec<Value> = Vec::new();
        if batch.iter().all(|v| v.get("pending_response_marker").is_some()) {
            let (id, code, msg) = {
                let b = batch[0].clone();
                (
                    b.get("id").cloned().unwrap_or(Value::Null),
                    b.get("code").and_then(Value::as_i64).unwrap_or(proto::PARSE),
                    b.get("msg").and_then(Value::as_str).unwrap_or("parse error").to_string(),
                )
            };
            responses.push(proto::error(&id, code, &msg));
        } else {
            for v in &batch {
                responses.extend(mcp::dispatch_value(&ctx, v).await);
            }
        }
        for r in responses {
            out.write_all(serde_json::to_string(&r).unwrap_or_default().as_bytes()).await?;
            out.write_all(b"\n").await?;
            out.flush().await?;
        }
    }
    Ok(())
}
