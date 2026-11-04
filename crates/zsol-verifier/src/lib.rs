//! SPEC §6 verification engine (C1–C8) and §8 solvency watch, run entirely on the verifier's
//! own view-only wallet. Every fact is recomputed from chain data (SPEC P5).

use anyhow::{Context as _, anyhow};
use serde::Serialize;
use zsol_lightwallet::{Wallet, chain};
use zsol_types::{
    AnchorMemo, AttestationObject, Challenge, CheckRow, ReasonCode, Tier, b64u_decode,
    now_epoch_secs, txid_internal, verify_presentation,
};

/// zcash_client_sqlite pool codes in `v_received_outputs.pool`.
const POOL_SAPLING: i64 = 2;
const POOL_IRONWOOD: i64 = 4;

fn pool_code(pool: &str) -> Option<i64> {
    match pool {
        "sapling" => Some(POOL_SAPLING),
        "ironwood" => Some(POOL_IRONWOOD),
        _ => None,
    }
}

/// What the attestor hands the verifier over the direct channel (never via the registry).
#[derive(Debug, Clone, serde::Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Presentation {
    pub object: AttestationObject,
    pub ufvk: String,
    pub challenge: Challenge,
    /// b64u Ed25519 signature (SPEC §7).
    pub signature: String,
}

/// One anchored note as the verifier sees it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnchoredNote {
    pub pool: &'static str,
    pub output_index: i64,
    pub value_zat: u64,
    pub binding: bool,
    pub spent_in_txid: Option<String>,
    pub spent_height: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub tier: Tier,
    pub reasons: Vec<ReasonCode>,
    pub checks: Vec<CheckRow>,
    pub agent_id: String,
    pub claim_zat: u64,
    pub anchor_txid: String,
    pub anchored_zat: u64,
    pub notes: Vec<AnchoredNote>,
    pub tip_height: u32,
    pub scanned_height: Option<u32>,
    pub verified_at: u64,
}

impl Report {
    pub fn is_verified(&self) -> bool {
        self.tier == Tier::Verified
    }
}

struct Rows(Vec<CheckRow>);

impl Rows {
    fn push(&mut self, check: &str, res: Result<String, (ReasonCode, String)>) -> bool {
        let (ok, code, detail) = match res {
            Ok(d) => (true, ReasonCode::Ok, d),
            Err((c, d)) => (false, c, d),
        };
        self.0.push(CheckRow {
            check: check.into(),
            ok,
            code,
            detail,
        });
        ok
    }
}

/// The anchor as found in the verifier's view-only account.
struct AnchorView {
    mined_height: Option<u32>,
    spent_note_count: i64,
    notes: Vec<AnchoredNote>,
    memos: Vec<Option<Vec<u8>>>,
}

fn read_anchor(
    wallet: &Wallet,
    account: uuid::Uuid,
    txid: [u8; 32],
) -> anyhow::Result<Option<AnchorView>> {
    let conn = wallet.views()?;
    let acct = account.as_bytes().to_vec();
    let tx_row: Option<(Option<u32>, i64)> = conn
        .query_row(
            "SELECT mined_height, IFNULL(spent_note_count, 0) FROM v_transactions
              WHERE txid = ?1 AND account_uuid = ?2",
            rusqlite::params![txid.to_vec(), acct],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(e),
        })?;
    let Some((mined_height, spent_note_count)) = tx_row else {
        return Ok(None);
    };
    let mut stmt = conn.prepare(
        "SELECT ro.pool, ro.output_index, ro.value, ro.memo,
                (SELECT hex(st.txid) FROM v_received_output_spends s
                   JOIN transactions st ON st.id_tx = s.transaction_id
                  WHERE s.pool = ro.pool AND s.received_output_id = ro.id_within_pool_table
                  ORDER BY st.mined_height LIMIT 1),
                (SELECT st.mined_height FROM v_received_output_spends s
                   JOIN transactions st ON st.id_tx = s.transaction_id
                  WHERE s.pool = ro.pool AND s.received_output_id = ro.id_within_pool_table
                  ORDER BY st.mined_height LIMIT 1)
           FROM v_received_outputs ro
           JOIN transactions t ON t.id_tx = ro.transaction_id
           JOIN accounts a ON a.id = ro.account_id
          WHERE t.txid = ?1 AND a.uuid = ?2
          ORDER BY ro.pool, ro.output_index",
    )?;
    let mut notes = Vec::new();
    let mut memos = Vec::new();
    let rows = stmt.query_map(rusqlite::params![txid.to_vec(), acct], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, Option<Vec<u8>>>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<u32>>(5)?,
        ))
    })?;
    for row in rows {
        let (pool, idx, value, memo, spent_hex, spent_h) = row?;
        notes.push(AnchoredNote {
            pool: match pool {
                POOL_SAPLING => "sapling",
                POOL_IRONWOOD => "ironwood",
                3 => "orchard",
                _ => "transparent",
            },
            output_index: idx,
            value_zat: u64::try_from(value).unwrap_or(0),
            binding: false,
            // `transactions.txid` is stored in internal order; present display order.
            spent_in_txid: spent_hex.map(|h| {
                let mut b = hex::decode(h).unwrap_or_default();
                b.reverse();
                hex::encode(b)
            }),
            spent_height: spent_h,
        });
        memos.push(memo);
    }
    Ok(Some(AnchorView {
        mined_height,
        spent_note_count,
        notes,
        memos,
    }))
}

