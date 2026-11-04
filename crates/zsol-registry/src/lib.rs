//! zsol-registry — public claim-tier attestation registry (SPEC §7, SRS FR-3.x).
//!
//! Honesty Perimeter (P5): the registry is evidence ABOUT claims, never a source of
//! truth for solvency. It cannot verify balances, cannot scan, holds no keys (P2).
//! It only:
//!   1. structurally validates postings (canonical form, §9-floors),
//!   2. dedupes `(agentId, anchorTxid)`,
//!   3. appends digest-chained rows — enforced append-only via SQLite triggers
//!      (UPDATE/DELETE RAISE(ABORT)); there is no UPDATE/DELETE code path anywhere.
//!
//! Transport (SPEC §7):
//!   POST /v0/attestations  body {"object": b64u(canonical json)} → 201 {entryId, chainDigest}
//!   GET  /v0/attestations/{agentId} → entries ascending by seq with running chainDigest
//!   GET  /v0/health → {ok, chainLength}
//!   chainDigestₙ = BLAKE2b-256(chainDigestₙ₋₁ ‖ canonical(objectₙ)), genesis = zero digest.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use zsol_types::{chain_digest, AttestationObject, ReasonCode};

/// Transport-level rejection slugs (distinct from SPEC §9 status codes, which are
/// attestation-verification semantics; §9 codes are reused where the taxonomy has one).
#[derive(Debug, thiserror::Error)]
pub enum RegistryReject {
    #[error("malformed posting: {0}")]
    Malformed(String),
    #[error("non-canonical object bytes")]
    NonCanonical,
    /// SPEC §9 / SRS FR-4.2: the registry never accepts anything named like a key.
    #[error("key-like field {0:?} refused: the registry never carries keys")]
    KeyLikeField(String),
    #[error("{0}")]
    Reason(ReasonCode),
    #[error("duplicate (agentId, anchorTxid); existing entry {entry_id}")]
    Duplicate {
        entry_id: String,
        chain_digest_hex: String,
    },
    #[error("registry unavailable: {0}")]
    Db(String),
    /// FR-3.4 block-diff guard: a POST whose anchorHeight is below the highest
    /// already-accepted anchorHeight is refused with 503 — mutations are refused
    /// while the chain appears to regress (SRS FR-3.4, T-08 harness).
    #[error("chain descended: posted anchorHeight {posted} < watermark {watermark}")]
    ChainDescended { watermark: u64, posted: u64 },
    /// FR-3.4 rate limit (429).
    #[error("rate limited: retry in {retry_after_ms} ms")]
    RateLimited { retry_after_ms: u64 },
}

impl RegistryReject {
    /// (status, body) per SPEC §7 + §9 code reuse.
    pub fn wire(&self) -> (StatusCode, Value) {
        match self {
            Self::Malformed(d) => (
                StatusCode::BAD_REQUEST,
                json!({"error": "malformed", "detail": d}),
            ),
            Self::NonCanonical => (StatusCode::BAD_REQUEST, json!({"error": "non-canonical"})),
            Self::KeyLikeField(f) => (
                StatusCode::BAD_REQUEST,
                json!({"error": "key-like-field", "field": f}),
            ),
            Self::Reason(rc) => (
                StatusCode::BAD_REQUEST,
                json!({"error": "reject", "reason": rc.as_str()}),
            ),
            Self::Duplicate {
                entry_id,
                chain_digest_hex,
            } => (
                StatusCode::CONFLICT,
                json!({"error": "duplicate", "entryId": entry_id, "chainDigest": chain_digest_hex}),
            ),
            Self::Db(d) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"error": "registry-unavailable", "detail": d}),
            ),
            Self::ChainDescended { watermark, posted } => (
                StatusCode::SERVICE_UNAVAILABLE,
                json!({"error": "chain-descended", "watermarkHeight": watermark, "postedHeight": posted}),
            ),
            Self::RateLimited { retry_after_ms } => (
                StatusCode::TOO_MANY_REQUESTS,
                json!({"error": "rate-limited", "retryAfterMs": retry_after_ms}),
            ),
        }
    }
}

/// Genesis of the digest chain (SPEC §7): digest₀ = 0.
pub const GENESIS_DIGEST_HEX: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Mutex<Connection>>,
}

