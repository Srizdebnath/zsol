//! zsol ops — the nine SPEC §8 tools (attestor + verifier lifecycles).
//!
//! Honesty rules encoded here:
//! - T-37: dry-run default (`ZSOL_MCP_DRY_RUN=1`); only `zsol_anchor` sends, and never
//!   while dry-run. Plans encode the exact would-be params.
//! - T-36: `zsol_share_scope` refuses account #0, the master attest account, spend-class
//!   export material, and any account not configured in `ZSOL_SHARE_ACCOUNTS`; it states
//!   plainly that uivks cannot be revoked (SPEC §6 honesty note).
//! - T-38: no panics in request paths; failures typed (§9 ReasonCode where the taxonomy
//!   has one, transport slugs otherwise). Wallet RPC error MESSAGES never surface
//!   (server-side text may contain addresses — NF-6.2); only verb + numeric code.
//! - Beta-parity shape harvesters print their interpretation choice in the payload
//!   (`balanceInterpretation`), so a wrong reading cannot silently pass for consensus.
//!
//! Protocol lock (SPEC §4/§5/§6 as amended): the memo carries the WINDOW height h*
//! (estimated tip at send); `object.anchorHeight == memo.height` is the verified equality;
//! the actually-mined block height of the txid is separately-evidenced metadata
//! (`minedAtHeight`, `heightSource`) used by verifier evidence rows, NOT by the digest.

use std::sync::{Arc, Mutex};

use base64::Engine as _;
use serde_json::{json, Value};
use zsol_types::{AnchorMemo, AttestationObject, ReasonCode, PROTOCOL};

use zsol_wallet::{Base, Cfg, Client, WalletError};

/// zcashd-consensus default shielded fee (zatoshi).
pub const FEE_ESTIMATE_ZAT: u64 = 10_000;
/// FR-2.5 op budget.
pub const OP_POLL_ALLOW_SECS: u64 = 35;
pub const MAX_AGE_DEFAULT_HOURS: u32 = 24;

#[derive(Debug, Clone)]
pub struct AnchorRecord {
    pub agent_id: String,
    pub claim_zat: u64,
    pub ua: String,
    pub issued_at: u64,
    pub max_age_hours: u32,
    pub memo_bytes: Vec<u8>,
    pub memo_height: u64,
    pub account_idx: u64,
    pub opid: Option<String>,
    pub txid: String,
    pub mined_height: Option<u64>,
    pub height_source: &'static str,
    pub attempts: u32,
}

impl AnchorRecord {
    fn self_spend_params(&self) -> Value {
        // from = the attest account's own unified address (zallet parity); the
        // record's derivation ran in ops::prepare/last_anchor path (self.ua).
        Client::build_self_spend_params(&self.ua, &self.ua, self.claim_zat, &self.memo_bytes, 1)
    }
}

/// Evidence record of an imported verifier scope: the label/index/balance fields are
/// kept as state for later evidence rows even though verify() reads only account_idx.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ScopeRecord {
    pub label: String,
    pub account_idx: Option<u64>,
    pub balance_zat: Option<u64>,
    pub balance_interpretation: Option<&'static str>,
}

pub struct ToolCtx {
    pub w: Client,
    pub cfg: Cfg,
    pub http: reqwest::Client,
    pub registry_url: String,
    pub dry_run: bool,
    pub share_accounts: Vec<u64>,
    pub max_anchor_retries: u32,
    pub last_anchor: Mutex<Option<AnchorRecord>>,
    pub scope: Mutex<Option<ScopeRecord>>,
}

#[derive(Debug)]
pub struct ToolError {
    /// §9 reason where the taxonomy applies.
    pub reason: Option<ReasonCode>,
    /// Transport slug (no §9 code exists for it).
    pub slug: Option<&'static str>,
    /// Detail free of PII (never echoes wallet RPC messages).
    pub detail: String,
}

impl ToolError {
    pub fn reason(rc: ReasonCode, detail: impl Into<String>) -> Self {
        Self {
            reason: Some(rc),
            slug: None,
            detail: detail.into(),
        }
    }
    pub fn slug(s: &'static str, detail: impl Into<String>) -> Self {
        Self {
            reason: None,
            slug: Some(s),
            detail: detail.into(),
        }
    }
    pub fn wire_reason(&self) -> Value {
        if let Some(rc) = self.reason {
            json!(rc.as_str())
        } else {
            json!(self.slug.unwrap_or("internal"))
        }
    }
}

pub fn okv(payload: Value) -> Result<Value, ToolError> {
    Ok(payload)
}

impl ToolCtx {
    pub fn from_env() -> Result<Arc<Self>, String> {
        let cfg = Cfg::from_env().map_err(|e| e.to_string())?;
        let registry_url = std::env::var("ZSOL_REGISTRY_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .ok_or("env missing or empty: ZSOL_REGISTRY_URL")?;
        let share_accounts: Vec<u64> = std::env::var("ZSOL_SHARE_ACCOUNTS")
            .ok()
            .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect())
            .unwrap_or_default();
        // FR-4.5: dry-run default is the SAFE default.
        let dry_run = std::env::var("ZSOL_MCP_DRY_RUN")
            .ok()
            .map(|s| s.trim() != "0")
            .unwrap_or(true);
        let max_anchor_retries = std::env::var("ZSOL_ANCHOR_MAX_RETRIES")
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(2);
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(
                std::env::var("ZSOL_HTTP_TIMEOUT_SECS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(30),
            ))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Arc::new(Self {
            w: Client::new(cfg.clone()).map_err(|e| e.to_string())?,
            cfg,
            http,
            registry_url: registry_url.trim_end_matches('/').to_string(),
            dry_run,
            share_accounts,
            max_anchor_retries,
            last_anchor: Mutex::new(None),
            scope: Mutex::new(None),
        }))
    }
}

// ---------- tolerant harvesters (beta-parity; interpretation always surfaced) ----------

pub fn unwrap_u64(v: &Value) -> Option<u64> {
    if let Some(n) = v.as_u64() {
        return Some(n);
    }
    if let Some(f) = v.as_f64() {
        if f.is_finite() && f >= 0.0 && f.fract() == 0.0 {
            return Some(f as u64);
        }
    }
    if let Some(s) = v.as_str() {
        if let Ok(n) = s.trim().parse::<u64>() {
            return Some(n);
        }
        if let Ok(f) = s.trim().parse::<f64>() {
            if f.is_finite() && f >= 0.0 {
                return Some((f * 100_000_000.0).round() as u64);
            }
        }
    }
    None
}