pub struct VerifierConfig {
    pub network: String,
    pub verifier_id: String,
    pub min_conf: u32,
}

/// Runs C1–C8 for `p` against the verifier's own wallet + chain view.
/// The verifier must already have issued `p.challenge` (it checks `verifierId` matches).
pub async fn verify(
    wallet: &mut Wallet,
    client: &mut chain::Client,
    cfg: &VerifierConfig,
    p: &Presentation,
) -> anyhow::Result<Report> {
    let o = &p.object;
    let now = now_epoch_secs();
    let mut rows = Rows(Vec::new());

    // C1 — structure, protocol, network, dust.
    let c1 = o.validate(&cfg.network);
    rows.push(
        "C1 protocol",
        c1.map(|_| format!("{} on {} pool {}", o.protocol, o.network, o.pool))
            .map_err(|c| (c, "object failed structural validation".into())),
    );

    // C8 early (pure crypto, independent of chain state).
    let c8 = if p.challenge.verifier_id != cfg.verifier_id {
        Err((
            ReasonCode::PresentationInvalid,
            "challenge was issued to a different verifier".into(),
        ))
    } else {
        verify_presentation(o, &p.challenge, &p.signature, now)
            .map(|_| "Ed25519 signature by sessionKey over this verifier's nonce".to_string())
            .map_err(|c| (c, "signature does not verify for this challenge".into()))
    };

    let txid = txid_internal(&o.anchor_txid).map_err(|_| anyhow!("bad txid"))?;
    let digest = o.frame_digest().map_err(|e| anyhow!("{e}"))?;
    let expected_memo = AnchorMemo { digest }.encode();
    let short = hex::encode(&digest[..6]);
    let account_name = format!("peer-{}-{short}", o.agent_id);

    // Import the UFVK view-only and scan from the declared birthday.
    let import = wallet
        .import_view_only(client, &account_name, &p.ufvk, o.birthday)
        .await;
    let account = match import {
        Ok(a) => a.expose_uuid(),
        Err(e) => {
            rows.push(
                "C3 spend authority",
                Err((ReasonCode::UfvkInvalid, e.to_string())),
            );
            rows.push("C8 presentation", c8);
            return Ok(finish(o, rows, 0, vec![], 0, None, now));
        }
    };
    // Blocks keep arriving while we scan (25 s spacing), so re-sync until caught up; a lag of
    // at most one block is tolerated (ARCHITECTURE §5), anything more is scan-incomplete.
    let mut sync = wallet
        .sync(client)
        .await
        .context("syncing verifier wallet")?;
    for _ in 0..3 {
        if sync
            .fully_scanned_height
            .is_some_and(|h| h >= sync.tip_height)
        {
            break;
        }
        sync = wallet
            .sync(client)
            .await
            .context("syncing verifier wallet")?;
    }
    let tip = sync.tip_height;
    let scan_lag = sync
        .fully_scanned_height
        .map_or(u32::MAX, |h| tip.saturating_sub(h));
    let mined = wallet.enhance(client, txid).await?;

    // C2 — mined at the claimed height with enough depth.
    let c2 = match mined {
        None => Err((
            ReasonCode::AnchorNotMined,
            "endpoint does not know this txid".into(),
        )),
        Some(None) => Err((
            ReasonCode::AnchorNotMined,
            "transaction is still in the mempool".into(),
        )),
        Some(Some(h)) if h != o.anchor_height => Err((
            ReasonCode::AnchorNotMined,
            format!("mined at {h}, object claims {}", o.anchor_height),
        )),
        Some(Some(h)) => {
            let depth = tip.saturating_sub(h) + 1;
            if depth >= cfg.min_conf {
                Ok(format!("mined at {h}, {depth} confirmations"))
            } else {
                Err((
                    ReasonCode::AnchorNotMined,
                    format!("{depth} < {} confirmations", cfg.min_conf),
                ))
            }
        }
    };
    rows.push("C2 anchor mined", c2);

    let view = read_anchor(wallet, account, txid)?;
    let (mut notes, memos, spent_note_count) = match view {
        Some(v) => {
            let _ = v.mined_height;
            (v.notes, v.memos, v.spent_note_count)
        }
        None => (vec![], vec![], 0),
    };

    // C3 — the anchor consumed this account's own notes (consensus spend authorization).
    rows.push(
        "C3 spend authority",
        if spent_note_count > 0 {
            Ok(format!(
                "anchor spends {spent_note_count} note(s) owned by the shared account"
            ))
        } else {
            Err((
                ReasonCode::NotSpentByAccount,
                "anchor spends no note of the shared account".into(),
            ))
        },
    );

    // C4 — exactly one output carries 0xFF‖zs02‖D(object), byte-equal to memoBinding.
    let presented = b64u_decode(&o.memo_binding).unwrap_or_default();
    let mut binding_hits = 0;
    for (n, m) in notes.iter_mut().zip(memos.iter()) {
        if let Some(m) = m
            && m.len() >= AnchorMemo::LEN
            && m[..AnchorMemo::LEN] == expected_memo
        {
            n.binding = true;
            binding_hits += 1;
        }
    }
    rows.push(
        "C4 binding",
        if binding_hits == 1 && presented == expected_memo {
            Ok(format!(
                "memo 0xFF‖zs02‖{}… matches the recomputed frame digest",
                hex::encode(&digest[..4])
            ))
        } else if binding_hits == 0 {
            Err((
                ReasonCode::MemoDigestMismatch,
                "no anchor output carries this object's digest".into(),
            ))
        } else if binding_hits > 1 {
            Err((
                ReasonCode::MemoDigestMismatch,
                format!("{binding_hits} outputs carry the digest (must be exactly 1)"),
            ))
        } else {
            Err((
                ReasonCode::MemoDigestMismatch,
                "memoBinding differs from the on-chain memo".into(),
            ))
        },
    );

    // C5 — anchored value in the declared pool covers the claim.
    let want_pool = pool_code(&o.pool);
    let anchored: u64 = notes
        .iter()
        .filter(|n| want_pool.is_some_and(|p| pool_code(n.pool) == Some(p)))
        .map(|n| n.value_zat)
        .sum();
    rows.push(
        "C5 value",
        if anchored >= o.claim_zat {
            Ok(format!("{anchored} zat anchored ≥ {} claimed", o.claim_zat))
        } else {
            Err((
                ReasonCode::ClaimExceedsOutputs,
                format!("{anchored} zat anchored < {} claimed", o.claim_zat),
            ))
        },
    );

    // C6 — no anchored note has been spent up to our synced tip.
    let spent: Vec<&AnchoredNote> = notes.iter().filter(|n| n.spent_in_txid.is_some()).collect();
    {
        let scanned = sync.fully_scanned_height;
        rows.push(
            "C6 liveness",
            if scan_lag > 1 {
                Err((
                    ReasonCode::ScanIncomplete,
                    format!("scanned to {scanned:?}, tip {tip}"),
                ))
            } else if notes.is_empty() {
                Err((
                    ReasonCode::AnchoredNoteSpent,
                    "no anchored notes visible".into(),
                ))
            } else if spent.is_empty() {
                Ok(format!(
                    "{} anchored note(s) unspent at height {tip}",
                    notes.len()
                ))
            } else {
                Err((
                    ReasonCode::AnchoredNoteSpent,
                    format!(
                        "spent in {} at height {:?}",
                        spent[0].spent_in_txid.as_deref().unwrap_or("?"),
                        spent[0].spent_height
                    ),
                ))
            },
        );
    }

    // C7 — freshness.
    let max_age = u64::from(o.max_age_hours) * 3600;
    let c7 = if now.saturating_sub(o.issued_at) > max_age {
        Err((
            ReasonCode::StaleAttestation,
            format!("issued {}s ago > {max_age}s", now - o.issued_at),
        ))
    } else {
        match chain::block_time(client, o.anchor_height).await {
            Ok(bt) if o.issued_at > u64::from(bt) + 7200 => Err((
                ReasonCode::StaleAttestation,
                "issuedAt is more than 2h after the anchor block".into(),
            )),
            Ok(_) => Ok(format!(
                "issued {}s ago, horizon {}h",
                now.saturating_sub(o.issued_at),
                o.max_age_hours
            )),
            Err(e) => Err((ReasonCode::ChainUnreachable, e.to_string())),
        }
    };
    rows.push("C7 freshness", c7);
    rows.push("C8 presentation", c8);

    Ok(finish(
        o,
        rows,
        anchored,
        notes,
        tip,
        sync.fully_scanned_height,
        now,
    ))
}

