//! Attestor side of SPEC §5 (anchor) and §7 (presentation). Session secrets are stored per
//! frame digest under `<wallet>/sessions/` with mode 0600; finished objects under
//! `<wallet>/attestations/`.

use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context as _, anyhow, bail};
use ed25519_dalek::SigningKey;
use serde::Serialize;
use zsol_types::{
    AnchorMemo, AttestationObject, Challenge, PROTOCOL, b64u_encode, encode_session_key,
    now_epoch_secs, sign_presentation, txid_internal, validate_claim_zat,
};

use crate::{Wallet, chain};

pub const ATTEST_ACCOUNT: &str = "zsol-attn";

#[derive(Debug, Clone)]
pub struct AnchorRequest {
    pub agent_id: String,
    pub claim_zat: u64,
    pub max_age_hours: u32,
    pub min_conf: u32,
    /// How long to wait for the anchor to be mined before giving up (it stays pending).
    pub mine_timeout: Duration,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnchorOutcome {
    pub object: AttestationObject,
    pub object_b64u: String,
    pub frame_digest: String,
    pub seconds_to_mine: u64,
}

fn sessions_dir(w: &Wallet) -> PathBuf {
    w.dir.join("sessions")
}

fn attestations_dir(w: &Wallet) -> PathBuf {
    w.dir.join("attestations")
}

fn write_private(path: &PathBuf, bytes: &[u8]) -> anyhow::Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}

fn save_object(w: &Wallet, o: &AttestationObject) -> anyhow::Result<()> {
    let d = hex::encode(o.frame_digest().map_err(|e| anyhow!("{e}"))?);
    std::fs::create_dir_all(attestations_dir(w))?;
    let path = attestations_dir(w).join(format!("{d}.json"));
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(o)?)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

/// All attestations this wallet has produced, newest first.
pub fn list_attestations(w: &Wallet) -> anyhow::Result<Vec<AttestationObject>> {
    let mut out = Vec::new();
    let dir = attestations_dir(w);
    if !dir.exists() {
        return Ok(out);
    }
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.extension().and_then(|s| s.to_str()) == Some("json") {
            out.push(serde_json::from_slice(&std::fs::read(&p)?)?);
        }
    }
    out.sort_by_key(|o: &AttestationObject| std::cmp::Reverse(o.issued_at));
    Ok(out)
}

