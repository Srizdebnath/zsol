//! zsol MCP tools (SRS FR-5.1, FR-6.1) over the light-client backend. Every tool acts on the
//! real chain through the configured lightwalletd endpoint; nothing is simulated.
//!
//! Config (env):
//!   ZSOL_WALLET_DIR      this agent's wallet directory (required)
//!   ZSOL_LWD             lightwalletd host:port (default testnet.zec.rocks:443)
//!   ZSOL_NETWORK         test | main (default test)
//!   ZSOL_REGISTRY_URL    claims registry base URL (optional)
//!   ZSOL_CREDIT_URL      Credit Desk base URL (optional)
//!   ZSOL_VERIFIER_ID     identity used when this agent issues challenges (default: wallet dir name)
//!   ZSOL_ALLOW_SPEND     must be "1" for any tool that broadcasts (FR-5.2)
//!   ZSOL_SPEND_CAP_ZAT   per-call ceiling for broadcasting tools (default 10_000_000 = 0.1 TAZ)
//!   ZSOL_MIN_CONF        confirmations for spendability and verification (default 1)

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::Mutex;
use zsol_lightwallet::{Wallet, attest, chain};
use zsol_types::{AttestationObject, Challenge, ReasonCode, b64u_encode, now_epoch_secs};
use zsol_verifier::{Presentation, VerifierConfig};

pub struct ToolCtx {
    pub http: reqwest::Client,
    pub wallet_dir: PathBuf,
    pub lwd: String,
    pub network: String,
    pub registry_url: Option<String>,
    pub credit_url: Option<String>,
    pub verifier_id: String,
    pub allow_spend: bool,
    pub spend_cap_zat: u64,
    pub min_conf: u32,
    session: Mutex<Option<(Wallet, chain::Client)>>,
}

#[derive(Debug)]
pub struct ToolError {
    pub reason: Option<ReasonCode>,
    pub slug: Option<&'static str>,
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
        match (self.reason, self.slug) {
            (Some(rc), _) => json!(rc.as_str()),
            (None, Some(s)) => json!(s),
            (None, None) => json!("internal"),
        }
    }
}

impl From<anyhow::Error> for ToolError {
    fn from(e: anyhow::Error) -> Self {
        ToolError::slug("failed", format!("{e:#}"))
    }
}

fn env(k: &str) -> Option<String> {
    std::env::var(k)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

impl ToolCtx {
    pub fn from_env() -> Result<Arc<Self>, String> {
        let wallet_dir =
            PathBuf::from(env("ZSOL_WALLET_DIR").ok_or("ZSOL_WALLET_DIR is required")?);
        let verifier_id = env("ZSOL_VERIFIER_ID").unwrap_or_else(|| {
            wallet_dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("verifier")
                .to_string()
        });
        Ok(Arc::new(Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(600))
                .build()
                .map_err(|e| e.to_string())?,
            wallet_dir,
            lwd: env("ZSOL_LWD").unwrap_or_else(|| chain::DEFAULT_TESTNET_ENDPOINT.into()),
            network: env("ZSOL_NETWORK").unwrap_or_else(|| "test".into()),
            registry_url: env("ZSOL_REGISTRY_URL").map(|s| s.trim_end_matches('/').to_string()),
            credit_url: env("ZSOL_CREDIT_URL").map(|s| s.trim_end_matches('/').to_string()),
            verifier_id,
            allow_spend: env("ZSOL_ALLOW_SPEND").as_deref() == Some("1"),
            spend_cap_zat: env("ZSOL_SPEND_CAP_ZAT")
                .and_then(|s| s.parse().ok())
                .unwrap_or(10_000_000),
            min_conf: env("ZSOL_MIN_CONF")
                .and_then(|s| s.parse().ok())
                .unwrap_or(1),
            session: Mutex::new(None),
        }))
    }

    /// Opens the wallet and a boot-checked chain connection on first use.
    async fn session(
        &self,
    ) -> Result<tokio::sync::MutexGuard<'_, Option<(Wallet, chain::Client)>>, ToolError> {
        let mut g = self.session.lock().await;
        if g.is_none() {
            let params = chain::parse_network(&self.network)?;
            let mut c = chain::connect(&self.lwd)
                .await
                .map_err(|e| ToolError::reason(ReasonCode::ChainUnreachable, format!("{e:#}")))?;
            chain::boot_check(&mut c, &params, &self.lwd)
                .await
                .map_err(|e| ToolError::reason(ReasonCode::ChainUnreachable, format!("{e:#}")))?;
            let w = Wallet::open(&self.wallet_dir)?;
            *g = Some((w, c));
        }
        Ok(g)
    }

    fn spend_gate(&self, zat: u64) -> Result<(), ToolError> {
        if !self.allow_spend {
            return Err(ToolError::slug(
                "spend-disabled",
                "this gateway runs without ZSOL_ALLOW_SPEND=1; no transaction was built",
            ));
        }
        if zat > self.spend_cap_zat {
            return Err(ToolError::slug(
                "spend-cap",
                format!(
                    "{zat} zat exceeds ZSOL_SPEND_CAP_ZAT={}",
                    self.spend_cap_zat
                ),
            ));
        }
        Ok(())
    }
}