/// Create schema if absent. Triggers make the table physically append-only.
pub fn init_db(conn: &mut Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS chain_head (
            id     INTEGER PRIMARY KEY CHECK (id = 1),
            digest BLOB NOT NULL,
            length INTEGER NOT NULL
        );
        INSERT OR IGNORE INTO chain_head (id, digest, length)
            VALUES (1, zeroblob(32), 0);

        CREATE TABLE IF NOT EXISTS attestations (
            entry_id    TEXT PRIMARY KEY,      -- hex(chainDigest), position-unique
            seq         INTEGER NOT NULL UNIQUE,
            agent_id    TEXT NOT NULL,
            anchor_txid TEXT NOT NULL,
            issued_at   INTEGER NOT NULL,
            received_at INTEGER NOT NULL,
            object      BLOB NOT NULL,         -- canonical bytes INCLUDING memoBinding
            digest      BLOB NOT NULL,         -- running chain digest (row-level)
            UNIQUE (agent_id, anchor_txid)     -- SPEC §7 dedupe
        );
        CREATE INDEX IF NOT EXISTS idx_attest_agent_seq ON attestations (agent_id, seq);

        CREATE TRIGGER IF NOT EXISTS attestations_no_update
        BEFORE UPDATE ON attestations
        BEGIN
            SELECT RAISE(ABORT, 'append-only: attestations rows are immutable');
        END;
        CREATE TRIGGER IF NOT EXISTS attestations_no_delete
        BEFORE DELETE ON attestations
        BEGIN
            SELECT RAISE(ABORT, 'append-only: attestations rows are immutable');
        END;
        CREATE TRIGGER IF NOT EXISTS chain_head_no_update
        BEFORE UPDATE ON chain_head WHEN OLD.id <> NEW.id
        BEGIN
            SELECT RAISE(ABORT, 'chain_head: only the singleton row may advance');
        END;
        CREATE TRIGGER IF NOT EXISTS chain_head_no_delete
        BEFORE DELETE ON chain_head
        BEGIN
            SELECT RAISE(ABORT, 'chain_head: immutable singleton');
        END;

        -- FR-3.4 block-diff guard state: highest anchorHeight ever accepted.
        -- Trigger makes regression physically impossible, not just inconvenient.
        CREATE TABLE IF NOT EXISTS chain_meta (
            id               INTEGER PRIMARY KEY CHECK (id = 1),
            watermark_height INTEGER NOT NULL
        );
        INSERT OR IGNORE INTO chain_meta (id, watermark_height) VALUES (1, 0);
        CREATE TRIGGER IF NOT EXISTS chain_meta_no_descend
        AFTER UPDATE ON chain_meta
        WHEN NEW.watermark_height < OLD.watermark_height
        BEGIN
            SELECT RAISE(ABORT, 'chain-descended: the watermark may only advance');
        END;
        CREATE TRIGGER IF NOT EXISTS chain_meta_no_delete
        BEFORE DELETE ON chain_meta
        BEGIN
            SELECT RAISE(ABORT, 'chain_meta: immutable singleton');
        END;
        "#,
    )
}

fn genesis() -> [u8; 32] {
    [0u8; 32]
}

/// Core publish logic (SPEC §7 acceptance order): decode → parse → canonicalize →
/// validate → dedupe → chain-append. All inside one IMMEDIATE transaction.
/// Field names that denote key material (SPEC §9: `ufvk`, `uivk`, `seed`, `sk`, plus common
/// spellings of spending keys / mnemonics). Compared case-insensitively with `_`/`-` removed.
const KEY_LIKE_FIELDS: &[&str] = &[
    "ufvk",
    "uivk",
    "seed",
    "seedphrase",
    "mnemonic",
    "sk",
    "usk",
    "spendingkey",
    "viewingkey",
    "privatekey",
];

fn is_key_like(name: &str) -> bool {
    let norm: String = name
        .chars()
        .filter(|c| *c != '_' && *c != '-')
        .collect::<String>()
        .to_ascii_lowercase();
    KEY_LIKE_FIELDS.contains(&norm.as_str())
}

/// First key-like field name anywhere in `v` (objects and arrays are walked recursively).
fn find_key_like(v: &Value) -> Option<String> {
    match v {
        Value::Object(m) => m.iter().find_map(|(k, child)| {
            if is_key_like(k) {
                Some(k.clone())
            } else {
                find_key_like(child)
            }
        }),
        Value::Array(items) => items.iter().find_map(find_key_like),
        _ => None,
    }
}

