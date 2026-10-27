//! zsol Credit Desk — underwriter service (SPEC §11, SRS FR-6). Verifies borrowers' live
//! attestations in its own wallet, buys through Bitrefill, detects shielded repayments, and
//! freezes credit the block an attestation is released.

mod bitrefill;
mod db;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, anyhow};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event as SseEvent, KeepAlive, Sse},
    },
    routing::{get, post},
};
use futures_util::stream::Stream;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{Mutex, broadcast};
use tokio_stream::{StreamExt as _, wrappers::BroadcastStream};
use zsol_lightwallet::{Wallet, chain};
use zsol_types::{Challenge, LoanMemo, b64u_encode, now_epoch_secs};
use zsol_verifier::{Presentation, VerifierConfig, WatchState};

use crate::bitrefill::Bitrefill;

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Policy {
    version: String,
    underwriter_id: String,
    mode: String,
    ltv_bps: u64,
    max_loan_usd: f64,
    max_tenor_blocks: u32,
    min_conf: u32,
    rate_max_age_secs: u64,
    allowed_products: Vec<String>,
}

struct Inner {
    wallet: Wallet,
    client: chain::Client,
    book: rusqlite::Connection,
}

struct AppState {
    inner: Mutex<Inner>,
    policy: Policy,
    network: String,
    lwd: String,
    bitrefill: Bitrefill,
    http: reqwest::Client,
    events: broadcast::Sender<db::Event>,
}

type Shared = Arc<AppState>;

struct ApiError(StatusCode, Value);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(self.1)).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        tracing::warn!(error = %e, "request failed");
        ApiError(
            StatusCode::BAD_GATEWAY,
            json!({"error": "internal", "detail": e.to_string()}),
        )
    }
}

fn reject(code: StatusCode, reason: &str, detail: impl Into<Value>) -> ApiError {
    ApiError(code, json!({"error": reason, "detail": detail.into()}))
}

fn emit(
    st: &AppState,
    inner: &Inner,
    loan: Option<&str>,
    kind: &str,
    height: Option<u32>,
    detail: Value,
) {
    match db::add_event(&inner.book, loan, kind, height, &detail, now_epoch_secs()) {
        Ok(e) => {
            tracing::info!(kind, loan = loan.unwrap_or("-"), "event");
            let _ = st.events.send(e);
        }
        Err(e) => tracing::error!(error = %e, "event persist failed"),
    }
}

/// Live ZEC/USD reference rate: CoinGecko, falling back to Kraken. Returned with its source.
async fn reference_rate(
    http: &reqwest::Client,
    max_age: u64,
) -> anyhow::Result<(f64, String, u64)> {
    let cg = async {
        let v: Value = http
            .get("https://api.coingecko.com/api/v3/simple/price?ids=zcash&vs_currencies=usd&include_last_updated_at=true")
            .send()
            .await?
            .json()
            .await?;
        let px = v["zcash"]["usd"].as_f64().ok_or_else(|| anyhow!("coingecko: no price"))?;
        let at = v["zcash"]["last_updated_at"].as_u64().unwrap_or(now_epoch_secs());
        anyhow::Ok((px, "coingecko:zcash/usd".to_string(), at))
    }
    .await;
    let r = match cg {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "coingecko failed; trying kraken");
            let v: Value = http
                .get("https://api.kraken.com/0/public/Ticker?pair=ZECUSD")
                .send()
                .await?
                .json()
                .await?;
            let px: f64 = v["result"]["XZECZUSD"]["c"][0]
                .as_str()
                .ok_or_else(|| anyhow!("kraken: no price"))?
                .parse()?;
            (px, "kraken:XZECZUSD/last".to_string(), now_epoch_secs())
        }
    };
    if now_epoch_secs().saturating_sub(r.2) > max_age || !(r.0.is_finite() && r.0 > 0.0) {
        anyhow::bail!("reference rate stale or invalid: {r:?}");
    }
    Ok(r)
}

async fn health(State(st): State<Shared>) -> Result<Json<Value>, ApiError> {
    let mut inner = st.inner.lock().await;
    let params = chain::parse_network(&st.network)?;
    let lwd = st.lwd.clone();
    let chain_info = chain::boot_check(&mut inner.client, &params, &lwd).await;
    let balances = inner.wallet.balances(st.policy.min_conf)?;
    let bitrefill_balance = st.bitrefill.balance().await.map_err(|e| e.to_string());
    let bitrefill = bitrefill_balance.is_ok();
    Ok(Json(json!({
        "ok": chain_info.is_ok() && bitrefill,
        "bitrefillBalance": bitrefill_balance,
        "chain": chain_info.map_err(|e| e.to_string()),
        "treasury": balances.into_iter().find(|b| b.account == "treasury"),
        "bitrefill": bitrefill,
        "policy": st.policy,
    })))
}