fn arg_str<'a>(args: &'a Value, k: &str) -> Option<&'a str> {
    args.get(k)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn arg_u64(args: &Value, k: &str) -> Option<u64> {
    args.get(k).and_then(Value::as_u64)
}

/// The object named by `args.object` (b64u), or this wallet's newest mined attestation.
fn pick_object(w: &Wallet, args: &Value) -> Result<AttestationObject, ToolError> {
    if let Some(b) = arg_str(args, "object") {
        return AttestationObject::from_b64u(b)
            .map_err(|e| ToolError::slug("malformed-object", e.to_string()));
    }
    attest::list_attestations(w)?
        .into_iter()
        .find(|o| o.anchor_height != 0)
        .ok_or_else(|| {
            ToolError::slug(
                "no-attestation",
                "this wallet has no mined attestation yet; call zsol_anchor first",
            )
        })
}

pub async fn health(ctx: &ToolCtx) -> Result<Value, ToolError> {
    let params = chain::parse_network(&ctx.network)?;
    let chain_info = match chain::connect(&ctx.lwd).await {
        Ok(mut c) => chain::boot_check(&mut c, &params, &ctx.lwd)
            .await
            .map_err(|e| format!("{e:#}")),
        Err(e) => Err(format!("{e:#}")),
    };
    let probe = |url: Option<String>| {
        let http = ctx.http.clone();
        async move {
            match url {
                None => json!("not configured"),
                Some(u) => match http
                    .get(format!("{u}/v0/health"))
                    .timeout(Duration::from_secs(10))
                    .send()
                    .await
                {
                    Ok(r) => json!({"url": u, "status": r.status().as_u16()}),
                    Err(e) => json!({"url": u, "error": e.to_string()}),
                },
            }
        }
    };
    Ok(json!({
        "chain": chain_info,
        "wallet": {"dir": ctx.wallet_dir, "exists": Wallet::exists(&ctx.wallet_dir)},
        "registry": probe(ctx.registry_url.clone()).await,
        "creditDesk": probe(ctx.credit_url.clone()).await,
        "spend": {"allowed": ctx.allow_spend, "capZat": ctx.spend_cap_zat},
    }))
}

pub async fn wallet_status(ctx: &ToolCtx) -> Result<Value, ToolError> {
    let mut g = ctx.session().await?;
    let (w, c) = g.as_mut().expect("session");
    let sync = w.sync(c).await?;
    let balances = w.balances(ctx.min_conf)?;
    let mut addresses = serde_json::Map::new();
    for name in w.meta.accounts.keys().cloned().collect::<Vec<_>>() {
        if w.meta.kind == zsol_lightwallet::WalletKind::Spending && !name.starts_with("peer-") {
            addresses.insert(name.clone(), json!(w.address(&name, false)?));
        }
    }
    Ok(json!({"sync": sync, "balances": balances, "receiveAddresses": addresses}))
}