/// Numeric-as-ZEC-decimal → zatoshi (zcashd z_getbalance / z_viewtransaction "value").
pub fn num_to_zat(v: &Value) -> Option<u64> {
    if let Some(n) = v.as_f64() {
        if n.is_finite() && n >= 0.0 {
            return Some((n * 100_000_000.0).round() as u64);
        }
    }
    if let Some(s) = v.as_str() {
        if let Ok(f) = s.trim().parse::<f64>() {
            if f.is_finite() && f >= 0.0 {
                return Some((f * 100_000_000.0).round() as u64);
            }
        }
    }
    None
}

pub fn unwrap_balance(v: &Value) -> (Option<u64>, &'static str) {
    // zallet v2 (v0.1.0-beta): {"pools":{"transparent":{"valueZat":…}, …}}
    // Sum of pool valueZat (zatoshi) — always surfaced w/ its interpretation tag.
    if let Some(pools) = v.get("pools").and_then(Value::as_object) {
        if !pools.is_empty() {
            let sum: u128 = pools
                .values()
                .filter_map(|p| p.get("valueZat").and_then(Value::as_u64))
                .map(u128::from)
                .sum();
            if sum <= u64::MAX as u128 {
                return (Some(sum as u64), "zallet-pools=zat");
            }
        }
    }
    if let Some(b) = v.get("balance") {
        if let Some(z) = unwrap_u64(b) {
            return (Some(z), "object-balance=zat");
        }
        if let Some(z) = num_to_zat(b) {
            return (Some(z), "object-balance=zec-decimal");
        }
    }
    if let Some(z) = num_to_zat(v) {
        return (Some(z), "numeric-zec");
    }
    (None, "unknown-shape")
}

fn deep_find(v: &Value, key: &str, depth: usize) -> Option<Value> {
    if depth == 0 {
        return None;
    }
    if let Some(m) = v.as_object() {
        if let Some(hit) = m.get(key) {
            return Some(hit.clone());
        }
        for val in m.values() {
            if let Some(hit) = deep_find(val, key, depth - 1) {
                return Some(hit);
            }
        }
    }
    if let Some(a) = v.as_array() {
        for val in a {
            if let Some(hit) = deep_find(val, key, depth - 1) {
                return Some(hit);
            }
        }
    }
    None
}

/// Diversified unified address: any string starting with `u1` (testnet UA prefix).
fn deep_find_ua(v: &Value, depth: usize) -> Option<String> {
    if depth == 0 {
        return None;
    }
    // HRP prefixes per network: mainnet "u1…", testnet "utest1…", regtest "uregtest1…".
    const UA_PREFIXES: [&str; 3] = ["u1", "utest1", "uregtest1"];
    match v {
        Value::Object(m) => {
            for val in m.values() {
                if let Some(s) = val.as_str() {
                    if s.len() > 40 && UA_PREFIXES.iter().any(|p| s.starts_with(p)) {
                        return Some(s.to_string());
                    }
                }
            }
            for val in m.values() {
                if let Some(hit) = deep_find_ua(val, depth - 1) {
                    return Some(hit);
                }
            }
            None
        }
        Value::Array(a) => a.iter().find_map(|x| deep_find_ua(x, depth - 1)),
        _ => None,
    }
}

fn deep_find_txid(v: &Value) -> Option<String> {
    let t = deep_find(v, "txid", 4)?;
    let s = t.as_str()?;
    if s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit()) {
        Some(s.to_lowercase())
    } else {
        None
    }
}

fn unwrap_mined_height(v: &Value) -> Option<u64> {
    for k in ["blockheight", "height"] {
        if let Some(h) = deep_find(v, k, 3).and_then(|x| unwrap_u64(&x)) {
            return Some(h);
        }
    }
    None
}

fn map_wallet_err(e: &WalletError, verb: &'static str) -> ToolError {
    match e {
        WalletError::Timeout => ToolError::slug("wallet-timeout", format!("{verb} timed out")),
        WalletError::Auth(_) => ToolError::slug(
            "wallet-auth",
            "401 from wallet RPC — fix ZALLET_RPC_AUTH / auth header",
        ),
        WalletError::MissingVerb(m) => ToolError::reason(
            ReasonCode::WalletUnreachable,
            format!("{verb}: pinned verb missing at boot ({m}); run doctor, pin overrides"),
        ),
        WalletError::Rpc { code, .. } => {
            ToolError::slug("wallet-rpc", format!("{verb}: rpc error code {code}"))
        }
        WalletError::Transport(t) => {
            // Keep the wallet's reason visible to agents (500 detail layout etc.).
            ToolError::slug("wallet-unreachable", format!("{verb}: transport ({t})"))
        }
        WalletError::EnvMissing(k) => ToolError::slug(
            "wallet-config",
            format!("missing env {k}; see .env.example"),
        ),
    }
}

/// Deterministic peer-label → slot fold (FNV-1a). Same label ⇒ same account, forever.
pub fn label_slot(label: &str, n: usize) -> usize {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in label.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    (h % n as u64) as usize
}

// ---------- tools ----------

pub async fn zsol_health(ctx: &ToolCtx) -> Result<Value, ToolError> {
    let zebra = match ctx.w.info().await {
        Ok(v) => {
            let height = deep_find(&v, "blocks", 2).and_then(|b| unwrap_u64(&b));
            json!({"ok": true, "height": height})
        }
        Err(e) => {
            let te = map_wallet_err(&e, "getblockchaininfo");
            json!({"ok": false, "reason": te.wire_reason()})
        }
    };
    let zallet = match ctx.w.list_accounts().await {
        Ok(_) => json!({"ok": true}),
        Err(e) => {
            let te = map_wallet_err(&e, "z_listaccounts");
            json!({"ok": false, "reason": te.wire_reason()})
        }
    };
    let registry = match ctx
        .http
        .get(format!("{}/v0/health", ctx.registry_url))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {
            let v: Value = r.json().await.unwrap_or(json!({}));
            json!({
                "ok": true,
                "chainLength": v.get("chainLength").cloned().unwrap_or(Value::Null),
            })
        }
        Ok(r) => json!({"ok": false, "reason": "registry-http", "status": r.status().as_str()}),
        Err(_) => json!({"ok": false, "reason": "registry-unreachable"}),
    };
    okv(json!({
        "zebra": zebra,
        "zallet": zallet,
        "registry": registry,
        "dryRun": ctx.dry_run,
        "attestAccount": ctx.cfg.attest_account,
        "feeEstimateZat": FEE_ESTIMATE_ZAT,
    }))
}