fn finish(
    o: &AttestationObject,
    rows: Rows,
    anchored: u64,
    notes: Vec<AnchoredNote>,
    tip: u32,
    scanned: Option<u32>,
    now: u64,
) -> Report {
    let reasons: Vec<ReasonCode> = rows.0.iter().filter(|r| !r.ok).map(|r| r.code).collect();
    Report {
        tier: if reasons.is_empty() {
            Tier::Verified
        } else {
            Tier::ClaimOnly
        },
        reasons,
        checks: rows.0,
        agent_id: o.agent_id.clone(),
        claim_zat: o.claim_zat,
        anchor_txid: o.anchor_txid.clone(),
        anchored_zat: anchored,
        notes,
        tip_height: tip,
        scanned_height: scanned,
        verified_at: now,
    }
}

/// SPEC §8 watch status for an already-imported attestation.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case", tag = "state")]
pub enum WatchState {
    Live { height: u32 },
    Released { height: u32, spent_in_txid: String },
    Expired,
}

/// Re-syncs and re-evaluates C6/C7 only (SPEC §8). Cheap: one incremental sync.
pub async fn watch_tick(
    wallet: &mut Wallet,
    client: &mut chain::Client,
    o: &AttestationObject,
) -> anyhow::Result<WatchState> {
    let digest = o.frame_digest().map_err(|e| anyhow!("{e}"))?;
    let account_name = format!("peer-{}-{}", o.agent_id, hex::encode(&digest[..6]));
    let account = wallet.account(&account_name)?.expose_uuid();
    let sync = wallet.sync(client).await?;
    let max_age = u64::from(o.max_age_hours) * 3600;
    if now_epoch_secs().saturating_sub(o.issued_at) > max_age {
        return Ok(WatchState::Expired);
    }
    let txid = txid_internal(&o.anchor_txid).map_err(|_| anyhow!("bad txid"))?;
    let view = read_anchor(wallet, account, txid)?
        .ok_or_else(|| anyhow!("anchor no longer visible (reorg?)"))?;
    if let Some(n) = view.notes.iter().find(|n| n.spent_in_txid.is_some()) {
        return Ok(WatchState::Released {
            height: n.spent_height.unwrap_or(sync.tip_height),
            spent_in_txid: n.spent_in_txid.clone().unwrap_or_default(),
        });
    }
    Ok(WatchState::Live {
        height: sync.tip_height,
    })
}