pub fn publish(conn: &mut Connection, body: Value, now: u64) -> Result<Published, RegistryReject> {
    // FR-4.2: refuse key-like fields in the envelope before anything else is looked at.
    if let Some(f) = find_key_like(&body) {
        return Err(RegistryReject::KeyLikeField(f));
    }
    let obj_field = body
        .as_object()
        .and_then(|m| m.get("object"))
        .and_then(Value::as_str)
        .ok_or_else(|| RegistryReject::Malformed("body must be {\"object\": \"<b64u>\"}".into()))?;

    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(obj_field)
        .map_err(|e| RegistryReject::Malformed(format!("b64u decode: {e}")))?;

    // FR-4.2 inside the object too (serde's deny_unknown_fields would also refuse it, but
    // as a generic malformed error; this names the offending field).
    if let Some(f) = serde_json::from_slice::<Value>(&bytes)
        .ok()
        .as_ref()
        .and_then(find_key_like)
    {
        return Err(RegistryReject::KeyLikeField(f));
    }

    let object: AttestationObject = serde_json::from_slice(&bytes)
        .map_err(|e| RegistryReject::Malformed(format!("object json: {e}")))?;

    // Non-canonical bytes are rejected verbatim (SPEC §7 MUST-reject list).
    let canonical = object
        .canonical()
        .map_err(|_| RegistryReject::Malformed("canonicalization failed".into()))?;
    if canonical != bytes {
        return Err(RegistryReject::NonCanonical);
    }

    // Structural floors (§9 codes reused where the taxonomy has one).
    // Structural floors (SPEC §6 C1 codes). Unknown fields — e.g. a smuggled `ufvk` — were
    // already refused by serde (`deny_unknown_fields`), so keys can never be stored (FR-4.2).
    let network = match object.network.as_str() {
        n @ ("test" | "main") => n.to_string(),
        _ => return Err(RegistryReject::Reason(ReasonCode::NetworkMismatch)),
    };
    object.validate(&network).map_err(RegistryReject::Reason)?;

    let tx = conn
        .transaction()
        .map_err(|e| RegistryReject::Db(format!("txn open: {e}")))?;

    let (prev_digest, seq): (Vec<u8>, i64) = tx
        .query_row(
            "SELECT digest, length FROM chain_head WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| RegistryReject::Db(format!("head read: {e}")))?;

    let mut prev = genesis();
    if prev_digest.len() != 32 {
        return Err(RegistryReject::Db("head digest not 32 bytes".into()));
    }
    prev.copy_from_slice(&prev_digest);

    // FR-3.4 block-diff guard: refuse mutations whose anchorHeight regresses
    // below the highest accepted height (checked in-txn so concurrent posts
    // cannot race past the watermark).
    let watermark: u64 = tx
        .query_row(
            "SELECT watermark_height FROM chain_meta WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .map_err(|e| RegistryReject::Db(format!("watermark read: {e}")))?;
    if u64::from(object.anchor_height) < watermark {
        return Err(RegistryReject::ChainDescended {
            watermark,
            posted: u64::from(object.anchor_height),
        });
    }

    // Dedupe inside the txn so a concurrent equal post can't race past us.
    let existing: Option<(String, Vec<u8>)> = tx
        .query_row(
            "SELECT entry_id, digest FROM attestations WHERE agent_id = ?1 AND anchor_txid = ?2",
            params![object.agent_id, object.anchor_txid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| RegistryReject::Db(format!("dedupe check: {e}")))?;
    if let Some((entry_id, digest)) = existing {
        let chain_digest_hex = hex::encode(digest);
        return Err(RegistryReject::Duplicate {
            entry_id,
            chain_digest_hex,
        });
    }

    // SPEC §7 exact formula: chainDigestₙ = BLAKE2b-256(chainDigestₙ₋₁ ‖ canonical(objectₙ))
    let new_digest = chain_digest(prev, &canonical);
    let entry_id = hex::encode(new_digest);
    let seq = seq + 1;

    tx.execute(
        "INSERT INTO attestations
            (entry_id, seq, agent_id, anchor_txid, issued_at, received_at, object, digest)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            entry_id,
            seq,
            object.agent_id,
            object.anchor_txid,
            object.issued_at,
            now,
            &canonical,
            &new_digest[..]
        ],
    )
    .map_err(|e| RegistryReject::Db(format!("insert: {e}")))?;

    tx.execute(
        "UPDATE chain_head SET digest = ?1, length = ?2 WHERE id = 1",
        params![&new_digest[..], seq],
    )
    .map_err(|e| RegistryReject::Db(format!("head advance: {e}")))?;

    // Watermark only ever advances (trigger guards the floor).
    if u64::from(object.anchor_height) > watermark {
        tx.execute(
            "UPDATE chain_meta SET watermark_height = ?1 WHERE id = 1",
            params![object.anchor_height],
        )
        .map_err(|e| RegistryReject::Db(format!("watermark advance: {e}")))?;
    }

    tx.commit()
        .map_err(|e| RegistryReject::Db(format!("txn commit: {e}")))?;

    Ok(Published {
        entry_id,
        chain_digest_hex: hex::encode(new_digest),
        seq,
    })
}

#[derive(Debug, Clone)]
pub struct Published {
    pub entry_id: String,
    pub chain_digest_hex: String,
    pub seq: i64,
}

#[derive(Debug, Clone)]
pub struct FetchEntry {
    pub seq: i64,
    pub entry_id: String,
    pub chain_digest_hex: String,
    /// Digest of the globally-prior row (whole-chain chain is one sequence);
    /// `None` marks the very first entry, whose prev is the zero genesis.
    pub prev_digest_hex: Option<String>,
    pub received_at: u64,
    /// Canonical bytes exactly as stored — verifiers recompute the chain from these.
    pub object_canonical: Vec<u8>,
    pub object: AttestationObject,
}

#[derive(Debug)]
pub struct FetchResult {
    pub agent_id: String,
    pub entries: Vec<FetchEntry>,
    /// Head of the WHOLE chain (not per-agent) — insertion order is global.
    pub chain_head_hex: Option<String>,
}

/// Ascending-by-adjacency fetch (SRS FR-3.2): rows ascending with running chainDigest.
pub fn fetch(conn: &Connection, agent_id: &str) -> Result<FetchResult, RegistryReject> {
    let mut stmt = conn
        .prepare(
            "SELECT seq, entry_id, digest, received_at, object
             FROM attestations WHERE agent_id = ?1 ORDER BY seq ASC",
        )
        .map_err(|e| RegistryReject::Db(format!("prepare: {e}")))?;

    let mut entries = Vec::new();
    // Prev-digest adjacency is GLOBAL (single chain_head row): each entry's
    // chainDigest depends on the digest of whatever was inserted immediately
    // before it across ALL agents. Export that adjacency so verifiers
    // recompute the exact SPEC §7 formula instead of per-agent assumptions.
    let mut prevs: HashMap<String, String> = HashMap::new();
    {
        let mut all = conn
            .prepare("SELECT entry_id, digest FROM attestations ORDER BY seq ASC")
            .map_err(|e| RegistryReject::Db(format!("prepare: {e}")))?;
        let walk = all
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
            })
            .map_err(|e| RegistryReject::Db(format!("rows: {e}")))?;
        let mut prev: Option<(String, String)> = None; // (entry_id, hex digest)
        for row in walk {
            let (id, digest) = row.map_err(|e| RegistryReject::Db(format!("row: {e}")))?;
            if let Some((_, ph)) = &prev {
                prevs.insert(id.clone(), ph.clone());
            }
            prev = Some((id, hex::encode(digest)));
        }
    }
    let rows = stmt
        .query_map(params![agent_id], |r| {
            let entry_id: String = r.get(1)?;
            let obj_bytes: Vec<u8> = r.get(4)?;
            let object: AttestationObject = serde_json::from_slice(&obj_bytes).map_err(|_| {
                rusqlite::Error::FromSqlConversionFailure(
                    4,
                    rusqlite::types::Type::Blob,
                    "stored object is not a valid AttestationObject".into(),
                )
            })?;
            Ok(FetchEntry {
                seq: r.get(0)?,
                prev_digest_hex: prevs.get(&entry_id).cloned(),
                entry_id,
                chain_digest_hex: hex::encode({
                    let d: Vec<u8> = r.get(2)?;
                    d
                }),
                received_at: r.get(3)?,
                object_canonical: obj_bytes.clone(),
                object,
            })
        })
        .map_err(|e| RegistryReject::Db(format!("fetch: {e}")))?;

    for row in rows {
        entries.push(row.map_err(|e| RegistryReject::Db(format!("row: {e}")))?);
    }

    let chain_head_hex = conn
        .query_row(
            "SELECT hex(digest) FROM chain_head WHERE id = 1 AND length > 0",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map(|row| row.map(|s| s.to_lowercase()))
        .map_err(|e| RegistryReject::Db(format!("head: {e}")))?;

    Ok(FetchResult {
        agent_id: agent_id.to_string(),
        entries,
        chain_head_hex,
    })
}

pub struct HealthInfo {
    pub ok: bool,
    pub chain_length: i64,
    pub watermark_height: u64,
}

pub fn health(conn: &Connection) -> Result<HealthInfo, RegistryReject> {
    let (length, watermark): (i64, u64) = conn
        .query_row(
            "SELECT length, watermark_height FROM chain_head, chain_meta WHERE chain_head.id = 1 AND chain_meta.id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| RegistryReject::Db(format!("health: {e}")))?;
    Ok(HealthInfo {
        ok: true,
        chain_length: length,
        watermark_height: watermark,
    })
}

// ---------------- FR-3.4 rate limiter (per-IP token bucket; 429) ----------------

pub struct RateLimiter {
    buckets: Mutex<HashMap<String, Bucket>>,
    pub per_second: f64,
    pub burst: f64,
}

struct Bucket {
    tokens: f64,
    last_refill: std::time::Instant,
}

pub enum LimitDecision {
    Allowed,
    Limited { retry_after_ms: u64 },
}

impl RateLimiter {
    pub fn new(per_second: u32, burst: u32) -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
            per_second: (per_second.max(1)) as f64,
            burst: (burst.max(1)) as f64,
        }
    }

    /// One token = one request for this key. Refill is continuous.
    pub fn check(&self, key: &str) -> LimitDecision {
        let now = std::time::Instant::now();
        // Poison recovery: availability over panic propagation.
        let mut buckets = self
            .buckets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let bucket = buckets.entry(key.to_string()).or_insert(Bucket {
            tokens: self.burst,
            last_refill: now,
        });
        let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * self.per_second).min(self.burst);
        bucket.last_refill = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            LimitDecision::Allowed
        } else {
            let need = 1.0 - bucket.tokens;
            LimitDecision::Limited {
                retry_after_ms: (need / self.per_second * 1000.0).ceil() as u64,
            }
        }
    }
}

