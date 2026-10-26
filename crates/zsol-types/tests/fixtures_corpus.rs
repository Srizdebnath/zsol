//! T-15: fixtures corpus (FR-7.2 integrity bar). Files live ONLY under tests/ —
//! runtime code must never read these (CI greps enforce that).
//!
//! `golden_object.json` is written by the crate itself (`ZSOL_DUMP_FIXTURES=1`
//! run) so the committed bytes are BY CONSTRUCTION consistent with the serde
//! model + canonical form — no hand-baked digests that could drift.
//! `tampered_claim.json` and `dust_claim.json` are derived from the golden and
//! exercise verifier-visible failure paths.

use base64::Engine as _;
use serde_json::Value;
use zsol_types::{AnchorMemo, AttestationObject, PROTOCOL};

fn fixtures_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn golden() -> AttestationObject {
    let mut o = AttestationObject {
        protocol: PROTOCOL.into(),
        agent_id: "agent-0001".into(),
        claim_zat: 1_000_000, // 0.01 TAZ (SRS §10.3)
        anchor_height: 4_509_856,
        anchor_txid: "0".repeat(64),
        account_ua: "u1test-diversified-ua".into(),
        memo_binding: String::new(),
        issued_at: 1_769_337_600,
        max_age_hours: 24,
    };
    let d = o.digest().unwrap();
    let memo = AnchorMemo { height: o.anchor_height, digest: d };
    o.memo_binding =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(memo.encode());
    o
}

// (the memo construction above is intentionally explicit; see sample() in lib tests)

/// Env-gated writer: `cargo test -p zsol-types --test fixtures_corpus -- --ignored dump 2>/dev/null || ZSOL_DUMP_FIXTURES=1 cargo test`
#[test]
fn dump_fixtures_when_env_set() {
    const DUMP_ENV: &str = "ZSOL_DUMP_FIXTURES";
    if std::env::var(DUMP_ENV).ok().as_deref() != Some("1") {
        eprintln!("skipping dump (set {DUMP_ENV}=1 to regenerate fixtures)");
        return;
    }
    let dir = fixtures_dir();
    std::fs::create_dir_all(&dir).unwrap();

    let g = golden();
    let canon = g.canonical().unwrap();
    std::fs::write(
        dir.join("golden_object.json"),
        serde_json::to_vec(&serde_json::from_slice::<Value>(&canon).unwrap()).unwrap(),
    )
    .unwrap();

    let mut tampered = g.clone();
    tampered.claim_zat += 1;
    let tampered_canon = tampered.canonical().unwrap();
    std::fs::write(
        dir.join("tampered_claim.json"),
        serde_json::to_vec(
            &serde_json::from_slice::<Value>(&tampered_canon).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();

    let mut dust = g.clone();
    dust.claim_zat = 9_999;
    let dust_canon = dust.canonical().unwrap();
    std::fs::write(
        dir.join("dust_claim.json"),
        serde_json::to_vec(&serde_json::from_slice::<Value>(&dust_canon).unwrap())
            .unwrap(),
    )
    .unwrap();

    // Memo-pad tolerance: golden memo bytes right-padded to 60 bytes (wallet pads
    // to word boundaries; SPEC §5 tolerance). Canonical object with the padded
    // memoBinding — decode_prefix still reads the 45-byte prefix.
    let mut padded = g.clone();
    let mut memo_bytes =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(&padded.memo_binding).unwrap();
    let pad = 60usize.saturating_sub(memo_bytes.len());
    memo_bytes.extend(std::iter::repeat(0u8).take(pad));
    padded.memo_binding =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&memo_bytes);
    let padded_canon = padded.canonical().unwrap();
    std::fs::write(
        dir.join("memo_padded.json"),
        serde_json::to_vec(&serde_json::from_slice::<Value>(&padded_canon).unwrap())
            .unwrap(),
    )
    .unwrap();
}

/// Every file in the corpus parses as AttestationObject, and the golden's on-disk
/// bytes are EXACTLY canonical(object) (serde → canonical ⇔ file bytes).
#[test]
fn load_all_fixtures() {
    let dir = fixtures_dir();
    let mut count = 0usize;
    for entry in std::fs::read_dir(&dir).expect("fixtures dir must exist (run dump first)") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        count += 1;
        let bytes = std::fs::read(&path).unwrap();
        let name_wants_valid = path.file_name().unwrap() != "dust_claim.json";
        let o: AttestationObject = serde_json::from_slice(&bytes)
            .unwrap_or_else(|e| panic!("{}: {}", path.display(), e));
        // canonical round-trip immutability (§7 MUST: canonical-only)
        let canon = o.canonical().unwrap();
        assert_eq!(canon, bytes, "file {} is not canonical", path.display());
        if name_wants_valid {
            o.validate().unwrap_or_else(|e| {
                panic!("{}: validate: {e}", path.display());
            });
        } else {
            assert!(matches!(
                o.validate(),
                Err(zsol_types::ZsolError::Claim(zsol_types::ReasonCode::DustClaim))
            ));
        }
    }
    assert_eq!(count, 4, "corpus must be non-empty (golden/tampered/dust/padded)");
}

/// Golden object: memo binds at least one non-nil digest equal to the recomputed
/// frame digest D (the fixture is self-consistent by construction).
#[test]
fn golden_memo_binds_frame_digest() {
    let bytes = std::fs::read(fixtures_dir().join("golden_object.json"))
        .expect("run the fixture dump first (ZSOL_DUMP_FIXTURES=1)");
    let o: AttestationObject = serde_json::from_slice(&bytes).unwrap();
    let memo_bytes =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(&o.memo_binding).unwrap();
    let memo = AnchorMemo::decode_prefix(&memo_bytes).unwrap();
    assert_eq!(memo.digest, o.digest().unwrap());
    assert_eq!(memo.height, o.anchor_height);
}

/// Tampered claim: verifier-visible consistency break — the memo digest does NOT
/// equal the recomputed frame digest of the tampered object (the memo was frozen
/// over claim 1_000_000; the tampered file says 1_000_001).
#[test]
fn tampered_claim_breaks_frame_digest() {
    let bytes = std::fs::read(fixtures_dir().join("tampered_claim.json"))
        .expect("run the fixture dump first (ZSOL_DUMP_FIXTURES=1)");
    let o: AttestationObject = serde_json::from_slice(&bytes).unwrap();
    let memo_bytes =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(&o.memo_binding).unwrap();
    let memo = AnchorMemo::decode_prefix(&memo_bytes).unwrap();
    assert_ne!(memo.digest, o.digest().unwrap());
    // claim moved by exactly +1 zat (the tamper is minimal, not accidental)
    assert_eq!(o.claim_zat, 1_000_001);
}

/// Dust-claim fixture: rejected by validate (§9 dust-claim).
#[test]
fn dust_claim_rejected() {
    let bytes = std::fs::read(fixtures_dir().join("dust_claim.json"))
        .expect("run the fixture dump first (ZSOL_DUMP_FIXTURES=1)");
    let o: AttestationObject = serde_json::from_slice(&bytes).unwrap();
    assert!(matches!(
        o.validate(),
        Err(zsol_types::ZsolError::Claim(zsol_types::ReasonCode::DustClaim))
    ));
}