async fn new_challenge(State(st): State<Shared>) -> Result<Json<Challenge>, ApiError> {
    let mut nonce = [0u8; 32];
    getrandom::fill(&mut nonce).map_err(|e| anyhow!("rng: {e}"))?;
    let ch = Challenge {
        nonce: b64u_encode(&nonce),
        verifier_id: st.policy.underwriter_id.clone(),
        expires_at: now_epoch_secs() + 600,
    };
    let inner = st.inner.lock().await;
    db::store_challenge(&inner.book, &ch.nonce, ch.expires_at)?;
    Ok(Json(ch))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenReq {
    presentation: Presentation,
    product_id: String,
    value_usd: u64,
}

async fn open_credit(
    State(st): State<Shared>,
    Json(req): Json<OpenReq>,
) -> Result<Json<Value>, ApiError> {
    let p = &req.presentation;
    if !st.policy.allowed_products.contains(&req.product_id) {
        return Err(reject(
            StatusCode::UNPROCESSABLE_ENTITY,
            "product-not-allowed",
            req.product_id.clone(),
        ));
    }
    if req.value_usd == 0 || req.value_usd as f64 > st.policy.max_loan_usd {
        return Err(reject(
            StatusCode::UNPROCESSABLE_ENTITY,
            "amount-out-of-policy",
            json!({"maxLoanUsd": st.policy.max_loan_usd}),
        ));
    }
    // Price checks first: a value Bitrefill does not sell must not burn the borrower's challenge.
    let product = st.bitrefill.product(&req.product_id).await?;
    if product["currency"].as_str() != Some("USD") {
        return Err(reject(
            StatusCode::UNPROCESSABLE_ENTITY,
            "product-currency",
            json!({"currency": product["currency"]}),
        ));
    }
    if !bitrefill::offers_value(&product, req.value_usd) {
        return Err(reject(
            StatusCode::UNPROCESSABLE_ENTITY,
            "value-not-offered",
            json!({"range": product["range"], "packages": product["packages"]}),
        ));
    }
    let mut guard = st.inner.lock().await;
    let inner = &mut *guard;
    if !db::consume_challenge(&inner.book, &p.challenge.nonce, now_epoch_secs())? {
        return Err(reject(
            StatusCode::UNPROCESSABLE_ENTITY,
            "presentation-invalid",
            "challenge unknown, expired or already used",
        ));
    }
    if db::agent_frozen(&inner.book, &p.object.agent_id)? {
        return Err(reject(
            StatusCode::FORBIDDEN,
            "line-frozen",
            "agent has an at-risk or defaulted loan",
        ));
    }
    let cfg = VerifierConfig {
        network: st.network.clone(),
        verifier_id: st.policy.underwriter_id.clone(),
        min_conf: st.policy.min_conf,
    };
    let report = zsol_verifier::verify(&mut inner.wallet, &mut inner.client, &cfg, p).await?;
    emit(
        &st,
        inner,
        None,
        "verification",
        Some(report.tip_height),
        json!({
            "agentId": report.agent_id, "tier": report.tier, "reasons": report.reasons,
            "anchorTxid": report.anchor_txid, "anchoredZat": report.anchored_zat,
        }),
    );
    if !report.is_verified() {
        return Err(ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({"error": "claim-only", "report": report}),
        ));
    }

    let (usd_per_zec, rate_source, rate_at) =
        reference_rate(&st.http, st.policy.rate_max_age_secs).await?;
    let principal_zat = ((req.value_usd as f64 / usd_per_zec) * 1e8).ceil() as u64;
    let outstanding = db::outstanding_zat(&inner.book, &p.object.agent_id)?;
    let cap = report.claim_zat.min(report.anchored_zat) * st.policy.ltv_bps / 10_000;
    let limit = cap.saturating_sub(outstanding);
    if principal_zat > limit {
        return Err(ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({
                "error": "limit-exceeded", "principalZat": principal_zat, "limitZat": limit,
                "outstandingZat": outstanding, "report": report,
            }),
        ));
    }

    let purchase = st
        .bitrefill
        .buy(&req.product_id, req.value_usd, st.policy.mode == "test")
        .await
        .map_err(|e| reject(StatusCode::BAD_GATEWAY, "purchase-failed", e.to_string()))?;

    let mut loan_id = [0u8; 16];
    getrandom::fill(&mut loan_id).map_err(|e| anyhow!("rng: {e}"))?;
    let memo = LoanMemo { loan_id }.encode_padded();
    let memo_hex = hex::encode(&memo[..LoanMemo::LEN]);
    let repay_to = inner.wallet.address("treasury", false)?;
    let now = now_epoch_secs();
    let loan = db::Loan {
        id: hex::encode(loan_id),
        agent_id: p.object.agent_id.clone(),
        object_b64u: p.object.b64u().map_err(|e| anyhow!("{e}"))?,
        anchor_txid: p.object.anchor_txid.clone(),
        status: "open".into(),
        principal_zat,
        value_usd: req.value_usd as f64,
        usd_per_zec,
        rate_source,
        rate_at,
        product_id: req.product_id.clone(),
        bitrefill_invoice_id: purchase.invoice_id.clone(),
        bitrefill_order_id: purchase.order_id.clone(),
        repay_to: repay_to.clone(),
        memo_hex: memo_hex.clone(),
        due_height: report.tip_height + st.policy.max_tenor_blocks,
        opened_height: report.tip_height,
        repaid_txid: None,
        created_at: now,
        updated_at: now,
    };
    db::insert_loan(&inner.book, &loan)?;
    emit(
        &st,
        inner,
        Some(&loan.id),
        "loan-opened",
        Some(report.tip_height),
        json!({
            "agentId": loan.agent_id, "principalZat": principal_zat, "valueUsd": req.value_usd,
            "product": req.product_id, "bitrefillInvoiceId": purchase.invoice_id, "dueHeight": loan.due_height,
        }),
    );
    Ok(Json(json!({
        "loan": loan,
        "redemption": purchase.redemption,
        "repay": {"to": repay_to, "zat": principal_zat, "memoHex": memo_hex},
        "report": report,
    })))
}