/// Body size caps + rate budgets, env-tunable: ZSOL_REGISTRY_MAX_BODY,
/// ZSOL_REGISTRY_RATE_PER_SEC, ZSOL_REGISTRY_RATE_BURST.
#[derive(Clone, Debug)]
pub struct Limits {
    pub max_body_bytes: usize,
    pub post_per_second: u32,
    pub post_burst: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_body_bytes: 64_000,
            post_per_second: 5,
            post_burst: 10,
        }
    }
}

impl Limits {
    pub fn from_env() -> Self {
        let env = |k: &str, d: i64| {
            std::env::var(k)
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(d)
        };
        Self {
            max_body_bytes: env("ZSOL_REGISTRY_MAX_BODY", 64_000).max(1) as usize,
            post_per_second: env("ZSOL_REGISTRY_RATE_PER_SEC", 5).max(1) as u32,
            post_burst: env("ZSOL_REGISTRY_RATE_BURST", 10).max(1) as u32,
        }
    }
}

// ---------------- T-29 JSON logs with req_id (NF-6.2: no PII at info) ----------------

static REQ_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Short, non-reversible request id (utc-nanosecond slice + seq). Never an address.
pub fn next_req_id() -> String {
    use std::sync::atomic::Ordering;
    let n = REQ_COUNTER.fetch_add(1, Ordering::Relaxed);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    format!("{:07x}-{:04x}", t & 0xfff_ffff_ffff, n)
}

