//! MCP protocol dispatch: initialize / ping / tools/list / tools/call (MCP 1.0 surface,
//! hand-rolled per FR-4.1/4.2 — no SDK dependency, exact MCP method semantics).

use serde_json::{Value, json};

use crate::ops::{self, ToolCtx, ToolError};
use crate::proto::{Call, Incoming, error, result};

pub const SERVER_NAME: &str = "zsol-mcp";
pub const SERVER_VERSION: &str = "0.2.0";

fn tool_defs() -> Vec<Value> {
    let schema = |props: Value, req: Value| json!({"type": "object", "properties": props, "required": req, "additionalProperties": false});
    let obj = json!({"type": "string", "description": "attestation object, b64u (default: this wallet's newest mined attestation)"});
    let tool = |name: &str, desc: &str, input: Value| json!({"name": name, "description": desc, "inputSchema": input});
    vec![
        tool(
            "zsol_health",
            "Live status: lightwalletd endpoint boot check (network + NU7 consensus branch), wallet presence, registry and Credit Desk reachability, spend permission.",
            schema(json!({}), json!([])),
        ),
        tool(
            "zsol_wallet_status",
            "Sync this agent's wallet to the chain tip; return balances per account and shielded receive addresses.",
            schema(json!({}), json!([])),
        ),
        tool(
            "zsol_anchor",
            "ATTESTOR, BROADCASTS: self-spend inside the dedicated zsol-attn account carrying memo 0xFF‖zs02‖D(frame); waits until mined and returns the finished attestation object (SPEC §5). Requires ZSOL_ALLOW_SPEND=1.",
            schema(
                json!({"agentId": {"type": "string", "maxLength": 64}, "claimZat": {"type": "integer", "minimum": zsol_types::DUST_FLOOR_ZAT}, "maxAgeHours": {"type": "integer", "minimum": 1, "maximum": 720}}),
                json!(["agentId", "claimZat"]),
            ),
        ),
        tool(
            "zsol_attestations",
            "ATTESTOR: list this wallet's attestations (finishing any whose anchor has since been mined).",
            schema(json!({}), json!([])),
        ),
        tool(
            "zsol_publish",
            "ATTESTOR: publish an attestation's claim tier to the public registry (no keys, no balances).",
            schema(json!({"object": obj.clone()}), json!([])),
        ),
        tool(
            "zsol_present",
            "ATTESTOR: answer a verifier's challenge: signs it with the session key and returns {object, ufvk, challenge, signature}. Discloses the zsol-attn account history to that verifier only.",
            schema(
                json!({"challenge": {"type": "object"}, "object": obj.clone()}),
                json!(["challenge"]),
            ),
        ),
        tool(
            "zsol_release",
            "ATTESTOR, BROADCASTS: move the zsol-attn balance to commerce, ending every attestation; watching verifiers see it within one block (SPEC §8).",
            schema(json!({}), json!([])),
        ),
        tool(
            "credit_request",
            "BORROWER: ask the Credit Desk for pay-later credit on a Bitrefill product, proving live solvency with the newest attestation (challenge-response). Returns the loan, the redemption payload and repayment instructions.",
            schema(
                json!({"productId": {"type": "string", "description": "Bitrefill product id, e.g. test-gift-card-code"}, "valueUsd": {"type": "integer", "minimum": 1}, "object": obj.clone()}),
                json!(["productId", "valueUsd"]),
            ),
        ),
        tool(
            "credit_repay",
            "BORROWER, BROADCASTS: repay a Credit Desk loan from the commerce account with the loan memo (shielded).",
            schema(json!({"loanId": {"type": "string"}}), json!(["loanId"])),
        ),
        tool(
            "zsol_challenge",
            "VERIFIER: issue a fresh challenge (32-byte nonce, 10-minute expiry) for an attestor to sign.",
            schema(json!({}), json!([])),
        ),
        tool(
            "zsol_verify",
            "VERIFIER: run checks C1–C8 (SPEC §6) on a presentation in this agent's own view-only wallet; returns tier verified|claim-only with a typed row per check.",
            schema(
                json!({"presentation": {"type": "object"}}),
                json!(["presentation"]),
            ),
        ),
        tool(
            "zsol_watch",
            "VERIFIER: one solvency-watch tick for an attestation already verified here: live, released (anchored notes spent) or expired.",
            schema(json!({"object": obj.clone()}), json!([])),
        ),
        tool(
            "zsol_registry_fetch",
            "Fetch an agent's registry entries and recompute the digest chain locally.",
            schema(json!({"agentId": {"type": "string"}}), json!(["agentId"])),
        ),
    ]
}

async fn call_tool(ctx: &ToolCtx, name: &str, args: &Value) -> Result<Value, ToolError> {
    match name {
        "zsol_health" => ops::health(ctx).await,
        "zsol_wallet_status" => ops::wallet_status(ctx).await,
        "zsol_anchor" => ops::anchor(ctx, args).await,
        "zsol_attestations" => ops::attestations(ctx).await,
        "zsol_publish" => ops::publish(ctx, args).await,
        "zsol_present" => ops::present(ctx, args).await,
        "zsol_release" => ops::release(ctx).await,
        "credit_request" => ops::credit_request(ctx, args).await,
        "credit_repay" => ops::credit_repay(ctx, args).await,
        "zsol_challenge" => ops::challenge(ctx).await,
        "zsol_verify" => ops::verify(ctx, args).await,
        "zsol_watch" => ops::watch(ctx, args).await,
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
                    "instructions": "zsol: live, private proof of funds on Zcash. Attestor: zsol_wallet_status → zsol_anchor → zsol_publish → zsol_present (or credit_request at the Credit Desk) → credit_repay. Verifier: zsol_challenge → zsol_verify → zsol_watch. Broadcasting tools need ZSOL_ALLOW_SPEND=1.",
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