pub async fn anchor(ctx: &ToolCtx, args: &Value) -> Result<Value, ToolError> {
    let agent_id =
        arg_str(args, "agentId").ok_or(ToolError::slug("invalid-args", "agentId required"))?;
    let claim =
        arg_u64(args, "claimZat").ok_or(ToolError::slug("invalid-args", "claimZat required"))?;
    ctx.spend_gate(claim)?;
    let mut g = ctx.session().await?;
    let (w, c) = g.as_mut().expect("session");
    let out = attest::anchor(
        w,
        c,
        &attest::AnchorRequest {
            agent_id: agent_id.to_string(),
            claim_zat: claim,
            max_age_hours: arg_u64(args, "maxAgeHours").unwrap_or(24) as u32,
            min_conf: ctx.min_conf,
            mine_timeout: Duration::from_secs(900),
        },
    )
    .await?;
    Ok(serde_json::to_value(out).map_err(anyhow::Error::from)?)
}

pub async fn attestations(ctx: &ToolCtx) -> Result<Value, ToolError> {
    let mut g = ctx.session().await?;
    let (w, c) = g.as_mut().expect("session");
    let finalized = attest::finalize_pending(w, c).await?;
    let list: Vec<Value> = attest::list_attestations(w)?
        .into_iter()
        .map(|o| json!({"objectB64u": o.b64u().ok(), "mined": o.anchor_height != 0, "object": o}))
        .collect();
    Ok(json!({"finalizedNow": finalized, "attestations": list}))
}

pub async fn publish(ctx: &ToolCtx, args: &Value) -> Result<Value, ToolError> {
    let base = ctx.registry_url.clone().ok_or(ToolError::reason(
        ReasonCode::RegistryUnreachable,
        "ZSOL_REGISTRY_URL not set",
    ))?;
    let o = {
        let mut g = ctx.session().await?;
        let (w, _) = g.as_mut().expect("session");
        pick_object(w, args)?
    };
    let b64 = o
        .b64u()
        .map_err(|e| ToolError::slug("malformed-object", e.to_string()))?;
    let resp = ctx
        .http
        .post(format!("{base}/v0/attestations"))
        .json(&json!({"object": b64}))
        .send()
        .await
        .map_err(|e| ToolError::reason(ReasonCode::RegistryUnreachable, e.to_string()))?;
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    match status {
        201 => Ok(json!({"entryId": body["entryId"], "chainDigest": body["chainDigest"]})),
        409 => Ok(
            json!({"duplicate": true, "entryId": body["entryId"], "chainDigest": body["chainDigest"]}),
        ),
        _ => Err(ToolError::slug(
            "registry-rejected",
            format!("HTTP {status}: {body}"),
        )),
    }
}

pub async fn present(ctx: &ToolCtx, args: &Value) -> Result<Value, ToolError> {
    let ch: Challenge =
        serde_json::from_value(args.get("challenge").cloned().unwrap_or(Value::Null))
            .map_err(|e| ToolError::slug("invalid-args", format!("challenge: {e}")))?;
    let mut g = ctx.session().await?;
    let (w, _) = g.as_mut().expect("session");
    let o = pick_object(w, args)?;
    let p = Presentation {
        signature: attest::sign_challenge(w, &o, &ch)?,
        ufvk: w.export_ufvk(attest::ATTEST_ACCOUNT)?,
        object: o,
        challenge: ch,
    };
    Ok(json!({
        "presentation": p,
        "disclosure": "The UFVK reveals the full history of the zsol-attn account only (SPEC §3). Send it over a direct channel, never to the registry.",
    }))
}

pub async fn challenge(ctx: &ToolCtx) -> Result<Value, ToolError> {
    let mut nonce = [0u8; 32];
    getrandom::fill(&mut nonce).map_err(|e| ToolError::slug("rng", e.to_string()))?;
    Ok(serde_json::to_value(Challenge {
        nonce: b64u_encode(&nonce),
        verifier_id: ctx.verifier_id.clone(),
        expires_at: now_epoch_secs() + 600,
    })
    .map_err(anyhow::Error::from)?)
}