/// Log only the route TEMPLATE — concrete agentIds never appear at info (NF-6.2).
fn redact_path(path: &str) -> String {
    if path.starts_with("/v0/attestations/") {
        "/v0/attestations/:id".into()
    } else {
        path.into()
    }
}

async fn request_logger(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::header;
    let method = req.method().clone();
    let path = redact_path(req.uri().path());
    let started = std::time::Instant::now();
    let req_id = next_req_id();
    let mut resp = next.run(req).await;
    tracing::info!(
        req_id = %req_id,
        method = %method,
        path = %path,
        status = resp.status().as_u16(),
        ms = started.elapsed().as_millis() as u64,
        "request"
    );
    if let Ok(v) = header::HeaderValue::from_str(&req_id) {
        resp.headers_mut()
            .insert(header::HeaderName::from_static("x-request-id"), v);
    }
    resp
}

async fn rate_limit_post(
    axum::extract::State(limiter): axum::extract::State<Arc<RateLimiter>>,
    axum::extract::ConnectInfo(addr): axum::extract::ConnectInfo<std::net::SocketAddr>,
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if req.method() != axum::http::Method::POST {
        return next.run(req).await;
    }
    match limiter.check(&addr.ip().to_string()) {
        LimitDecision::Allowed => next.run(req).await,
        LimitDecision::Limited { retry_after_ms } => {
            tracing::warn!(ip = %addr.ip(), ms = retry_after_ms, "rate limited POST");
            let (status, body) = RegistryReject::RateLimited { retry_after_ms }.wire();
            (status, Json(body)).into_response()
        }
    }
}

// ---------------- axum glue (thin; logic lives above) ----------------

fn state_db_guard(
    state: &AppState,
) -> Result<std::sync::MutexGuard<'_, Connection>, RegistryReject> {
    state
        .db
        .lock()
        .map_err(|_| RegistryReject::Db("connection mutex poisoned (previous panic)".into()))
}

async fn publish_route(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let mut guard = match state_db_guard(&state) {
        Ok(g) => g,
        Err(e) => return e.wire().into_wrapped(),
    };
    match publish(&mut guard, body, zsol_types::now_epoch_secs()) {
        Ok(p) => (
            StatusCode::CREATED,
            Json(json!({"entryId": p.entry_id, "chainDigest": p.chain_digest_hex})),
        ),
        Err(e) => e.wire().into_wrapped(),
    }
}

async fn fetch_route(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let guard = match state_db_guard(&state) {
        Ok(g) => g,
        Err(e) => return e.wire().into_wrapped(),
    };
    match fetch(&guard, &agent_id) {
        Ok(f) => (
            StatusCode::OK,
            Json(json!({
                "agentId": f.agent_id,
                "chainHead": f.chain_head_hex,
                "entries": f.entries.iter().map(|e| json!({
                    "seq": e.seq,
                    "entryId": e.entry_id,
                    "chainDigest": e.chain_digest_hex,
                    "prevDigest": e.prev_digest_hex,
                    "receivedAt": e.received_at,
                    "object": e.object,
                })).collect::<Vec<_>>(),
            })),
        ),
        Err(e) => e.wire().into_wrapped(),
    }
}

