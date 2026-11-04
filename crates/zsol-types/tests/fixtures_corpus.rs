//! Fixtures corpus (FR-1.1). Files live ONLY under tests/ — runtime code never reads them
//! (scripts/no-mocks.sh enforces that). `golden_object.json` is written by the crate itself
//! (`ZSOL_DUMP_FIXTURES=1 cargo test -p zsol-types --test fixtures_corpus`), so committed
//! bytes are canonical by construction; the other files derive from it.

use serde_json::Value;
use zsol_types::{b64u_decode, b64u_encode, AnchorMemo, AttestationObject, ReasonCode, PROTOCOL};

fn dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn golden() -> AttestationObject {
    let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
    let mut o = AttestationObject {
        protocol: PROTOCOL.into(),
        network: "test".into(),
        agent_id: "agent-0001".into(),
        claim_zat: 10_000_000,
        pool: "ironwood".into(),
        session_key: b64u_encode(sk.verifying_key().as_bytes()),
        birthday: 4_544_845,
        issued_at: 1_793_040_000,
        max_age_hours: 24,
        anchor_txid: "ab".repeat(32),
        anchor_height: 4_544_900,
        memo_binding: String::new(),
    };
    o.memo_binding = b64u_encode(&o.expected_memo().unwrap());
    o
}

fn write(name: &str, o: &AttestationObject) {
    std::fs::create_dir_all(dir()).unwrap();
    std::fs::write(dir().join(name), o.canonical().unwrap()).unwrap();
}

fn read(name: &str) -> (Vec<u8>, AttestationObject) {
    let bytes = std::fs::read(dir().join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    (bytes.clone(), serde_json::from_slice(&bytes).unwrap())
}

#[test]
fn dump_fixtures_when_env_set() {
    if std::env::var("ZSOL_DUMP_FIXTURES").as_deref() != Ok("1") {
        return;
    }
    let g = golden();
    write("golden_object.json", &g);
    let mut t = g.clone();
    t.claim_zat += 1; // memoBinding left as-is: binding must now fail
    write("tampered_claim.json", &t);
    let mut d = g.clone();
    d.claim_zat = 9_999;
    d.memo_binding = b64u_encode(&d.expected_memo().unwrap());
    write("dust_claim.json", &d);
}

#[test]
fn fixtures_are_canonical_and_golden_is_stable() {
    for f in [
        "golden_object.json",
        "tampered_claim.json",
        "dust_claim.json",
    ] {
        let (bytes, o) = read(f);
        assert_eq!(o.canonical().unwrap(), bytes, "{f} is not canonical");
    }
    assert_eq!(read("golden_object.json").1, golden());
}

#[test]
fn golden_memo_binds_frame_digest() {
    let (_, o) = read("golden_object.json");
    let memo = AnchorMemo::decode(&b64u_decode(&o.memo_binding).unwrap()).unwrap();
    assert_eq!(memo.digest, o.frame_digest().unwrap());
    assert_eq!(o.validate("test"), Ok(()));
}

#[test]
fn tampered_claim_breaks_binding() {
    let (_, o) = read("tampered_claim.json");
    let memo = AnchorMemo::decode(&b64u_decode(&o.memo_binding).unwrap()).unwrap();
    assert_ne!(memo.digest, o.frame_digest().unwrap());
}

#[test]
fn dust_claim_rejected() {
    let (_, o) = read("dust_claim.json");
    assert_eq!(o.validate("test"), Err(ReasonCode::DustClaim));
    let v: Value = serde_json::from_slice(&o.canonical().unwrap()).unwrap();
    assert_eq!(v["claimZat"], 9_999);
}