pub async fn verify(ctx: &ToolCtx, args: &Value) -> Result<Value, ToolError> {
    let p: Presentation =
        serde_json::from_value(args.get("presentation").cloned().unwrap_or(Value::Null))
            .map_err(|e| ToolError::slug("invalid-args", format!("presentation: {e}")))?;
    let mut g = ctx.session().await?;
    let (w, c) = g.as_mut().expect("session");
    let cfg = VerifierConfig {
        network: ctx.network.clone(),
        verifier_id: ctx.verifier_id.clone(),
        min_conf: ctx.min_conf,
    };
    let report = zsol_verifier::verify(w, c, &cfg, &p).await?;
    Ok(serde_json::to_value(report).map_err(anyhow::Error::from)?)
}

pub async fn watch(ctx: &ToolCtx, args: &Value) -> Result<Value, ToolError> {
    let mut g = ctx.session().await?;
    let (w, c) = g.as_mut().expect("session");
    let o = pick_object(w, args)?;
    Ok(
        serde_json::to_value(zsol_verifier::watch_tick(w, c, &o).await?)
            .map_err(anyhow::Error::from)?,
    )
}

pub async fn credit_request(ctx: &ToolCtx, args: &Value) -> Result<Value, ToolError> {
    let base = ctx.credit_url.clone().ok_or(ToolError::slug(
        "credit-unconfigured",
        "ZSOL_CREDIT_URL not set",
    ))?;
    let product =
        arg_str(args, "productId").ok_or(ToolError::slug("invalid-args", "productId required"))?;
    let value_usd =
        arg_u64(args, "valueUsd").ok_or(ToolError::slug("invalid-args", "valueUsd required"))?;
    let ch: Challenge = ctx
        .http
        .post(format!("{base}/v0/credit/challenge"))
        .send()
        .await
        .map_err(|e| ToolError::slug("credit-unreachable", e.to_string()))?
        .json()
        .await
        .map_err(|e| ToolError::slug("credit-shape", e.to_string()))?;
    let presentation = {
        let mut g = ctx.session().await?;
        let (w, _) = g.as_mut().expect("session");
        let o = pick_object(w, args)?;
        Presentation {
            signature: attest::sign_challenge(w, &o, &ch)?,
            ufvk: w.export_ufvk(attest::ATTEST_ACCOUNT)?,
            object: o,
            challenge: ch,
        }
    };
    let resp = ctx
        .http
        .post(format!("{base}/v0/credit/open"))
        .json(&json!({"presentation": presentation, "productId": product, "valueUsd": value_usd}))
        .send()
        .await
        .map_err(|e| ToolError::slug("credit-unreachable", e.to_string()))?;
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if status == 200 {
        Ok(body)
    } else {
        Err(ToolError::slug(
            "credit-refused",
            format!("HTTP {status}: {body}"),
        ))
    }
}

pub async fn credit_repay(ctx: &ToolCtx, args: &Value) -> Result<Value, ToolError> {
    let base = ctx.credit_url.clone().ok_or(ToolError::slug(
        "credit-unconfigured",
        "ZSOL_CREDIT_URL not set",
    ))?;
    let loan_id =
        arg_str(args, "loanId").ok_or(ToolError::slug("invalid-args", "loanId required"))?;
    let loan: Value = ctx
        .http
        .get(format!("{base}/v0/credit/loans/{loan_id}"))
        .send()
        .await
        .map_err(|e| ToolError::slug("credit-unreachable", e.to_string()))?
        .json()
        .await
        .map_err(|e| ToolError::slug("credit-shape", e.to_string()))?;
    let zat = loan["principalZat"]
        .as_u64()
        .ok_or(ToolError::slug("credit-shape", "loan has no principalZat"))?;
    let to = loan["repayTo"]
        .as_str()
        .ok_or(ToolError::slug("credit-shape", "loan has no repayTo"))?
        .to_string();
    let memo = hex::decode(loan["memoHex"].as_str().unwrap_or(""))
        .map_err(|_| ToolError::slug("credit-shape", "memoHex"))?;
    if zsol_types::LoanMemo::decode(&memo).map(|m| hex::encode(m.loan_id))
        != Ok(loan_id.to_string())
    {
        return Err(ToolError::slug(
            "credit-shape",
            "desk memo does not encode this loanId",
        ));
    }
    ctx.spend_gate(zat)?;
    let mut g = ctx.session().await?;
    let (w, c) = g.as_mut().expect("session");
    w.sync(c).await?;
    let txid = w
        .pay(c, "commerce", &to, zat, Some(&memo), ctx.min_conf)
        .await?;
    Ok(json!({"txid": txid, "paidZat": zat, "from": "commerce", "loanId": loan_id}))
}