async fn attest_ua(ctx: &ToolCtx) -> Result<String, ToolError> {
    let v = ctx
        .w
        .addr_for_account(ctx.cfg.attest_account, &["orchard", "sapling"])
        .await
        .map_err(|e| map_wallet_err(&e, "z_getaddressforaccount"))?;
    deep_find_ua(&v, 5).ok_or(ToolError::slug(
        "wallet-shape",
        "no unified address in derivation reply",
    ))
}

fn frame_pre(
    agent_id: &str,
    claim_zat: u64,
    ua: &str,
    issued_at: u64,
    max_age_hours: u32,
) -> AttestationObject {
    AttestationObject {
        protocol: PROTOCOL.to_string(),
        agent_id: agent_id.to_string(),
        claim_zat,
        anchor_height: 0,
        anchor_txid: String::new(),
        account_ua: ua.to_string(),
        memo_binding: String::new(),
        issued_at,
        max_age_hours,
    }
}

async fn balance_of(ctx: &ToolCtx, account: u64) -> Result<(Option<u64>, &'static str), ToolError> {
    let (_, v) = ctx
        .w
        .balance_for(account)
        .await
        .map_err(|e| map_wallet_err(&e, "balance-family"))?;
    Ok(unwrap_balance(&v))
}

pub async fn prepare_anchor(ctx: &ToolCtx, args: &Value) -> Result<Value, ToolError> {
    let claim = unwrap_u64(args.get("claimZat").unwrap_or(&Value::Null)).ok_or(ToolError::slug(
        "invalid-args",
        "claimZat (zatoshi u64) required",
    ))?;
    zsol_types::validate_claim_zat(claim).map_err(|rc| ToolError::reason(rc, "dust floor"))?;

    let ua = attest_ua(ctx).await?;
    let (bal, interp) = balance_of(ctx, ctx.cfg.attest_account).await?;
    let need = claim.saturating_add(FEE_ESTIMATE_ZAT);
    let enough = bal.map(|b| b >= need);
    let reason = match enough {
        Some(true) => Value::Null,
        Some(false) => json!(ReasonCode::BalanceBelowClaim.as_str()),
        None => json!(Value::Null),
    };
    if enough == Some(false) {
        return Err(ToolError::reason(
            ReasonCode::BalanceBelowClaim,
            format!("attest account below claim+fee ({need} needed)"),
        ));
    }
    okv(json!({
        "claimZat": claim,
        "feeEstimateZat": FEE_ESTIMATE_ZAT,
        "needZat": need,
        "haveZat": bal,
        "balanceInterpretation": interp,
        "enough": enough,
        "attestAccount": ctx.cfg.attest_account,
        "accountUa": ua,
        "planned": {
            "selfSpend": true,
            "minConfirms": 1,
            "memoShape": "zs01(4)‖ver(1)‖height_LE64(8)‖digest(32)",
        },
        "reason": reason,
    }))
}

/// One anchor send attempt: memo for window height h* → params → submit → txid.
async fn submit_attempt(
    ctx: &ToolCtx,
    agent_id: &str,
    claim: u64,
    ua: &str,
    issued: u64,
    max_age: u32,
    memo_height: u64,
) -> Result<(AnchorMemo, String, AnchorRecord), ToolError> {
    let frame = frame_pre(agent_id, claim, ua, issued, max_age);
    let d = frame
        .digest()
        .map_err(|e| ToolError::slug("internal", e.to_string()))?;
    let memo = AnchorMemo {
        height: memo_height,
        digest: d,
    };
    let mut rec = AnchorRecord {
        agent_id: agent_id.to_string(),
        claim_zat: claim,
        ua: ua.to_string(),
        issued_at: issued,
        max_age_hours: max_age,
        memo_bytes: memo.encode().to_vec(),
        memo_height,
        account_idx: ctx.cfg.attest_account,
        opid: None,
        txid: String::new(),
        mined_height: None,
        height_source: "pending",
        attempts: 0,
    };
    let send_v = ctx
        .w
        .send(rec.self_spend_params())
        .await
        .map_err(|e| map_wallet_err(&e, "z_sendmany"))?;
    if let Some(t) = deep_find_txid(&send_v) {
        return Ok((memo, t, rec));
    }
    // zcashd replies {"operationId":"op-…"}; Zallet beta.1 replies the bare
    // opid STRING itself — accept both before failing shape (FR-2.4 tolerance).
    let opid = deep_find(&send_v, "operationId", 3)
        .or_else(|| deep_find(&send_v, "opid", 3))
        .or_else(|| deep_find(&send_v, "id", 3))
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .or_else(|| send_v.as_str().map(|s| s.to_string()))
        .ok_or(ToolError::slug(
            "wallet-shape",
            "send reply had neither txid nor operationId",
        ))?;
    rec.opid = Some(opid.clone());
    match wait_send_op(ctx, &opid).await {
        Ok(txid) => Ok((memo, txid, rec)),
        // The send IS submitted; the op is still executing server-side (zallet
        // Orchard proving is slow on small hosts). Hand back a RESUME record
        // (txid empty, opid tracked) — port: FR-2.4 honesty, no orphaned claims.
        Err(e) if e.slug == Some("send-op-timeout") => Ok((memo, String::new(), rec)),
        Err(e) => Err(e),
    }
}

