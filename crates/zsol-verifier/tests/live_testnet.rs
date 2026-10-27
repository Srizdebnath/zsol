//! SRS FR-3.2 / gate G3: adversarial cases reproduced LIVE on Zcash testnet. These tests
//! broadcast real transactions, so they are `#[ignore]`d and run only on request:
//!
//!   ZSOL_LIVE_ATTESTOR=~/.zsol/borrower-a ZSOL_LIVE_ADVERSARY=~/.zsol/borrower-b \
//!   cargo test --release -p zsol-verifier --test live_testnet -- --ignored --nocapture --test-threads 1
//!
//! The attestor wallet must hold a mined attestation (`zsol attest anchor`); the adversary
//! wallet needs ≥ 0.001 TAZ in both `zsol-attn` and `commerce`.

use std::path::PathBuf;
use std::time::Duration;

use ed25519_dalek::SigningKey;
use zcash_keys::keys::{UnifiedAddressRequest, UnifiedFullViewingKey};
use zsol_lightwallet::{Wallet, attest, chain};
use zsol_types::{
    AnchorMemo, AttestationObject, Challenge, PROTOCOL, ReasonCode, Tier, b64u_encode,
    encode_session_key, now_epoch_secs, sign_presentation,
};
use zsol_verifier::{Presentation, Report, VerifierConfig};

fn dir(var: &str) -> PathBuf {
    let raw = std::env::var(var).unwrap_or_else(|_| panic!("{var} must point at a wallet dir"));
    PathBuf::from(raw.replace('~', &std::env::var("HOME").unwrap()))
}

async fn client() -> (chain::Client, zcash_protocol::consensus::Network) {
    let params = zcash_protocol::consensus::Network::TestNetwork;
    let ep = std::env::var("ZSOL_LWD").unwrap_or_else(|_| chain::DEFAULT_TESTNET_ENDPOINT.into());
    let mut c = chain::connect(&ep).await.unwrap();
    chain::boot_check(&mut c, &params, &ep).await.unwrap();
    (c, params)
}

fn challenge(verifier_id: &str) -> Challenge {
    let mut n = [0u8; 32];
    getrandom::fill(&mut n).unwrap();
    Challenge {
        nonce: b64u_encode(&n),
        verifier_id: verifier_id.into(),
        expires_at: now_epoch_secs() + 600,
    }
}

/// A fresh, empty verifier wallet per case so no state leaks between cases.
async fn verify_fresh(p: &Presentation, verifier_id: &str) -> Report {
    let (mut c, params) = client().await;
    let d = std::env::temp_dir().join(format!(
        "zsol-live-verifier-{}-{}",
        verifier_id,
        now_epoch_secs()
    ));
    let mut w = Wallet::create_view_only(&d, params, p.object.birthday).unwrap();
    let cfg = VerifierConfig {
        network: "test".into(),
        verifier_id: verifier_id.into(),
        min_conf: 1,
    };
    let r = zsol_verifier::verify(&mut w, &mut c, &cfg, p)
        .await
        .unwrap();
    for row in &r.checks {
        println!(
            "    {} {:<20} {:<24} {}",
            if row.ok { "PASS" } else { "FAIL" },
            row.check,
            row.code.as_str(),
            row.detail
        );
    }
    std::fs::remove_dir_all(&d).ok();
    r
}

fn latest(w: &Wallet) -> AttestationObject {
    attest::list_attestations(w)
        .unwrap()
        .into_iter()
        .find(|o| o.anchor_height != 0)
        .expect("a mined attestation")
}

fn session_secret(w: &Wallet, o: &AttestationObject) -> SigningKey {
    let d = hex::encode(o.frame_digest().unwrap());
    let b = std::fs::read(w.dir.join("sessions").join(format!("{d}.key"))).unwrap();
    SigningKey::from_bytes(&b.try_into().unwrap())
}