/// SPEC §5: self-spend from `zsol-attn` to its own Ironwood address carrying `0xFF‖zs02‖D`.
pub async fn anchor(
    w: &mut Wallet,
    client: &mut chain::Client,
    req: &AnchorRequest,
) -> anyhow::Result<AnchorOutcome> {
    validate_claim_zat(req.claim_zat).map_err(|c| anyhow!("claim rejected: {c}"))?;
    if req.agent_id.is_empty() || req.agent_id.len() > 64 {
        bail!("agentId must be 1..=64 characters");
    }
    w.sync(client).await?;
    let bal = w
        .balances(req.min_conf)?
        .into_iter()
        .find(|b| b.account == ATTEST_ACCOUNT)
        .ok_or_else(|| anyhow!("wallet has no {ATTEST_ACCOUNT} account"))?;
    if bal.spendable_zat < req.claim_zat + 20_000 {
        bail!(
            "{ATTEST_ACCOUNT} spendable {} zat < claim {} + fee margin 20000 (total {}, pending {})",
            bal.spendable_zat,
            req.claim_zat,
            bal.total_zat,
            bal.pending_zat
        );
    }

    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret).map_err(|e| anyhow!("rng: {e}"))?;
    let sk = SigningKey::from_bytes(&secret);
    let mut o = AttestationObject {
        protocol: PROTOCOL.into(),
        network: w.meta.network.clone(),
        agent_id: req.agent_id.clone(),
        claim_zat: req.claim_zat,
        pool: "ironwood".into(),
        session_key: encode_session_key(&sk.verifying_key()),
        birthday: w.meta.birthday,
        issued_at: now_epoch_secs(),
        max_age_hours: req.max_age_hours,
        anchor_txid: String::new(),
        anchor_height: 0,
        memo_binding: String::new(),
    };
    let digest = o.frame_digest().map_err(|e| anyhow!("{e}"))?;
    std::fs::create_dir_all(sessions_dir(w))?;
    write_private(
        &sessions_dir(w).join(format!("{}.key", hex::encode(digest))),
        &secret,
    )?;
    let memo = AnchorMemo { digest }.encode_padded();
    let to = w.address(ATTEST_ACCOUNT, true)?;

    let started = std::time::Instant::now();
    let txid = w
        .pay(
            client,
            ATTEST_ACCOUNT,
            &to,
            req.claim_zat,
            Some(&memo),
            req.min_conf,
        )
        .await?;
    o.anchor_txid = txid.clone();
    o.memo_binding = b64u_encode(&memo[..AnchorMemo::LEN]);
    save_object(w, &o)?;
    tracing::info!("anchor broadcast");
    tracing::debug!(frame = %hex::encode(&digest[..6]), "anchor frame");

    let internal = txid_internal(&txid).map_err(|e| anyhow!("{e}"))?;
    let deadline = started + req.mine_timeout;
    loop {
        if let Some(h) = chain::fetch_tx(client, &w.params, internal)
            .await?
            .and_then(|f| f.mined_height)
        {
            o.anchor_height = h;
            break;
        }
        if std::time::Instant::now() > deadline {
            bail!(
                "anchor {txid} broadcast but not mined within {:?}; re-run finalize later",
                req.mine_timeout
            );
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    w.sync(client).await?;
    save_object(w, &o)?;
    Ok(AnchorOutcome {
        object_b64u: o.b64u().map_err(|e| anyhow!("{e}"))?,
        frame_digest: hex::encode(digest),
        seconds_to_mine: started.elapsed().as_secs(),
        object: o,
    })
}

/// Completes objects whose anchor was broadcast but not yet seen mined.
pub async fn finalize_pending(w: &mut Wallet, client: &mut chain::Client) -> anyhow::Result<usize> {
    let mut done = 0;
    for mut o in list_attestations(w)? {
        if o.anchor_height != 0 || o.anchor_txid.is_empty() {
            continue;
        }
        let internal = txid_internal(&o.anchor_txid).map_err(|e| anyhow!("{e}"))?;
        if let Some(h) = chain::fetch_tx(client, &w.params, internal)
            .await?
            .and_then(|f| f.mined_height)
        {
            o.anchor_height = h;
            save_object(w, &o)?;
            done += 1;
        }
    }
    Ok(done)
}

fn load_session(w: &Wallet, o: &AttestationObject) -> anyhow::Result<SigningKey> {
    let d = hex::encode(o.frame_digest().map_err(|e| anyhow!("{e}"))?);
    let bytes = std::fs::read(sessions_dir(w).join(format!("{d}.key")))
        .context("no session secret for this attestation in this wallet")?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow!("corrupt session key"))?;
    let sk = SigningKey::from_bytes(&arr);
    if encode_session_key(&sk.verifying_key()) != o.session_key {
        bail!("stored session secret does not match object.sessionKey");
    }
    Ok(sk)
}

/// SPEC §7 attestor response. Refuses expired challenges and unfinished objects.
pub fn sign_challenge(
    w: &Wallet,
    o: &AttestationObject,
    challenge: &Challenge,
) -> anyhow::Result<String> {
    if o.anchor_height == 0 {
        bail!("attestation anchor not mined yet");
    }
    if challenge.expires_at < now_epoch_secs() {
        bail!("challenge already expired");
    }
    let sk = load_session(w, o)?;
    sign_presentation(&sk, o, challenge).map_err(|e| anyhow!("{e}"))
}