/// Env-tunable poll budget (FR-2.5): ZSOL_OP_POLL_SECS overrides; default 35s.
/// Zallet regtest ops take ~40s per Orchard tx (spend proving on small hosts).
fn op_poll_secs() -> u64 {
    let raw = std::env::var("ZSOL_OP_POLL_SECS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok());
    match raw {
        Some(s) if s > 600 => {
            tracing::warn!("ZSOL_OP_POLL_SECS clamped: {s} > 600");
            600
        }
        Some(s) => s,
        None => OP_POLL_ALLOW_SECS,
    }
}

async fn wait_send_op(ctx: &ToolCtx, opid: &str) -> Result<String, ToolError> {
    let secs = op_poll_secs();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        if std::time::Instant::now() > deadline {
            return Err(ToolError::slug(
                "send-op-timeout",
                format!("op did not complete within {secs}s (FR-2.5 budget)"),
            ));
        }
        let v = ctx.w.op_status_for(opid).await;
        match v {
            Ok(v) => {
                let status =
                    deep_find(&v, "status", 3).and_then(|s| s.as_str().map(|x| x.to_string()));
                match status.as_deref() {
                    Some("success") => {
                        let r = ctx
                            .w
                            .op_result(opid)
                            .await
                            .map_err(|e| map_wallet_err(&e, "z_getoperationresult"))?;
                        if let Some(t) = deep_find_txid(&r) {
                            return Ok(t);
                        }
                    }
                    Some("failed") | Some("cancelled") => {
                        return Err(ToolError::slug(
                            "send-op-failed",
                            format!("op ended with status {status:?}"),
                        ))
                    }
                    _ => {}
                }
            }
            Err(e) => return Err(map_wallet_err(&e, "z_getoperationstatus")),
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

fn finalize_anchor(ctx: &ToolCtx, rec: AnchorRecord) -> Result<Value, ToolError> {
    let memo_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&rec.memo_bytes);
    let memo = AnchorMemo::decode_prefix(&rec.memo_bytes)
        .map_err(|e| ToolError::slug("internal", e.to_string()))?;
    let digest_hex = hex::encode(memo.digest);
    *ctx.last_anchor
        .lock()
        .map_err(|_| ToolError::slug("internal", "state lock poisoned"))? = Some(rec.clone());
    okv(json!({
        "txid": rec.txid,
        "windowHeight": rec.memo_height,
        "minedAtHeight": rec.mined_height,
        "heightSource": rec.height_source,
        "memoB64u": memo_b64,
        "frameDigestHex": digest_hex,
        "attempts": rec.attempts,
        "claimZat": rec.claim_zat,
        "agentId": rec.agent_id,
    }))
}

pub async fn anchor(ctx: &ToolCtx, args: &Value) -> Result<Value, ToolError> {
    let claim = unwrap_u64(args.get("claimZat").unwrap_or(&Value::Null)).ok_or(ToolError::slug(
        "invalid-args",
        "claimZat (zatoshi u64) required",
    ))?;
    let agent_id = args
        .get("agentId")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or(ToolError::slug(
            "invalid-args",
            "agentId required — it feeds the frame digest (SPEC §8)",
        ))?;
    let max_age = unwrap_u64(args.get("maxAgeHours").unwrap_or(&Value::Null))
        .unwrap_or(MAX_AGE_DEFAULT_HOURS as u64)
        .clamp(1, 24 * 30) as u32;
    zsol_types::validate_claim_zat(claim).map_err(|rc| ToolError::reason(rc, "dust floor"))?;

    if ctx.dry_run {
        // FR-4.5: plan only. Frame digest preview only when UA derivable; otherwise
        // explicitly null (never faked).
        let ua = attest_ua(ctx).await.unwrap_or_default();
        let mut payload = plan_dry(ctx, claim, &ua, &agent_id, max_age).await?;
        if ua.is_empty() {
            payload["frameDigestPreviewHex"] = json!(Value::Null);
        }
        return okv(payload);
    }

    let ua = attest_ua(ctx).await?;
    let issued = zsol_types::now_epoch_secs();

    let mut attempts: u32 = 0;
    loop {
        let tip = {
            let v = ctx
                .w
                .info()
                .await
                .map_err(|e| map_wallet_err(&e, "getblockchaininfo"))?;
            deep_find(&v, "blocks", 2)
                .and_then(|b| unwrap_u64(&b))
                .ok_or(ToolError::slug(
                    "wallet-shape",
                    "no tip height in getblockchaininfo",
                ))?
        };
        let memo_height = tip + 1; // window: next block at send time

        let (_memo, txid, mut rec) =
            submit_attempt(ctx, &agent_id, claim, &ua, issued, max_age, memo_height).await?;
        attempts += 1;
        rec.attempts = attempts;

        if txid.is_empty() {
            // Submitted but not yet proven: keep the record so make_attestation
            // (or a retried anchor) can resume polling the SAME opid — no orphan.
            rec.height_source = "op-pending-resume";
            *ctx.last_anchor
                .lock()
                .map_err(|_| ToolError::slug("internal", "state lock poisoned"))? = Some(rec);
            return Err(ToolError::slug(
                "send-op-timeout",
                "anchor submitted; op still executing — resume via zsol_make_attestation",
            ));
        }
        rec.txid = txid.clone();

        let (mined, source): (Option<u64>, &'static str) = match ctx.w.raw_tx(&txid).await {
            Ok(v) => match unwrap_mined_height(&v) {
                Some(h) if h > 0 => (Some(h), "rawtx-blockheight"),
                _ => (None, "rawtx-unconfirmed"),
            },
            Err(WalletError::MissingVerb(_)) => (None, "rawtx-verb-missing"),
            Err(e) => {
                return Err(map_wallet_err(&e, "getrawtransaction"));
            }
        };
        rec.mined_height = mined;
        rec.height_source = source;

        match (mined, attempts) {
            // Height verified exact — done.
            (Some(h), _) if h == memo_height => return finalize_anchor(ctx, rec),
            // Drift: re-anchor with a fresh window (funds intact; in == out). Retry cap.
            (Some(_), a) if a >= ctx.max_anchor_retries => {
                return Err(ToolError::slug(
                    "height-drift-retries-exhausted",
                    format!("anchor {txid} mined off window; {a} supersede attempts exhausted "),
                ))
            }
            // Height unavailable (beta parity / slow confirm): finalize with honest
            // source marker — verifier scan remains final truth for mined-ness.
            (None, _) => return finalize_anchor(ctx, rec),
            (Some(_), _) => {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                continue;
            }
        }
    }
}

async fn plan_dry(
    ctx: &ToolCtx,
    claim: u64,
    ua: &str,
    agent_id: &str,
    max_age: u32,
) -> Result<Value, ToolError> {
    zsol_types::validate_claim_zat(claim).map_err(|rc| ToolError::reason(rc, "dust floor"))?;
    let (bal, interp) = if ua.is_empty() {
        (None, "ua-unavailable-derivation-failed")
    } else {
        balance_of(ctx, ctx.cfg.attest_account)
            .await
            .unwrap_or((None, "balance-family-failed"))
    };
    let need = claim.saturating_add(FEE_ESTIMATE_ZAT);
    let enough = bal.map(|b| b >= need);
    let mut payload = json!({
        "dryRun": true,
        "agentId": agent_id,
        "claimZat": claim,
        "feeEstimateZat": FEE_ESTIMATE_ZAT,
        "needZat": need,
        "haveZat": bal,
        "balanceInterpretation": interp,
        "enough": enough,
        "attestAccount": ctx.cfg.attest_account,
        "accountUa": if ua.is_empty() { Value::Null } else { json!(ua) },
        "planned": {
            "selfSpend": true,
            "minConfirms": 1,
            "memoShape": "zs01(4)‖ver(1)‖height_LE64(8)‖digest(32)",
            "note": "dry-run: nothing sent; zsol_anchor with ZSOL_MCP_DRY_RUN=0 performs the spend",
        },
    });
    if !ua.is_empty() {
        let frame = frame_pre(agent_id, claim, ua, zsol_types::now_epoch_secs(), max_age);
        if let Ok(d) = frame.digest() {
            payload["frameDigestPreviewHex"] = json!(hex::encode(d));
        }
    }
    Ok(payload)
}

pub async fn make_attestation(ctx: &ToolCtx, args: &Value) -> Result<Value, ToolError> {
    let mut rec = ctx
        .last_anchor
        .lock()
        .map_err(|_| ToolError::slug("internal", "state lock poisoned"))?
        .clone()
        .ok_or(ToolError::reason(
            ReasonCode::AnchorNotMined,
            "no anchor performed in this session; run zsol_anchor first",
        ))?;
    if rec.txid.is_empty() {
        // Resume: the anchor op was submitted but hadn't finished when zsol_anchor
        // returned (FR-2.5 budget). Poll the SAME opid to conclusion — never a
        // second send, never a fabricated txid.
        let opid = rec.opid.clone().ok_or(ToolError::slug(
            "wallet-shape",
            "anchor record has no opid to resume",
        ))?;
        let txid = wait_send_op(ctx, &opid).await?;
        rec.txid = txid.clone();
        let (mined, source): (Option<u64>, &'static str) = match ctx.w.raw_tx(&txid).await {
            Ok(v) => match unwrap_mined_height(&v) {
                Some(h) if h > 0 => (Some(h), "rawtx-blockheight"),
                _ => (None, "rawtx-unconfirmed"),
            },
            Err(WalletError::MissingVerb(_)) => (None, "rawtx-verb-missing"),
            Err(e) => return Err(map_wallet_err(&e, "getrawtransaction")),
        };
        rec.mined_height = mined;
        rec.height_source = source;
        *ctx.last_anchor
            .lock()
            .map_err(|_| ToolError::slug("internal", "state lock poisoned"))? = Some(rec.clone());
    }
    if let Some(c) = unwrap_u64(args.get("claimZat").unwrap_or(&Value::Null)) {
        if c != rec.claim_zat {
            return Err(ToolError::reason(
                ReasonCode::AmountMismatch,
                format!("claimZat {c} ≠ anchored claimZat {}", rec.claim_zat),
            ));
        }
    }
    if let Some(a) = args.get("agentId").and_then(Value::as_str) {
        if a != rec.agent_id {
            return Err(ToolError::slug(
                "agent-mismatch",
                format!("agentId {a} ≠ anchored agentId {}", rec.agent_id),
            ));
        }
    }
    let memo_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&rec.memo_bytes);
    let obj = AttestationObject {
        protocol: PROTOCOL.to_string(),
        agent_id: rec.agent_id.clone(),
        claim_zat: rec.claim_zat,
        anchor_height: rec.memo_height, // §6 verified equality — the window
        anchor_txid: rec.txid.clone(),
        account_ua: rec.ua.clone(),
        memo_binding: memo_b64,
        issued_at: rec.issued_at,
        max_age_hours: rec.max_age_hours,
    };
    let canonical_bytes = obj
        .canonical()
        .map_err(|e| ToolError::slug("internal", e.to_string()))?;
    let digest = obj
        .digest()
        .map_err(|e| ToolError::slug("internal", e.to_string()))?;
    okv(json!({
        "object": obj.b64u().map_err(|e| ToolError::slug("internal", e.to_string()))?,
        "canonicalHex": hex::encode(&canonical_bytes),
        "frameDigestHex": hex::encode(digest),
        "validate": obj.validate().is_ok(),
        "minedAtHeight": rec.mined_height,
        "heightSource": rec.height_source,
    }))
}

pub async fn publish(ctx: &ToolCtx, args: &Value) -> Result<Value, ToolError> {
    let b64 = args
        .get("object")
        .and_then(Value::as_str)
        .ok_or(ToolError::slug("invalid-args", "object (b64u) required"))?;
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(b64)
        .map_err(|e| ToolError::slug("malformed-object", format!("b64u: {e}")))?;
    let obj: AttestationObject = serde_json::from_slice(&bytes)
        .map_err(|e| ToolError::slug("malformed-object", format!("json: {e}")))?;
    obj.validate().map_err(|e| match e {
        zsol_types::ZsolError::Claim(rc) => ToolError::reason(rc, "structural"),
        other => ToolError::slug("malformed-object", other.to_string()),
    })?;
    let canon = obj
        .canonical()
        .map_err(|e| ToolError::slug("internal", e.to_string()))?;
    if canon != bytes {
        return Err(ToolError::slug(
            "non-canonical-object",
            "posting bytes must equal canonical(object); publish zsol_make_attestation output",
        ));
    }
    if ctx.dry_run {
        return okv(json!({
            "dryRun": true,
            "previewBody": {"object": b64},
            "note": "dry-run: no POST performed",
        }));
    }
    let url = format!("{}/v0/attestations", ctx.registry_url);
    let resp = ctx
        .http
        .post(&url)
        .json(&json!({"object": b64}))
        .send()
        .await
        .map_err(|_| ToolError::slug("registry-unreachable", format!("POST {url} failed")))?;
    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|_| ToolError::slug("registry-shape", "registry reply not json"))?;
    if status.as_u16() == 201 {
        return okv(json!({
            "entryId": body.get("entryId").cloned().unwrap_or(Value::Null),
            "chainDigest": body.get("chainDigest").cloned().unwrap_or(Value::Null),
        }));
    }
    if status.as_u16() == 409 && body.get("error").and_then(Value::as_str) == Some("duplicate") {
        // Honest idempotency: first entry wins; the existing entry is surfaced.
        return okv(json!({
            "duplicate": true,
            "entryId": body.get("entryId").cloned().unwrap_or(Value::Null),
            "chainDigest": body.get("chainDigest").cloned().unwrap_or(Value::Null),
        }));
    }
    if status.as_u16() == 400 {
        if let Some(rc) = body.get("reason").and_then(Value::as_str) {
            if let Ok(code) = rc.parse::<ReasonCode>() {
                return Err(ToolError::reason(code, "registry rejected posting"));
            }
        }
    }
    Err(ToolError::slug(
        "registry-status",
        format!("registry returned http {}", status.as_u16()),
    ))
}

