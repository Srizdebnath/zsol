//! The Credit Desk policy (SPEC §11, SRS FR-6.2) is repo config with a real consumer
//! (zsol-credit). This test keeps it within protocol bounds.

use serde_json::Value;

fn load() -> Value {
    let p =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/credit-policy.json");
    serde_json::from_slice(&std::fs::read(&p).unwrap()).expect("credit-policy.json must parse")
}

#[test]
fn credit_policy_within_bounds() {
    let p = load();
    let ltv = p["ltvBps"].as_u64().unwrap();
    assert!(ltv > 0 && ltv <= 10_000, "ltvBps must be in (0, 10000]");
    assert!(p["maxLoanUsd"].as_f64().unwrap() > 0.0);
    assert!(p["maxTenorBlocks"].as_u64().unwrap() > 0);
    assert!(p["minConf"].as_u64().unwrap() >= 1);
    assert!(!p["underwriterId"].as_str().unwrap().is_empty());
}

#[test]
fn test_mode_only_allows_bitrefill_test_products() {
    let p = load();
    if p["mode"] == "test" {
        for prod in p["allowedProducts"].as_array().unwrap() {
            assert!(
                prod.as_str().unwrap().starts_with("test-"),
                "{prod} is not a Bitrefill test product"
            );
        }
    }
}
