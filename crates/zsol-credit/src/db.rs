//! Underwriter book: loans, events, issued challenges (SQLite, WAL). Stores no keys and no
//! Bitrefill redemption codes (those go to the borrower once, in the open response).

use std::path::Path;

use anyhow::Context as _;
use rusqlite::{Connection, OptionalExtension as _, params};
use serde::Serialize;

pub fn open(path: &Path) -> anyhow::Result<Connection> {
    let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;
         CREATE TABLE IF NOT EXISTS challenges (
            nonce TEXT PRIMARY KEY,
            expires_at INTEGER NOT NULL,
            used INTEGER NOT NULL DEFAULT 0
         );
         CREATE TABLE IF NOT EXISTS loans (
            id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            object_b64u TEXT NOT NULL,
            anchor_txid TEXT NOT NULL,
            status TEXT NOT NULL CHECK (status IN ('open','at-risk','repaid','defaulted')),
            principal_zat INTEGER NOT NULL,
            value_usd REAL NOT NULL,
            usd_per_zec REAL NOT NULL,
            rate_source TEXT NOT NULL,
            rate_at INTEGER NOT NULL,
            product_id TEXT NOT NULL,
            bitrefill_invoice_id TEXT NOT NULL,
            bitrefill_order_id TEXT,
            repay_to TEXT NOT NULL,
            memo_hex TEXT NOT NULL,
            due_height INTEGER NOT NULL,
            opened_height INTEGER NOT NULL,
            repaid_txid TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS loans_agent ON loans(agent_id, status);
         CREATE TABLE IF NOT EXISTS events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            loan_id TEXT,
            kind TEXT NOT NULL,
            height INTEGER,
            detail TEXT NOT NULL,
            at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS enhanced (txid TEXT PRIMARY KEY);",
    )?;
    Ok(conn)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Loan {
    pub id: String,
    pub agent_id: String,
    pub object_b64u: String,
    pub anchor_txid: String,
    pub status: String,
    pub principal_zat: u64,
    pub value_usd: f64,
    pub usd_per_zec: f64,
    pub rate_source: String,
    pub rate_at: u64,
    pub product_id: String,
    pub bitrefill_invoice_id: String,
    pub bitrefill_order_id: Option<String>,
    pub repay_to: String,
    pub memo_hex: String,
    pub due_height: u32,
    pub opened_height: u32,
    pub repaid_txid: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

const LOAN_COLS: &str = "id, agent_id, object_b64u, anchor_txid, status, principal_zat, value_usd,
    usd_per_zec, rate_source, rate_at, product_id, bitrefill_invoice_id, bitrefill_order_id,
    repay_to, memo_hex, due_height, opened_height, repaid_txid, created_at, updated_at";

fn row_to_loan(r: &rusqlite::Row<'_>) -> rusqlite::Result<Loan> {
    Ok(Loan {
        id: r.get(0)?,
        agent_id: r.get(1)?,
        object_b64u: r.get(2)?,
        anchor_txid: r.get(3)?,
        status: r.get(4)?,
        principal_zat: r.get::<_, i64>(5)? as u64,
        value_usd: r.get(6)?,
        usd_per_zec: r.get(7)?,
        rate_source: r.get(8)?,
        rate_at: r.get::<_, i64>(9)? as u64,
        product_id: r.get(10)?,
        bitrefill_invoice_id: r.get(11)?,
        bitrefill_order_id: r.get(12)?,
        repay_to: r.get(13)?,
        memo_hex: r.get(14)?,
        due_height: r.get(15)?,
        opened_height: r.get(16)?,
        repaid_txid: r.get(17)?,
        created_at: r.get::<_, i64>(18)? as u64,
        updated_at: r.get::<_, i64>(19)? as u64,
    })
}

pub fn insert_loan(c: &Connection, l: &Loan) -> anyhow::Result<()> {
    c.execute(
        &format!("INSERT INTO loans ({LOAN_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)"),
        params![
            l.id, l.agent_id, l.object_b64u, l.anchor_txid, l.status, l.principal_zat as i64,
            l.value_usd, l.usd_per_zec, l.rate_source, l.rate_at as i64, l.product_id,
            l.bitrefill_invoice_id, l.bitrefill_order_id, l.repay_to, l.memo_hex, l.due_height,
            l.opened_height, l.repaid_txid, l.created_at as i64, l.updated_at as i64
        ],
    )?;
    Ok(())
}

pub fn get_loan(c: &Connection, id: &str) -> anyhow::Result<Option<Loan>> {
    Ok(c.query_row(
        &format!("SELECT {LOAN_COLS} FROM loans WHERE id = ?1"),
        [id],
        row_to_loan,
    )
    .optional()?)
}

pub fn list_loans(c: &Connection) -> anyhow::Result<Vec<Loan>> {
    let mut s = c.prepare(&format!(
        "SELECT {LOAN_COLS} FROM loans ORDER BY created_at DESC"
    ))?;
    let rows = s.query_map([], row_to_loan)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn active_loans(c: &Connection) -> anyhow::Result<Vec<Loan>> {
    let mut s = c.prepare(&format!(
        "SELECT {LOAN_COLS} FROM loans WHERE status IN ('open','at-risk') ORDER BY created_at"
    ))?;
    let rows = s.query_map([], row_to_loan)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn outstanding_zat(c: &Connection, agent_id: &str) -> anyhow::Result<u64> {
    let v: i64 = c.query_row(
        "SELECT IFNULL(SUM(principal_zat),0) FROM loans WHERE agent_id = ?1 AND status IN ('open','at-risk')",
        [agent_id],
        |r| r.get(0),
    )?;
    Ok(v as u64)
}

pub fn agent_frozen(c: &Connection, agent_id: &str) -> anyhow::Result<bool> {
    let n: i64 = c.query_row(
        "SELECT COUNT(*) FROM loans WHERE agent_id = ?1 AND status IN ('at-risk','defaulted')",
        [agent_id],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

pub fn set_status(
    c: &Connection,
    id: &str,
    status: &str,
    repaid_txid: Option<&str>,
    now: u64,
) -> anyhow::Result<()> {
    c.execute(
        "UPDATE loans SET status = ?2, repaid_txid = COALESCE(?3, repaid_txid), updated_at = ?4 WHERE id = ?1",
        params![id, status, repaid_txid, now as i64],
    )?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: i64,
    pub loan_id: Option<String>,
    pub kind: String,
    pub height: Option<u32>,
    pub detail: serde_json::Value,
    pub at: u64,
}

pub fn add_event(
    c: &Connection,
    loan_id: Option<&str>,
    kind: &str,
    height: Option<u32>,
    detail: &serde_json::Value,
    now: u64,
) -> anyhow::Result<Event> {
    c.execute(
        "INSERT INTO events (loan_id, kind, height, detail, at) VALUES (?1,?2,?3,?4,?5)",
        params![loan_id, kind, height, detail.to_string(), now as i64],
    )?;
    Ok(Event {
        id: c.last_insert_rowid(),
        loan_id: loan_id.map(str::to_string),
        kind: kind.into(),
        height,
        detail: detail.clone(),
        at: now,
    })
}

pub fn recent_events(c: &Connection, limit: u32) -> anyhow::Result<Vec<Event>> {
    let mut s = c.prepare(
        "SELECT id, loan_id, kind, height, detail, at FROM events ORDER BY id DESC LIMIT ?1",
    )?;
    let rows = s.query_map([limit], |r| {
        Ok(Event {
            id: r.get(0)?,
            loan_id: r.get(1)?,
            kind: r.get(2)?,
            height: r.get(3)?,
            detail: serde_json::from_str(&r.get::<_, String>(4)?)
                .unwrap_or(serde_json::Value::Null),
            at: r.get::<_, i64>(5)? as u64,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn store_challenge(c: &Connection, nonce: &str, expires_at: u64) -> anyhow::Result<()> {
    c.execute(
        "INSERT INTO challenges (nonce, expires_at) VALUES (?1, ?2)",
        params![nonce, expires_at as i64],
    )?;
    Ok(())
}

/// Marks a challenge used; false if unknown, expired or already used (replay).
pub fn consume_challenge(c: &Connection, nonce: &str, now: u64) -> anyhow::Result<bool> {
    let n = c.execute(
        "UPDATE challenges SET used = 1 WHERE nonce = ?1 AND used = 0 AND expires_at >= ?2",
        params![nonce, now as i64],
    )?;
    Ok(n == 1)
}

pub fn mark_enhanced(c: &Connection, txid: &str) -> anyhow::Result<bool> {
    Ok(c.execute("INSERT OR IGNORE INTO enhanced (txid) VALUES (?1)", [txid])? == 1)
}