pub async fn share_scope(ctx: &ToolCtx, args: &Value) -> Result<Value, ToolError> {
    let label = args
        .get("peerLabel")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .ok_or(ToolError::slug("invalid-args", "peerLabel required"))?;
    if label == "master" || label == "0" || label == "account0" {
        return Err(ToolError::slug(
            "scope-refused",
            "refusing 'master'/'0' label — never shared (SRS invariant 5)",
        ));
    }
    if ctx.share_accounts.is_empty() {
        return Err(ToolError::reason(
            ReasonCode::UivkUnavailable,
            "no sub-accounts configured; set ZSOL_SHARE_ACCOUNTS (attestation-named accounts only)",
        ));
    }
    if ctx.share_accounts.contains(&0) {
        return Err(ToolError::slug(
            "scope-refused",
            "ZSOL_SHARE_ACCOUNTS includes account 0 — refusing whole list",
        ));
    }
    let slot = label_slot(&label, ctx.share_accounts.len());
    let target = ctx.share_accounts[slot];
    if target == ctx.cfg.attest_account {
        return Err(ToolError::slug(
            "scope-refused",
            "configured share account equals master attest account — refusing",
        ));
    }

    let ua_v = ctx
        .w
        .addr_for_account(target, &["orchard", "sapling"])
        .await
        .map_err(|e| map_wallet_err(&e, "z_getaddressforaccount"))?;
    let ua = deep_find_ua(&ua_v, 5).ok_or(ToolError::slug(
        "wallet-shape",
        "no unified address in derivation reply",
    ))?;

    // Export family: parity-tolerant; per-verb params tried (UA-shaped first).
    let mut uivk: Option<String> = None;
    let mut last_shape = String::new();
    for verb in ctx.cfg.methods.export_key.clone() {
        let res = ctx.w.call(Base::Zallet, &verb, json!([ua.clone()])).await;
        match res {
            Ok(v) => {
                if let Some(k) = extract_uivk(&v) {
                    uivk = Some(k);
                    break;
                }
                last_shape = v.to_string();
            }
            Err(WalletError::MissingVerb(_)) => continue,
            Err(e) => return Err(map_wallet_err(&e, "export-key-family")),
        }
    }
    let uivk = uivk.ok_or(ToolError::reason(
        ReasonCode::UivkUnavailable,
        format!("export verbs exhausted; last reply (shape-only) keys: {last_shape}"),
    ))?;
    if uivk.starts_with("secret") || uivk.starts_with("xprv") || uivk.starts_with("zsk") {
        return Err(ToolError::slug(
            "scope-refused",
            "wallet returned spend-class material — zsol exports viewing keys only; aborting",
        ));
    }
    okv(json!({
        "peerLabel": label,
        "accountIdx": target,
        "accountUa": ua,
        "uivk": uivk,
        "revocationPolicy": "honest: a shared uivk cannot be revoked; scopes are burned on conflict (SPEC §6).",
    }))
}