async fn wait_mined(
    c: &mut chain::Client,
    params: &zcash_protocol::consensus::Network,
    txid: &str,
) -> u32 {
    let internal = zsol_types::txid_internal(txid).unwrap();
    for _ in 0..120 {
        if let Some(h) = chain::fetch_tx(c, params, internal)
            .await
            .unwrap()
            .and_then(|f| f.mined_height)
        {
            return h;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    panic!("{txid} not mined in 10 minutes");
}

fn only_failure(r: &Report, code: ReasonCode) {
    assert_eq!(r.tier, Tier::ClaimOnly);
    assert_eq!(
        r.reasons,
        vec![code],
        "expected exactly [{code}] to fail, got {:?}",
        r.reasons
    );
}

#[tokio::test]
#[ignore = "live testnet"]
async fn g3a_edited_agent_id_fails_binding() {
    let w = Wallet::open(&dir("ZSOL_LIVE_ATTESTOR")).unwrap();
    let mut o = latest(&w);
    let sk = session_secret(&w, &o);
    o.agent_id = format!("{}-evil", o.agent_id);
    let ch = challenge("g3-verifier");
    let p = Presentation {
        signature: sign_presentation(&sk, &o, &ch).unwrap(),
        ufvk: w.export_ufvk(attest::ATTEST_ACCOUNT).unwrap(),
        object: o,
        challenge: ch,
    };
    println!("G3a edited agentId (re-signed by the genuine session key):");
    only_failure(
        &verify_fresh(&p, "g3-verifier").await,
        ReasonCode::MemoDigestMismatch,
    );
}

#[tokio::test]
#[ignore = "live testnet"]
async fn g3b_relayed_presentation_fails() {
    let w = Wallet::open(&dir("ZSOL_LIVE_ATTESTOR")).unwrap();
    let o = latest(&w);
    let ch_v1 = challenge("verifier-1");
    let genuine = Presentation {
        signature: attest::sign_challenge(&w, &o, &ch_v1).unwrap(),
        ufvk: w.export_ufvk(attest::ATTEST_ACCOUNT).unwrap(),
        object: o.clone(),
        challenge: ch_v1,
    };
    println!("G3b-1 V1 forwards A's presentation (made for V1) to V2:");
    only_failure(
        &verify_fresh(&genuine, "verifier-2").await,
        ReasonCode::PresentationInvalid,
    );
    println!("G3b-2 V1 swaps in V2's fresh challenge but cannot re-sign:");
    let mut swapped = genuine.clone();
    swapped.challenge = challenge("verifier-2");
    only_failure(
        &verify_fresh(&swapped, "verifier-2").await,
        ReasonCode::PresentationInvalid,
    );
}

#[tokio::test]
#[ignore = "live testnet"]
async fn g3c_overclaim_fails_value() {
    let (mut c, params) = client().await;
    let mut w = Wallet::open(&dir("ZSOL_LIVE_ADVERSARY")).unwrap();
    w.sync(&mut c).await.unwrap();
    let sk = SigningKey::from_bytes(&[0x42; 32]);
    let mut o = AttestationObject {
        protocol: PROTOCOL.into(),
        network: "test".into(),
        agent_id: "agent-b-overclaim".into(),
        claim_zat: 5_000_000_000, // 50 TAZ, far above what B holds
        pool: "ironwood".into(),
        session_key: encode_session_key(&sk.verifying_key()),
        birthday: w.meta.birthday,
        issued_at: now_epoch_secs(),
        max_age_hours: 24,
        anchor_txid: String::new(),
        anchor_height: 0,
        memo_binding: String::new(),
    };
    let memo = AnchorMemo {
        digest: o.frame_digest().unwrap(),
    }
    .encode_padded();
    let to = w.address(attest::ATTEST_ACCOUNT, true).unwrap();
    let txid = w
        .pay(&mut c, attest::ATTEST_ACCOUNT, &to, 20_000, Some(&memo), 1)
        .await
        .unwrap();
    o.anchor_height = wait_mined(&mut c, &params, &txid).await;
    o.anchor_txid = txid.clone();
    o.memo_binding = b64u_encode(&memo[..AnchorMemo::LEN]);
    println!("G3c over-claim anchor {txid} at {}:", o.anchor_height);
    let ch = challenge("g3-verifier");
    let p = Presentation {
        signature: sign_presentation(&sk, &o, &ch).unwrap(),
        ufvk: w.export_ufvk(attest::ATTEST_ACCOUNT).unwrap(),
        object: o,
        challenge: ch,
    };
    only_failure(
        &verify_fresh(&p, "g3-verifier").await,
        ReasonCode::ClaimExceedsOutputs,
    );
}

#[tokio::test]
#[ignore = "live testnet"]
async fn g3d_donation_forgery_fails_spend_authority() {
    let (mut c, params) = client().await;
    let victim = Wallet::open(&dir("ZSOL_LIVE_ATTESTOR")).unwrap();
    let victim_ufvk = victim.export_ufvk(attest::ATTEST_ACCOUNT).unwrap();
    // The attacker only has what any past verifier has: the victim's UFVK.
    let ufvk = UnifiedFullViewingKey::decode(&params, &victim_ufvk).unwrap();
    let (victim_addr, _) = ufvk
        .default_address(UnifiedAddressRequest::ORCHARD)
        .unwrap();
    let mut attacker = Wallet::open(&dir("ZSOL_LIVE_ADVERSARY")).unwrap();
    attacker.sync(&mut c).await.unwrap();
    let sk = SigningKey::from_bytes(&[0x66; 32]);
    let mut o = AttestationObject {
        protocol: PROTOCOL.into(),
        network: "test".into(),
        agent_id: "mallory".into(),
        claim_zat: 20_000,
        pool: "ironwood".into(),
        session_key: encode_session_key(&sk.verifying_key()),
        birthday: victim.meta.birthday,
        issued_at: now_epoch_secs(),
        max_age_hours: 24,
        anchor_txid: String::new(),
        anchor_height: 0,
        memo_binding: String::new(),
    };
    let memo = AnchorMemo {
        digest: o.frame_digest().unwrap(),
    }
    .encode_padded();
    let to = victim_addr.encode(&params);
    let txid = attacker
        .pay(&mut c, "commerce", &to, 20_000, Some(&memo), 1)
        .await
        .unwrap();
    o.anchor_height = wait_mined(&mut c, &params, &txid).await;
    o.anchor_txid = txid.clone();
    o.memo_binding = b64u_encode(&memo[..AnchorMemo::LEN]);
    println!(
        "G3d donation {txid} at {} (attacker funds → victim zsol-attn, attacker's memo):",
        o.anchor_height
    );
    let ch = challenge("g3-verifier");
    let p = Presentation {
        signature: sign_presentation(&sk, &o, &ch).unwrap(),
        ufvk: victim_ufvk,
        object: o,
        challenge: ch,
    };
    only_failure(
        &verify_fresh(&p, "g3-verifier").await,
        ReasonCode::NotSpentByAccount,
    );
}