/// Ends this wallet's attestations by moving the zsol-attn balance to commerce (SPEC §8:
/// withdrawal is the attestor's revocation primitive; every watching verifier sees it).
pub async fn release(ctx: &ToolCtx) -> Result<Value, ToolError> {
    let mut g = ctx.session().await?;
    let (w, c) = g.as_mut().expect("session");
    w.sync(c).await?;
    let bal = w
        .balances(ctx.min_conf)?
        .into_iter()
        .find(|b| b.account == attest::ATTEST_ACCOUNT)
        .ok_or(ToolError::slug("no-account", "no zsol-attn account"))?;
    let amount = bal.spendable_zat.saturating_sub(20_000);
    if amount == 0 {
        return Err(ToolError::slug(
            "nothing-to-release",
            format!("spendable {} zat", bal.spendable_zat),
        ));
    }
    ctx.spend_gate(amount)?;
    let to = w.address("commerce", false)?;
    let txid = w
        .pay(c, attest::ATTEST_ACCOUNT, &to, amount, None, ctx.min_conf)
        .await?;
    Ok(
        json!({"txid": txid, "movedZat": amount, "note": "every verifier watching this attestation will see it released within one block"}),
    )
}

pub async fn registry_fetch(ctx: &ToolCtx, args: &Value) -> Result<Value, ToolError> {
    let base = ctx.registry_url.clone().ok_or(ToolError::reason(
        ReasonCode::RegistryUnreachable,
        "ZSOL_REGISTRY_URL not set",
    ))?;
    let agent_id =
        arg_str(args, "agentId").ok_or(ToolError::slug("invalid-args", "agentId required"))?;
    let resp = ctx
        .http
        .get(format!("{base}/v0/attestations/{agent_id}"))
        .send()
        .await
        .map_err(|e| ToolError::reason(ReasonCode::RegistryUnreachable, e.to_string()))?;
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
    // The registry chain is GLOBAL (one chain_head across all agents): each entry's prev is the
    // digest of whatever was inserted just before it, exposed as prevDigest (null = genesis).
    // We recompute every digest ourselves and never trust the registry's value (SPEC §9, P5).
    let mut rows = Vec::new();
    for e in body
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let obj: Option<AttestationObject> = serde_json::from_value(e["object"].clone()).ok();
        let canon = obj.as_ref().and_then(|o| o.canonical().ok());
        let prev = match e.get("prevDigest").and_then(Value::as_str) {
            None => Some([0u8; 32]),
            Some(h) => hex::decode(h)
                .ok()
                .and_then(|b| <[u8; 32]>::try_from(b).ok()),
        };
        let (chain_ok, recomputed) = match (prev, canon) {
            (Some(p), Some(c)) => {
                let d = hex::encode(zsol_types::chain_digest(p, &c));
                (
                    e["chainDigest"].as_str().map(str::to_lowercase) == Some(d.clone()),
                    Some(d),
                )
            }
            _ => (false, None),
        };
        rows.push(json!({
            "entryId": e["entryId"],
            "chainDigest": recomputed,
            "chainOk": chain_ok,
            "objectB64u": obj.and_then(|o| o.b64u().ok()),
        }));
    }
    Ok(
        json!({"agentId": agent_id, "entries": rows, "note": "chainOk recomputed locally per SPEC §9"}),
    )
}