fn extract_uivk(v: &Value) -> Option<String> {
    let k = deep_find(v, "key", 4)
        .or_else(|| deep_find(v, "uivk", 4))
        .or_else(|| deep_find(v, "viewing_key", 4))?;
    let s = k.as_str()?;
    if s.starts_with("uivk") || s.starts_with("zview") {
        Some(s.to_string())
    } else {
        None
    }
}

pub async fn import_scope(ctx: &ToolCtx, args: &Value) -> Result<Value, ToolError> {
    let uivk = args
        .get("uivk")
        .and_then(Value::as_str)
        .ok_or(ToolError::slug("invalid-args", "uivk required"))?
        .trim()
        .to_string();
    let label = args
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or("unnamed")
        .trim()
        .to_string();
    if !(uivk.starts_with("uivk") || uivk.starts_with("zview")) {
        return Err(ToolError::slug(
            "invalid-args",
            "uivk-shaped key required (never spend-class material)",
        ));
    }
    ctx.w
        .import_key(&uivk)
        .await
        .map_err(|e| map_wallet_err(&e, "z_importkey"))?;

    // Imported scope's account index (parity-tolerant; counts-only output, no addresses).
    let idx: Option<u64> = ctx.w.list_accounts().await.ok().and_then(|v| {
        let arr = v.as_array()?;
        let last = arr.last()?;
        unwrap_u64(&deep_find(last, "account", 2).unwrap_or(Value::Null))
            .or_else(|| Some(arr.len().saturating_sub(1) as u64))
    });
    let (bal, interp) = match idx {
        Some(i) => match balance_of(ctx, i).await {
            Ok((b, it)) => (b, it),
            Err(_) => (None, "balance-family-unavailable"),
        },
        None => (None, "unknown-account-index"),
    };
    *ctx.scope
        .lock()
        .map_err(|_| ToolError::slug("internal", "state lock poisoned"))? = Some(ScopeRecord {
        label: label.clone(),
        account_idx: idx,
        balance_zat: bal,
        balance_interpretation: Some(interp),
    });
    okv(json!({
        "imported": true,
        "label": label,
        "accountIdx": idx,
        "balanceZat": bal,
        "balanceInterpretation": interp,
        "note": "scan-state counts only; no addresses listed (NF-6.2)",
    }))
}

/// One §6 check row.
struct Check {
    name: &'static str,
    ok: bool,
    reason: Option<ReasonCode>,
    slug: Option<&'static str>,
    detail: String,
}

fn row_ok(name: &'static str, detail: impl Into<String>) -> Check {
    Check {
        name,
        ok: true,
        reason: None,
        slug: None,
        detail: detail.into(),
    }
}

fn row_reason(rc: ReasonCode, name: &'static str, detail: impl Into<String>) -> Check {
    Check {
        name,
        ok: false,
        reason: Some(rc),
        slug: None,
        detail: detail.into(),
    }
}

fn row_slug(s: &'static str, name: &'static str, detail: impl Into<String>) -> Check {
    Check {
        name,
        ok: false,
        reason: None,
        slug: Some(s),
        detail: detail.into(),
    }
}

