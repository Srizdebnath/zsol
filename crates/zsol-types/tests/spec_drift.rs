//! T-16: SPEC drift cross-check (FR-7.2). The serde field names of
//! `AttestationObject` are asserted against the §4 table of the repo's
//! docs/SPEC.md — a field rename that skips the spec (or vice versa) fails CI
//! instead of silently diverging wire shape from documentation. Also pins the
//! frame-digest amendment and the documented memo byte layout (§5).

use serde_json::Value;
use zsol_types::{AttestationObject, PROTOCOL};

fn spec_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/SPEC.md")
}

fn spec_text() -> String {
    let p = spec_path();
    std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("cannot read {}: {e} (test must run inside the repo)", p.display()))
}

/// Real object → serde Value → the EXACT wire field names the code writes.
fn actual_field_names() -> Vec<String> {
    let o = AttestationObject {
        protocol: PROTOCOL.into(),
        agent_id: "x".into(),
        claim_zat: 1,
        anchor_height: 1,
        anchor_txid: "x".into(),
        account_ua: "x".into(),
        memo_binding: "x".into(),
        issued_at: 1,
        max_age_hours: 1,
    };
    let v = serde_json::to_value(&o).unwrap();
    match v {
        Value::Object(m) => m.keys().cloned().collect(),
        _ => panic!("object must serialize as json object"),
    }
}

#[test]
fn spec_section4_lists_every_wire_field() {
    let spec = spec_text();
    let names = actual_field_names();
    assert_eq!(names.len(), 9, "unexpected field count; update SPEC §4 table too");
    for name in &names {
        assert!(
            spec.contains(name),
            "SPEC.md drift: wire field {name:?} missing from docs (T-16 fails the build)"
        );
    }
}

#[test]
fn spec_pins_frame_digest_exclusions_and_memo_layout() {
    let spec = spec_text();
    // §4 amendment: frame digest excludes all three later-known fields.
    for token in ["memoBinding, anchorTxid, anchorHeight"] {
        assert!(
            spec.contains(token),
            "SPEC.md drift: frame-digest exclusion set {token:?} not documented"
        );
    }
    // §5 memo layout: zs01(4)‖ver(1)‖height_LE64(8)‖digest(32)
    for token in ["zs01(4)", "ver(1)", "height_LE64(8)", "digest(32)"] {
        assert!(
            spec.contains(token),
            "SPEC.md drift: memo layout token {token:?} not documented"
        );
    }
}

#[test]
fn spec_lists_all_registry_reject_slugs() {
    let spec = spec_text();
    for slug in ["chain-descended", "rate-limited", "non-canonical", "duplicate"] {
        assert!(
            spec.contains(slug),
            "SPEC.md drift: registry reject slug {slug:?} not documented (FR-3.4/§7)"
        );
    }
}