async fn health_route(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    let guard = match state_db_guard(&state) {
        Ok(g) => g,
        Err(e) => return e.wire().into_wrapped(),
    };
    match health(&guard) {
        Ok(h) => (
            StatusCode::OK,
            Json(json!({
                "ok": h.ok,
                "chainLength": h.chain_length,
                "watermarkHeight": h.watermark_height,
            })),
        ),
        Err(e) => e.wire().into_wrapped(),
    }
}

trait WireTuple {
    fn into_wrapped(self) -> (StatusCode, Json<Value>);
}

impl WireTuple for (StatusCode, Value) {
    fn into_wrapped(self) -> (StatusCode, Json<Value>) {
        (self.0, Json(self.1))
    }
}

/// Router with FR-3.4 layers. Outermost = logger (sees 429/413 too), then the
/// per-IP POST token bucket, then the body-size cap; routes innermost.
/// Serve with `app.into_make_service_with_connect_info::<SocketAddr>()` so the
/// rate bucket keys on the real client IP.
pub fn router(state: AppState, limits: Limits) -> Router {
    let limiter = Arc::new(RateLimiter::new(limits.post_per_second, limits.post_burst));
    Router::new()
        .route("/v0/attestations", post(publish_route))
        // axum 0.8 path-param syntax
        .route("/v0/attestations/{agentId}", get(fetch_route))
        .route("/v0/health", get(health_route))
        .with_state(state)
        .layer(axum::extract::DefaultBodyLimit::max(limits.max_body_bytes))
        .layer(axum::middleware::from_fn_with_state(
            limiter,
            rate_limit_post,
        ))
        .layer(axum::middleware::from_fn(request_logger))
}