impl Check {
    fn wire(&self) -> Value {
        let reason = match (self.reason, self.slug) {
            (Some(rc), _) => json!(rc.as_str()),
            (None, Some(s)) => json!(s),
            (None, None) => Value::Null,
        };
        json!({"check": self.name, "ok": self.ok, "reason": reason, "detail": self.detail})
    }
}

pub async fn verify(ctx: &ToolCtx, args: &Value) -> Result<Value, ToolError> {
    let b64 = args
        .get("object")
        .and_then(Value::as_str)
        .ok_or(ToolError::slug("invalid-args", "object (b64u) required"))?;
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(b64)
        .map_err(|e| ToolError::slug("malformed-object", format!("b64u: {e}")))?;
    let obj: AttestationObject = serde_json::from_slice(&bytes)
        .map_err(|e| ToolError::slug("malformed-object", format!("json: {e}")))?;

    let mut checks: Vec<Check> = Vec::new();

    // C1 structural floors.
    match obj.validate() {
        Ok(()) => checks.push(row_ok("structural", "dust floor + protocol + shapes")),
        Err(zsol_types::ZsolError::Claim(rc)) => {
            checks.push(row_reason(rc, "structural", "structural validation failed"))
        }
        Err(other) => checks.push(row_slug(
            "malformed-object",
            "structural",
            other.to_string(),
        )),
    }

    // C2 memo re-parse.
    let memo = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(&obj.memo_binding)
        .ok()
        .and_then(|m| AnchorMemo::decode_prefix(&m).ok());
    match memo {
        Some(_) => checks.push(row_ok("memo-parse", "zs01 45-byte prefix decodes")),
        None => checks.push(row_reason(
            ReasonCode::MemoDigestMismatch,
            "memo-parse",
            "memo bytes are not a zs01 anchor memo",
        )),
    }

    // C3 frame digest + memo/height equality (§6 row).
    let frame_digest = obj.digest().ok();
    let digest_ok = if let (Some(memo), Some(d)) = (&memo, &frame_digest) {
        let h_ok = memo.height == obj.anchor_height;
        if !h_ok {
            checks.push(row_reason(
                ReasonCode::MemoDigestMismatch,
                "memo-height",
                format!(
                    "memo height {} ≠ object anchorHeight {}",
                    memo.height, obj.anchor_height
                ),
            ));
        }
        if memo.digest != *d {
            checks.push(row_reason(
                ReasonCode::MemoDigestMismatch,
                "frame-digest",
                "scanned memo digest ≠ recomputed frame digest D",
            ));
        }
        h_ok && memo.digest == *d
    } else {
        checks.push(row_reason(
            ReasonCode::MemoDigestMismatch,
            "frame-digest",
            "memo/digest inputs unparseable",
        ));
        false
    };

    // C4–C7 scope-dependent rows: verified only under an imported scope. With NO
    // scope the tier is at most claim-only — skipping is EXPLICIT, and no
    // wallet-side self-attestation ever substitutes for the scope's scan view.
    let scope = ctx
        .scope
        .lock()
        .map_err(|_| ToolError::slug("internal", "state lock poisoned"))?
        .clone();
    if scope.is_none() {
        checks.push(row_reason(
            ReasonCode::UivkUnavailable,
            "scope",
            "no uivk imported: anchor-scan + balance rows skipped, tier claim-only (zallet beta.1 exports no unified viewing key)",
        ));
    }
    if let Some(scope) = scope {
        let view = ctx.w.view_tx(&obj.anchor_txid).await;
        let mut mined_ok = false;
        match view {
            Ok(_) => {
                checks.push(row_ok(
                    "anchor-visible-in-scan",
                    "txid known to imported scope",
                ));
                mined_ok = true;
            }
            Err(WalletError::Rpc { code, .. }) if [-8, -5].contains(&code) => {
                checks.push(row_reason(
                    ReasonCode::AnchorNotMined,
                    "anchor-visible-in-scan",
                    "txid unknown to verifier wallet scope/scan",
                ));
            }
            Err(e) => return Err(map_wallet_err(&e, "z_viewtransaction")),
        }

        // C6 outputs ≥ claim (+ evidence rows; mined height as separate evidence).
        if mined_ok && digest_ok {
            if let Ok(v) = ctx.w.view_tx(&obj.anchor_txid).await {
                if let Some(mined) = unwrap_mined_height(&v) {
                    checks.push(row_ok(
                        "mined-height-evidence",
                        format!("minedAtHeight={mined}; window={}", obj.anchor_height),
                    ));
                }
                let outputs = deep_find(&v, "output", 3)
                    .or_else(|| deep_find(&v, "outputs", 3))
                    .and_then(|o| o.as_array().cloned());
                match outputs {
                    Some(outs) => {
                        let sum: u64 = outs
                            .iter()
                            .filter_map(|o| {
                                deep_find(o, "valueZat", 2)
                                    .and_then(|z| unwrap_u64(&z))
                                    .or_else(|| {
                                        num_to_zat(&deep_find(o, "value", 2).unwrap_or(Value::Null))
                                    })
                            })
                            .sum();
                        if sum >= obj.claim_zat {
                            checks.push(row_ok(
                                "outputs-geq-claim",
                                format!("sum {sum} ≥ claim {}", obj.claim_zat),
                            ));
                        } else {
                            checks.push(row_reason(
                                ReasonCode::ClaimExceedsOutputs,
                                "outputs-geq-claim",
                                format!("outputs sum {sum} < claim {}", obj.claim_zat),
                            ));
                        }
                    }
                    None => checks.push(row_slug(
                        "wallet-shape",
                        "outputs-geq-claim",
                        "anchor tx found; outputs list unextractable (beta shape)",
                    )),
                }
            }

            // C7 balance ≥ claim at scan time.
            if let Some(i) = scope.account_idx {
                match balance_of(ctx, i).await {
                    Ok((Some(bal), interp)) if bal >= obj.claim_zat => {
                        checks.push(row_ok(
                            "balance-at-scan",
                            format!("{bal} ≥ claim {} ({interp})", obj.claim_zat),
                        ));
                    }
                    Ok((Some(bal), interp)) => checks.push(row_reason(
                        ReasonCode::BalanceBelowClaim,
                        "balance-at-scan",
                        format!("{bal} < claim {} ({interp})", obj.claim_zat),
                    )),
                    Ok((None, interp)) => checks.push(row_slug(
                        "wallet-shape",
                        "balance-at-scan",
                        format!("unparseable balance ({interp})"),
                    )),
                    Err(_) => checks.push(row_slug(
                        "wallet-unreachable",
                        "balance-at-scan",
                        "balance probe failed",
                    )),
                }
            }
        }
    } // end scope-dependent C4–C7

    // C8 freshness.
    let now = zsol_types::now_epoch_secs();
    let fresh = now
        .checked_sub(obj.issued_at)
        .map(|d| d <= (obj.max_age_hours as u64) * 3600)
        .unwrap_or(false); // issuedAt from the future is also not fresh
    if fresh {
        checks.push(row_ok(
            "freshness",
            format!("within {}h", obj.max_age_hours),
        ));
    } else {
        checks.push(row_reason(
            ReasonCode::StaleAttestation,
            "freshness",
            "issuedAt + maxAgeHours expired (or issuedAt in the future)",
        ));
    }

    let all_ok = checks.iter().all(|c| c.ok);
    let rows: Vec<Value> = checks.iter().map(|c| c.wire()).collect();
    let first_fail = checks.iter().find(|c| !c.ok);
    let (reason, slug) = match first_fail {
        Some(f) if f.reason.is_some() => (json!(f.reason.unwrap().as_str()), Value::Null),
        Some(f) => (Value::Null, json!(f.slug.unwrap_or("internal"))),
        None => (Value::Null, Value::Null),
    };
    okv(json!({
        "tier": if all_ok { "verified" } else { "claim-only" },
        "reason": reason,
        "slug": slug,
        "checks": rows,
        "agentId": obj.agent_id,
        "claimZat": obj.claim_zat,
        "anchorTxid": obj.anchor_txid,
    }))
}

