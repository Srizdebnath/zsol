//! T-42: the merchant pricing policy object is REPO CONFIG with a real parser —
//! a decorative JSON file nobody reads would violate the 0-mock contract.

use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Tier {
    pub discount_bps: u32,
    #[allow(dead_code)]
    pub note: String,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
    pub version: String,
    #[allow(dead_code)]
    pub issuer: String,
    pub tiers: std::collections::BTreeMap<String, Tier>,
    pub stale_after_hours: u64,
    #[allow(dead_code)]
    pub note: String,
}

fn load() -> Policy {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/pricing-policy.json");
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| {
        panic!("cannot read {}: {e} (test must run inside the repo)", p.display())
    }))
    .expect("config/pricing-policy.json must parse")
}

#[test]
fn policy_sane() {
    let pol = load();
    assert_eq!(pol.version, "1");
    assert!(pol.stale_after_hours >= 1);
    let verified = pol.tiers.get("verified").expect("verified tier");
    let claim_only = pol.tiers.get("claim-only").expect("claim-only tier");
    // verified reward ≤ claim-only (never negative-incentivize verification)
    assert!(verified.discount_bps >= claim_only.discount_bps);
    // bps bounds: 0 % ≤ x ≤ 100 %
    for (name, t) in &pol.tiers {
        assert!(t.discount_bps <= 10_000, "tier {name} discount out of range");
    }
    assert_eq!(verified.discount_bps, 100, "demo default: 1 % verified discount");
    assert_eq!(claim_only.discount_bps, 0, "claim-only never surcharges");
}

#[test]
fn stale_after_matches_max_age_default() {
    let pol = load();
    // Pricing staleness must match the attestor default maxAgeHours (24 h) so
    // discounted claims cannot outlive their freshness window by config drift.
    assert_eq!(pol.stale_after_hours, 24);
}