// ---------------- tests (SRS FR-3.x ACs) ----------------

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    fn open_mem() -> Connection {
        let mut c = Connection::open_in_memory().unwrap();
        init_db(&mut c).unwrap();
        c
    }

    /// Self-consistent 0.2 fixture: memoBinding = 0xFF‖zs02‖D(frame).
    fn sample(agent: &str, txid: &str, issued_at: u64) -> AttestationObject {
        sample_h(agent, txid, issued_at, 4_544_800)
    }

    /// Height-parameterized variant (T-08 harness needs distinct heights).
    fn sample_h(agent: &str, txid: &str, issued_at: u64, height: u64) -> AttestationObject {
        let mut o = AttestationObject {
            protocol: zsol_types::PROTOCOL.into(),
            network: "test".into(),
            agent_id: agent.into(),
            claim_zat: 1_000_000,
            pool: "ironwood".into(),
            session_key: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
                ed25519_dalek::SigningKey::from_bytes(&[7u8; 32])
                    .verifying_key()
                    .as_bytes(),
            ),
            birthday: 4_540_000,
            issued_at,
            max_age_hours: 24,
            anchor_txid: txid.into(),
            anchor_height: height as u32,
            memo_binding: String::new(),
        };
        o.memo_binding =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(o.expected_memo().unwrap());
        o
    }

    fn body_of(o: &AttestationObject) -> Value {
        json!({"object": o.b64u().unwrap()})
    }

    /// FR-3.1/OBJ-2: canonical post accepted; entryId = hex(chainDigest) at genesis.
    #[test]
    fn ac_publish_canonical_genesis() {
        let mut c = open_mem();
        let o = sample("agent-1", &"a".repeat(64), 1_769_337_600);
        let p = publish(&mut c, body_of(&o), 111).unwrap();
        let expected = chain_digest(genesis(), &o.canonical().unwrap());
        assert_eq!(p.entry_id, hex::encode(expected));
        assert_eq!(p.chain_digest_hex, p.entry_id);
        assert_eq!(p.seq, 1);
    }

    /// Registry MUST reject non-canonical object bytes (struct-order JSON ≠ canonical).
    #[test]
    fn ac_publish_rejects_non_canonical() {
        let mut c = open_mem();
        let o = sample("agent-1", &"b".repeat(64), 1_769_337_600);
        let struct_order = serde_json::to_vec(&o).unwrap(); // NOT key-sorted
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&struct_order);
        let err = publish(&mut c, json!({"object": b64}), 111).unwrap_err();
        assert!(matches!(err, RegistryReject::NonCanonical));
    }

    /// MUST reject dust + unknown protocol (§9 codes reused).
    #[test]
    fn ac_publish_rejects_dust_and_protocol() {
        let mut c = open_mem();
        let mut dust = sample("agent-1", &"c".repeat(64), 1_769_337_600);
        dust.claim_zat = 9_999;
        let err = publish(&mut c, body_of(&dust), 111).unwrap_err();
        assert!(matches!(err, RegistryReject::Reason(ReasonCode::DustClaim)));

        let mut proto = sample("agent-1", &"d".repeat(64), 1_769_337_600);
        proto.protocol = "other/9".into();
        let err = publish(&mut c, body_of(&proto), 111).unwrap_err();
        assert!(matches!(
            err,
            RegistryReject::Reason(ReasonCode::ProtocolUnknown)
        ));
    }

    /// MUST reject duplicate (agentId, anchorTxid) with existing entry surfaced.
    #[test]
    fn ac_publish_rejects_duplicate() {
        let mut c = open_mem();
        let o = sample("agent-1", &"e".repeat(64), 1_769_337_600);
        let first = publish(&mut c, body_of(&o), 111).unwrap();
        let err = publish(&mut c, body_of(&o), 112).unwrap_err();
        match err {
            RegistryReject::Duplicate {
                entry_id,
                chain_digest_hex,
            } => {
                assert_eq!(entry_id, first.entry_id);
                assert_eq!(chain_digest_hex, first.chain_digest_hex);
            }
            other => panic!("expected Duplicate, got {other:?}"),
        }
        // Same agent, different txid → NOT a duplicate (append allowed)
        let mut o2 = o.clone();
        o2.anchor_txid = "f".repeat(64);
        publish(&mut c, body_of(&o2), 113).unwrap();
    }

    /// FR-3.2: ascending entries with running chainDigest, independently recomputed.
    #[test]
    fn ac_fetch_ascending_running_chain() {
        let mut c = open_mem();
        let mut prev = genesis();
        for i in 0..3u64 {
            let o = sample(
                "agent-1",
                &format!("{:064x}", i + 1),
                1_769_337_600 + i * 10,
            );
            publish(&mut c, body_of(&o), 100 + i).unwrap();
            let f = fetch(&c, "agent-1").unwrap();
            assert_eq!(f.entries.len(), (i as usize) + 1);
            let last = f.entries.last().unwrap();
            // Independent recompute of the running chain digest from stored bytes
            let expected = chain_digest(prev, &last.object_canonical);
            assert_eq!(hex::encode(expected), last.chain_digest_hex);
            prev = expected;
        }
        // ascending by seq and by issuedAt
        let f = fetch(&c, "agent-1").unwrap();
        for w in f.entries.windows(2) {
            assert!(w[0].seq < w[1].seq);
            assert!(w[0].object.issued_at < w[1].object.issued_at);
        }
        assert_eq!(hex::encode(prev), f.chain_head_hex.unwrap());
    }

    /// FR-1.5 tamper AC + SRS invariant 6: row mutation MUST be physically rejected.
    #[test]
    fn ac_append_only_enforced_by_triggers() {
        let mut c = open_mem();
        let o = sample("agent-1", "1f".repeat(64).get(..64).unwrap(), 1_769_337_600);
        publish(&mut c, body_of(&o), 111).unwrap();
        let up = c.execute(
            "UPDATE attestations SET object = ?1 WHERE seq = 1",
            params![&o.canonical().unwrap()],
        );
        assert!(up.is_err(), "UPDATE must abort (append-only trigger)");
        let del = c.execute("DELETE FROM attestations WHERE seq = 1", []);
        assert!(del.is_err(), "DELETE must abort (append-only trigger)");
        // state unchanged: still fetchable
        assert_eq!(fetch(&c, "agent-1").unwrap().entries.len(), 1);
    }

    /// Chain head only advances on real appends; genesis stays zero-length.
    #[test]
    fn ac_health_genesis_and_advance() {
        let c = open_mem();
        let h = health(&c).unwrap();
        assert!(h.ok);
        assert_eq!(h.chain_length, 0);
        assert_eq!(h.watermark_height, 0);
    }

    /// T-08 harness (FR-3.4): height regression ⇒ ChainDescended; equal-or-higher OK.
    #[test]
    fn ac_t08_chain_descended_harness() {
        let mut c = open_mem();
        // watermark starts at 0; any height accepted first
        publish(
            &mut c,
            body_of(&sample_h("agent-1", &"10".repeat(32), 111, 500)),
            111,
        )
        .expect("first anchor at height 500 accepted");
        // REGRESSION: 499 < 500 → refused with ChainDescended
        let err = publish(
            &mut c,
            body_of(&sample_h("agent-1", &"11".repeat(32), 112, 499)),
            112,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            RegistryReject::ChainDescended {
                watermark: 500,
                posted: 499
            }
        ));
        // wire status is 503 with slug chain-descended
        let (status, body) = err.wire();
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["error"], "chain-descended");
        // EQUAL height is allowed (multiple anchors may share a height)
        publish(
            &mut c,
            body_of(&sample_h("agent-1", &"12".repeat(32), 113, 500)),
            113,
        )
        .expect("equal height accepted");
        // advance
        publish(
            &mut c,
            body_of(&sample_h("agent-1", &"13".repeat(32), 114, 501)),
            114,
        )
        .expect("advance accepted");
        // finally: an older agent re-anchoring at an older height regresses
        let err = publish(
            &mut c,
            body_of(&sample_h("agent-2", &"14".repeat(32), 115, 400)),
            115,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            RegistryReject::ChainDescended { watermark: 501, .. }
        ));
        // health reports the watermark
        let h = health(&c).unwrap();
        assert_eq!(h.watermark_height, 501);
    }

    /// The anti-regression trigger is PHYSICAL: direct SQL cannot lower the watermark.
    #[test]
    fn ac_chain_descended_trigger_physical() {
        let mut c = open_mem();
        publish(
            &mut c,
            body_of(&sample_h("agent-1", &"a1".repeat(32), 111, 900)),
            111,
        )
        .unwrap();
        let lowered = c.execute(
            "UPDATE chain_meta SET watermark_height = 500 WHERE id = 1",
            params![],
        );
        assert!(
            lowered.is_err(),
            "watermark regression must abort (trigger)"
        );
        let deleted = c.execute("DELETE FROM chain_meta WHERE id = 1", params![]);
        assert!(deleted.is_err(), "chain_meta row is immutable");
    }

    /// FR-3.4 rate limit: burst exhausts → Limited with retry-after; other keys
    /// unaffected; time refills the bucket.
    #[test]
    fn ac_rate_limiter_bucket() {
        let rl = RateLimiter::new(2, 2); // 2/s, burst 2
        for _ in 0..2 {
            assert!(matches!(rl.check("ip-1"), LimitDecision::Allowed));
        }
        match rl.check("ip-1") {
            LimitDecision::Limited { retry_after_ms } => assert!(retry_after_ms > 0),
            LimitDecision::Allowed => panic!("burst exhausted but allowed"),
        }
        // other key isolated
        assert!(matches!(rl.check("ip-2"), LimitDecision::Allowed));
        // ~0.6 s at 2/s ≈ 1 token back
        std::thread::sleep(std::time::Duration::from_millis(600));
        assert!(matches!(rl.check("ip-1"), LimitDecision::Allowed));
    }

    /// POST through the real router (all FR-3.4 layers) without binding a socket.
    async fn post_http(body: Value) -> (StatusCode, Value) {
        use tower::ServiceExt as _;
        let state = AppState {
            db: Arc::new(Mutex::new(open_mem())),
        };
        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri("/v0/attestations")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                [127, 0, 0, 1],
                1,
            ))));
        let resp = router(state, Limits::default()).oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// FR-4.2 AC: a POST containing a `ufvk` field returns 400 (and nothing is stored).
    #[tokio::test]
    async fn ac_fr42_post_with_ufvk_is_400() {
        let o = sample("agent-1", &"a".repeat(64), 1_769_337_600);
        // control: the clean envelope is accepted
        let (status, _) = post_http(body_of(&o)).await;
        assert_eq!(status, StatusCode::CREATED);

        let mut body = body_of(&o);
        body["ufvk"] = json!("uviewtest1qqqq");
        let (status, reply) = post_http(body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(reply["error"], "key-like-field");
        assert_eq!(reply["field"], "ufvk");
    }

    /// FR-4.2 / SPEC §9: every key-like name is refused with 400, at any depth and in
    /// either envelope or object, case/separator-insensitively; nothing is appended.
    #[tokio::test]
    async fn ac_fr42_key_like_fields_rejected() {
        let o = sample("agent-1", &"b".repeat(64), 1_769_337_600);
        for name in [
            "ufvk",
            "uivk",
            "seed",
            "sk",
            "spendingKey",
            "spending_key",
            "USK",
            "mnemonic",
        ] {
            let mut top = body_of(&o);
            top[name] = json!("x");
            let (status, reply) = post_http(top).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "top-level {name}");
            assert_eq!(reply["error"], "key-like-field", "top-level {name}");

            let nested = json!({"object": o.b64u().unwrap(), "meta": [{name: "x"}]});
            let (status, _) = post_http(nested).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "nested {name}");
        }

        // smuggled inside the b64u object itself
        let mut inner = serde_json::to_value(&o).unwrap();
        inner["ufvk"] = json!("uviewtest1qqqq");
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&inner).unwrap());
        let mut c = open_mem();
        let err = publish(&mut c, json!({"object": b64}), 111).unwrap_err();
        assert!(matches!(err, RegistryReject::KeyLikeField(ref f) if f == "ufvk"));
        assert_eq!(err.wire().0, StatusCode::BAD_REQUEST);
        assert_eq!(health(&c).unwrap().chain_length, 0, "nothing stored");

        // a legitimate field that merely contains "key" (sessionKey) is NOT key-like
        assert!(!is_key_like("sessionKey"));
        assert!(!is_key_like("anchorTxid"));
    }
}
