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
    routing::{get, post},
    Json, Router,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

use zsol_types::{chain_digest, AttestationObject, ReasonCode, ZsolError};

/// Transport-level rejection slugs (distinct from SPEC §9 status codes, which are
/// attestation-verification semantics; §9 codes are reused where the taxonomy has one).
#[derive(Debug, thiserror::Error)]
pub enum RegistryReject {
    #[error("malformed posting: {0}")]
    Malformed(String),
    #[error("non-canonical object bytes")]
    NonCanonical,
    #[error("{0}")]
    Reason(ReasonCode),
    #[error("duplicate (agentId, anchorTxid); existing entry {entry_id}")]
    Duplicate {
        entry_id: String,
        chain_digest_hex: String,
    },
    #[error("registry unavailable: {0}")]
    Db(String),
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
        }
    }
}

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
        "#,
    )
}

fn genesis() -> [u8; 32] {
    [0u8; 32]
}

/// Core publish logic (SPEC §7 acceptance order): decode → parse → canonicalize →
/// validate → dedupe → chain-append. All inside one IMMEDIATE transaction.
pub fn publish(conn: &mut Connection, body: Value, now: u64) -> Result<Published, RegistryReject> {
    let obj_field = body
        .as_object()
        .and_then(|m| m.get("object"))
        .and_then(Value::as_str)
        .ok_or_else(|| RegistryReject::Malformed("body must be {\"object\": \"<b64u>\"}".into()))?;

    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(obj_field)
        .map_err(|e| RegistryReject::Malformed(format!("b64u decode: {e}")))?;

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
    object
        .validate()
        .map_err(|e| match e {
            ZsolError::Claim(rc) => RegistryReject::Reason(rc),
            ZsolError::Json => RegistryReject::Malformed("object json".into()),
            ZsolError::Base64 => RegistryReject::Malformed("object b64u".into()),
            other => RegistryReject::Malformed(format!("{other}")),
        })?;

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
    let rows = stmt
        .query_map(params![agent_id], |r| {
            let obj_bytes: Vec<u8> = r.get(4)?;
            let object: AttestationObject = serde_json::from_slice(&obj_bytes)
                .map_err(|_| rusqlite::Error::FromSqlConversionFailure(
                    4,
                    rusqlite::types::Type::Blob,
                    "stored object is not a valid AttestationObject".into(),
                ))?;
            Ok(FetchEntry {
                seq: r.get(0)?,
                entry_id: r.get(1)?,
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
        entries
            .push(row.map_err(|e| RegistryReject::Db(format!("row: {e}")))?);
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
}

pub fn health(conn: &Connection) -> Result<HealthInfo, RegistryReject> {
    let length: i64 = conn
        .query_row("SELECT length FROM chain_head WHERE id = 1", [], |r| r.get(0))
        .map_err(|e| RegistryReject::Db(format!("health: {e}")))?;
    Ok(HealthInfo {
        ok: true,
        chain_length: length,
    })
}

// ---------------- axum glue (thin; logic lives above) ----------------

fn state_db_guard(state: &AppState) -> Result<std::sync::MutexGuard<'_, Connection>, RegistryReject> {
    state.db.lock().map_err(|_| {
        RegistryReject::Db("connection mutex poisoned (previous panic)".into())
    })
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
            Json(json!({"ok": h.ok, "chainLength": h.chain_length})),
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

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v0/attestations", post(publish_route))
        // axum 0.8 path-param syntax
        .route("/v0/attestations/{agentId}", get(fetch_route))
        .route("/v0/health", get(health_route))
        .with_state(state)
}

// ---------------- tests (SRS FR-3.x ACs) ----------------

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use zsol_types::ANCHOR_MEMO_TAG;

    fn open_mem() -> Connection {
        let mut c = Connection::open_in_memory().unwrap();
        init_db(&mut c).unwrap();
        c
    }

    /// Self-consistent fixture (fixed point via memoBinding exclusion, SPEC §4).
    fn sample(agent: &str, txid: &str, issued_at: u64) -> AttestationObject {
        let mut o = AttestationObject {
            protocol: zsol_types::PROTOCOL.into(),
            agent_id: agent.into(),
            claim_zat: 1_000_000,
            anchor_height: 4_509_856,
            anchor_txid: txid.into(),
            account_ua: "u1test".into(),
            memo_binding: String::new(),
            issued_at,
            max_age_hours: 24,
        };
        let d = o.digest().unwrap();
        let mut memo = vec![];
        memo.extend_from_slice(&ANCHOR_MEMO_TAG);
        memo.push(zsol_types::ANCHOR_MEMO_VER);
        memo.extend_from_slice(&o.anchor_height.to_le_bytes());
        memo.extend_from_slice(&d);
        o.memo_binding = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&memo);
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
        assert!(matches!(err, RegistryReject::Reason(ReasonCode::ProtocolUnknown)));
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
        // rebuild fixed point for the new txid (digest shifts with txid field)
        let d2 = o2.digest().unwrap();
        let mut memo = vec![];
        memo.extend_from_slice(&ANCHOR_MEMO_TAG);
        memo.push(zsol_types::ANCHOR_MEMO_VER);
        memo.extend_from_slice(&o2.anchor_height.to_le_bytes());
        memo.extend_from_slice(&d2);
        o2.memo_binding = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&memo);
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
        assert_eq!(
            hex::encode(prev),
            f.chain_head_hex.unwrap()
        );
    }

    /// FR-1.5 tamper AC + SRS invariant 6: row mutation MUST be physically rejected.
    #[test]
    fn ac_append_only_enforced_by_triggers() {
        let mut c = open_mem();
        let o = sample("agent-1", &"1f".repeat(64).get(..64).unwrap(), 1_769_337_600);
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
    }
}