async fn list_loans(State(st): State<Shared>) -> Result<Json<Value>, ApiError> {
    let inner = st.inner.lock().await;
    Ok(Json(json!({
        "loans": db::list_loans(&inner.book)?,
        "events": db::recent_events(&inner.book, 50)?,
    })))
}

async fn get_loan(
    State(st): State<Shared>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let inner = st.inner.lock().await;
    match db::get_loan(&inner.book, &id)? {
        Some(l) => Ok(Json(serde_json::to_value(l).map_err(anyhow::Error::from)?)),
        None => Err(reject(StatusCode::NOT_FOUND, "not-found", id)),
    }
}

async fn sse(
    State(st): State<Shared>,
) -> Sse<impl Stream<Item = Result<SseEvent, std::convert::Infallible>>> {
    let rx = st.events.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|e| {
        e.ok().map(|ev| {
            Ok(SseEvent::default()
                .event(ev.kind.clone())
                .data(serde_json::to_string(&ev).unwrap_or_default()))
        })
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// One underwriter tick: sync, match repayments, watch attestations, mark defaults.
async fn tick(st: &AppState) -> anyhow::Result<()> {
    let mut guard = st.inner.lock().await;
    let inner = &mut *guard;
    let active = db::active_loans(&inner.book)?;
    let report = inner.wallet.sync(&mut inner.client).await?;
    let tip = report.tip_height;

    // Repayments: memos only exist in full transactions, so enhance each new treasury receipt.
    let treasury = inner.wallet.account("treasury")?.expose_uuid();
    let new_txids: Vec<String> = {
        let views = inner.wallet.views()?;
        let mut s = views.prepare(
            "SELECT DISTINCT hex(txid) FROM v_tx_outputs
              WHERE to_account_uuid = ?1 AND is_change = 0 AND tx_mined_height IS NOT NULL",
        )?;
        let rows = s.query_map([treasury.as_bytes().to_vec()], |r| r.get::<_, String>(0))?;
        rows.collect::<Result<_, _>>()?
    };
    for hex_internal in new_txids {
        if db::mark_enhanced(&inner.book, &hex_internal)? {
            let bytes: [u8; 32] = hex::decode(&hex_internal)?
                .try_into()
                .map_err(|_| anyhow!("txid length"))?;
            inner.wallet.enhance(&mut inner.client, bytes).await?;
        }
    }
    let receipts: Vec<(String, i64, Vec<u8>)> = {
        let views = inner.wallet.views()?;
        let mut s = views.prepare(
            "SELECT hex(txid), value, memo FROM v_tx_outputs
              WHERE to_account_uuid = ?1 AND is_change = 0 AND tx_mined_height IS NOT NULL
                AND memo IS NOT NULL",
        )?;
        let rows = s.query_map([treasury.as_bytes().to_vec()], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        rows.collect::<Result<_, _>>()?
    };
    for (hex_internal, value, memo) in receipts {
        let Ok(m) = LoanMemo::decode(&memo) else {
            continue;
        };
        let id = hex::encode(m.loan_id);
        let Some(loan) = active.iter().find(|l| l.id == id) else {
            continue;
        };
        let mut display = hex::decode(&hex_internal)?;
        display.reverse();
        let txid = hex::encode(display);
        if (value as u64) >= loan.principal_zat {
            db::set_status(&inner.book, &id, "repaid", Some(&txid), now_epoch_secs())?;
            emit(
                st,
                inner,
                Some(&id),
                "loan-repaid",
                Some(tip),
                json!({"txid": txid, "valueZat": value}),
            );
        } else {
            emit(
                st,
                inner,
                Some(&id),
                "repayment-short",
                Some(tip),
                json!({"txid": txid, "valueZat": value, "principalZat": loan.principal_zat}),
            );
        }
    }

    // Live solvency watch on every still-active loan (SPEC §8).
    for loan in db::active_loans(&inner.book)? {
        let o = zsol_types::AttestationObject::from_b64u(&loan.object_b64u)
            .map_err(|e| anyhow!("{e}"))?;
        match zsol_verifier::watch_tick(&mut inner.wallet, &mut inner.client, &o).await {
            Ok(WatchState::Released {
                height,
                spent_in_txid,
            }) if loan.status == "open" => {
                db::set_status(&inner.book, &loan.id, "at-risk", None, now_epoch_secs())?;
                emit(
                    st,
                    inner,
                    Some(&loan.id),
                    "attestation-released",
                    Some(height),
                    json!({
                        "agentId": loan.agent_id, "spentInTxid": spent_in_txid, "newStatus": "at-risk",
                    }),
                );
            }
            Ok(WatchState::Expired) if loan.status == "open" => {
                emit(
                    st,
                    inner,
                    Some(&loan.id),
                    "attestation-expired",
                    Some(tip),
                    json!({"agentId": loan.agent_id}),
                );
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, loan = %loan.id, "watch tick failed"),
        }
        if tip > loan.due_height {
            db::set_status(&inner.book, &loan.id, "defaulted", None, now_epoch_secs())?;
            emit(
                st,
                inner,
                Some(&loan.id),
                "loan-defaulted",
                Some(tip),
                json!({"dueHeight": loan.due_height}),
            );
        }
    }
    let _ = st.events.send(db::Event {
        id: 0,
        loan_id: None,
        kind: "tick".into(),
        height: Some(tip),
        detail: json!({"scanned": report.fully_scanned_height}),
        at: now_epoch_secs(),
    });
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("ZSOL_LOG")
                .unwrap_or_else(|_| "info".into()),
        )
        .init();
    let dir = PathBuf::from(
        std::env::var("ZSOL_CREDIT_DIR")
            .context("ZSOL_CREDIT_DIR (underwriter wallet dir) is required")?,
    );
    let policy_path =
        std::env::var("ZSOL_CREDIT_POLICY").unwrap_or_else(|_| "config/credit-policy.json".into());
    let policy: Policy = serde_json::from_slice(
        &std::fs::read(&policy_path).with_context(|| format!("reading {policy_path}"))?,
    )?;
    let network = std::env::var("ZSOL_NETWORK").unwrap_or_else(|_| "test".into());
    let lwd = std::env::var("ZSOL_LWD").unwrap_or_else(|_| chain::DEFAULT_TESTNET_ENDPOINT.into());
    let bind = std::env::var("ZSOL_CREDIT_BIND").unwrap_or_else(|_| "127.0.0.1:8090".into());
    let key = std::env::var("BITREFILL_API_KEY").context("BITREFILL_API_KEY is required")?;

    let params = chain::parse_network(&network)?;
    let mut client = chain::connect(&lwd).await?;
    let info = chain::boot_check(&mut client, &params, &lwd).await?;
    tracing::info!(branch = %info.consensus_branch_id, tip = info.tip_height, "chain ok");
    let wallet = Wallet::open(&dir)?;
    wallet.account("treasury")?;
    let book = db::open(&dir.join("loans.sqlite"))?;
    let bitrefill = Bitrefill::new(key)?;
    bitrefill.ping().await.context("Bitrefill auth")?;
    let (tx, _) = broadcast::channel(256);
    let st: Shared = Arc::new(AppState {
        inner: Mutex::new(Inner {
            wallet,
            client,
            book,
        }),
        policy,
        network,
        lwd,
        bitrefill,
        http: reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()?,
        events: tx,
    });

    let ticker = st.clone();
    tokio::spawn(async move {
        loop {
            if let Err(e) = tick(&ticker).await {
                tracing::warn!(error = %e, "tick failed");
            }
            tokio::time::sleep(Duration::from_secs(20)).await;
        }
    });

    let app = Router::new()
        .route("/v0/health", get(health))
        .route("/v0/credit/challenge", post(new_challenge))
        .route("/v0/credit/open", post(open_credit))
        .route("/v0/credit/loans", get(list_loans))
        .route("/v0/credit/loans/{id}", get(get_loan))
        .route("/v0/events", get(sse))
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(st);
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, "zsol-credit listening");
    axum::serve(listener, app).await?;
    Ok(())
}
