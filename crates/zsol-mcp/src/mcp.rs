//! MCP protocol dispatch: initialize / ping / tools/list / tools/call (MCP 1.0 surface,
//! hand-rolled per FR-4.1/4.2 — no SDK dependency, exact MCP method semantics).

use serde_json::{json, Value};

use crate::ops::{self, ToolCtx, ToolError};
use crate::proto::{error, result, Call, Incoming};

pub const SERVER_NAME: &str = "zsol-mcp";
pub const SERVER_VERSION: &str = "0.1.0";

fn tool_defs() -> Vec<Value> {
    let schema = |props: Value, req: Value| json!({"type": "object", "properties": props, "required": req, "additionalProperties": false});
    let int = json!({"type": "integer", "minimum": zsol_types::DUST_FLOOR_ZAT});
    vec![
        json!({"name": "zsol_health", "description": "Stack status: zebra ready / zallet verbs pinned / registry reachable / height. SPEC §8.", "inputSchema": schema(json!({}), json!([]))}),
        json!({"name": "zsol_prepare_anchor", "description": "Dry-run: checks attest account balance ≥ claimZat + fee floor, returns the planned anchor shape. Never sends.", "inputSchema": schema(json!({"claimZat": int}), json!(["claimZat"]))}),
        json!({"name": "zsol_anchor", "description": "PERFORMS the self-spend anchor (blocked by dry-run default, ZSOL_MCP_DRY_RUN). Returns txid + window height + mined-height evidence.", "inputSchema": schema(json!({"claimZat": int, "agentId": json!({"type": "string"}), "maxAgeHours": json!({"type": "integer", "minimum": 1, "maximum": 720})}), json!(["claimZat", "agentId"]))}),
        json!({"name": "zsol_make_attestation", "description": "Derives the attestation object (incl. memo binding) from the last anchored tx; returns canonical b64u + frame digest.", "inputSchema": schema(json!({"claimZat": json!({"type": "integer"}), "agentId": json!({"type": "string"})}), json!([]))}),
        json!({"name": "zsol_publish", "description": "Publishes a canonical object (b64u) to the registry POST /v0/attestations. Dry-run previews the exact body.", "inputSchema": schema(json!({"object": json!({"type": "string"})}), json!(["object"]))}),
        json!({"name": "zsol_share_scope", "description": "Exports the scoped uivk of a per-verifier CLEAN sub-account (never account #0, never master). Honest revocation note included.", "inputSchema": schema(json!({"peerLabel": json!({"type": "string"})}), json!(["peerLabel"]))}),
        json!({"name": "zsol_import_scope", "description": "Verifier: imports a uivk (never spend-class material) into the verifier Zallet; returns scan-state counts only.", "inputSchema": schema(json!({"uivk": json!({"type": "string"}), "label": json!({"type": "string"})}), json!(["uivk"]))}),
        json!({"name": "zsol_verify", "description": "Verifier: runs all §6 checks on an object; returns tier (verified|claim-only) + per-check reason rows (§9 codes).", "inputSchema": schema(json!({"object": json!({"type": "string"})}), json!(["object"]))}),
        json!({"name": "zsol_registry_fetch", "description": "Verifier: fetches an agent's registry entries; recomputes the digest chain independently per SPEC §7.", "inputSchema": schema(json!({"agentId": json!({"type": "string"})}), json!(["agentId"]))}),
    ]
}

async fn call_tool(ctx: &ToolCtx, name: &str, args: &Value) -> Result<Value, ToolError> {
    match name {
        "zsol_health" => ops::zsol_health(ctx).await,
        "zsol_prepare_anchor" => ops::prepare_anchor(ctx, args).await,
        "zsol_anchor" => ops::anchor(ctx, args).await,
        "zsol_make_attestation" => ops::make_attestation(ctx, args).await,
        "zsol_publish" => ops::publish(ctx, args).await,
        "zsol_share_scope" => ops::share_scope(ctx, args).await,
        "zsol_import_scope" => ops::import_scope(ctx, args).await,
        "zsol_verify" => ops::verify(ctx, args).await,
        "zsol_registry_fetch" => ops::registry_fetch(ctx, args).await,
        other => Err(ToolError::slug(
            "unknown-tool",
            format!("tool not found: {other}"),
        )),
    }
}

fn wrap(tool_name: &str, r: Result<Value, ToolError>) -> Value {
    let payload = match r {
        Ok(v) => json!({"content": [{"type": "text", "text": v.to_string()}], "isError": false}),
        Err(e) => json!({
            "content": [{"type": "text", "text": json!({
                "error": true,
                "reason": e.wire_reason(),
                "detail": e.detail,
            }).to_string()}],
            "isError": true,
        }),
    };
    if crate::stdio::is_diag(tool_name) {
        tracing::debug!(tool = tool_name, "tool call completed");
    }
    payload
}

async fn dispatch_call(ctx: &ToolCtx, c: &Call) -> Value {
    let Call(id, method, params) = c;
    match method.as_str() {
        "initialize" => {
            // Echo the client's advertised version when known, else our latest.
            let requested = params.get("protocolVersion").and_then(Value::as_str);
            let version = match requested {
                Some("2024-11-05") | Some("2025-03-26") | Some("2025-06-18") => requested.unwrap(),
                _ => "2025-03-26",
            };
            result(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": SERVER_NAME, "version": SERVER_VERSION},
                    "instructions": "zsol proof-of-funds attestation: attestor flow prepare_anchor→anchor→make_attestation→publish; verifier flow import_scope→registry_fetch→verify. Dry-run default: anchor sends nothing until ZSOL_MCP_DRY_RUN=0.",
                }),
            )
        }
        "ping" => result(id, json!({})),
        "tools/list" => result(id, json!({"tools": tool_defs()})),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            if name.is_empty() {
                return error(
                    id,
                    crate::proto::INVALID_REQUEST,
                    "tools/call requires name",
                );
            }
            result(id, wrap(name, call_tool(ctx, name, &args).await))
        }
        m => error(
            id,
            crate::proto::METHOD_NOT_FOUND,
            &format!("method not found: {m}"),
        ),
    }
}

/// Dispatch one parsed JSON value into MCP semantics. Returns the responses to transmit
/// (possibly empty — pure notifications are never answered, JSON-RPC 2.0 rule).
pub async fn dispatch_value(ctx: &ToolCtx, v: &Value) -> Vec<Value> {
    match crate::proto::parse_one(v) {
        Ok(Incoming::Call(c)) => vec![dispatch_call(ctx, &c).await],
        Ok(Incoming::Notification(m, _)) => {
            tracing::debug!(method = %m, "notification received (no reply)");
            vec![]
        }
        Err((code, msg)) => vec![error(&Value::Null, code, msg)],
    }
}