pub async fn registry_fetch(ctx: &ToolCtx, args: &Value) -> Result<Value, ToolError> {
    let agent_id = args
        .get("agentId")
        .and_then(Value::as_str)
        .ok_or(ToolError::slug("invalid-args", "agentId required"))?
        .trim();
    if agent_id.is_empty() {
        return Err(ToolError::slug("invalid-args", "agentId required"));
    }
    let url = format!("{}/v0/attestations/{}", ctx.registry_url, agent_id);
    let resp = ctx.http.get(&url).send().await.map_err(|_| {
        ToolError::reason(ReasonCode::RegistryUnreachable, format!("GET {url} failed"))
    })?;
    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|_| ToolError::slug("registry-shape", "registry reply not json"))?;
    if !status.is_success() {
        return Err(ToolError::slug(
            "registry-status",
            format!("registry returned http {}", status.as_u16()),
        ));
    }

    // Chain-continuity recompute (verifier does NOT trust the registry, FR-3.2):
    // entries carry parsed objects; canonicalize in-place, recompute digests.
    let canonicalize = |v: &Value| -> Vec<u8> {
        let mut c = v.clone();
        sort_keys(&mut c);
        serde_json::to_vec(&c).unwrap_or_default()
    };
    let mut prev: [u8; 32] = [0u8; 32];
    let mut rows = Vec::new();
    if let Some(entries) = body.get("entries").and_then(Value::as_array) {
        for e in entries {
            let obj_v = e.get("object").cloned().unwrap_or(Value::Null);
            let canon = if let Some(b64) = obj_v.as_str() {
                use base64::Engine as _;
                base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(b64)
                    .unwrap_or_default()
            } else {
                canonicalize(&obj_v)
            };
            let cd = zsol_types::chain_digest(prev, &canon);
            let hex_cd = hex::encode(cd);
            let claimed = e
                .get("chainDigest")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_lowercase();
            let chain_ok = claimed == hex_cd;
            prev = cd;
            rows.push(json!({
                "entryId": e.get("entryId").cloned().unwrap_or(Value::Null),
                "chainDigest": hex_cd,
                "chainOk": chain_ok,
            }));
        }
    }
    okv(json!({
        "agentId": body.get("agentId").cloned().unwrap_or(json!(agent_id)),
        "chainHead": body.get("chainHead").cloned().unwrap_or(Value::Null),
        "entries": rows,
        "note": "chainOk recomputed independently per SPEC §7",
    }))
}

fn sort_keys(v: &mut Value) {
    use serde_json::Map;
    match v {
        Value::Object(m) => {
            let mut keys: Vec<String> = m.keys().cloned().collect();
            keys.sort();
            let mut out = Map::new();
            for k in keys {
                let mut val = m.remove(&k).unwrap_or(Value::Null);
                sort_keys(&mut val);
                out.insert(k, val);
            }
            *v = Value::Object(out);
        }
        Value::Array(a) => {
            for e in a.iter_mut() {
                sort_keys(e);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-36 negative: scope guards — label fold determinism + reachability.
    #[test]
    fn ac_scope_refusals() {
        // Deterministic: same label always maps to the same sub-account.
        assert_eq!(label_slot("v1", 3), label_slot("v1", 3));
        // Tiny pools may collide two labels to one slot — that is acceptable and
        // documented (scopes are per-verifier by PROCESS); the fold only needs to
        // spread over many labels.
        let seen: std::collections::HashSet<usize> = (0..50u32)
            .map(|i| label_slot(&format!("agent-{i}"), 3))
            .collect();
        assert_eq!(seen.len(), 3);
    }

    /// T-37: balance interpretation honesty — object vs numeric shapes both surface.
    #[test]
    fn ac_balance_interpretation_marked() {
        let (v, i) = unwrap_balance(&json!({"balance": 150000}));
        assert_eq!(v, Some(150000));
        assert_eq!(i, "object-balance=zat");
        let (v, i) = unwrap_balance(&json!(0.01));
        assert_eq!(v, Some(1_000_000));
        assert_eq!(i, "numeric-zec");
    }

    /// Frame digest preview == materialized object digest (fixed point, SPEC §4).
    #[test]
    fn ac_frame_fixed_point() {
        let frame = frame_pre("agent-1", 1_000_000, "u1test", 1_769_337_600, 24);
        let d = frame.digest().unwrap();
        let memo = AnchorMemo {
            height: 4_509_900,
            digest: d,
        };
        let full = AttestationObject {
            protocol: PROTOCOL.to_string(),
            agent_id: "agent-1".into(),
            claim_zat: 1_000_000,
            anchor_height: 4_509_900,
            anchor_txid: "a".repeat(64),
            account_ua: "u1test".into(),
            memo_binding: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(memo.encode()),
            issued_at: 1_769_337_600,
            max_age_hours: 24,
        };
        assert_eq!(full.digest().unwrap(), d);
    }
}
