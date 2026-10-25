//! `zsol-wallet doctor` — boot-step-0 probe (docs/OPERATIONS.md §1b, SRS FR-7.3).
//! Probes each pinned RPC verb with SAFE params (never executes a send; import probe
//! uses an invalid key so nothing is imported). Prints one JSON line per verb:
//!   {"verb":"<name>","status":"present|missing|param-shape|unreachable|no-auth","detail?"}
//! With `--emit-pins`, also prints a `.env.zallet-pins` skeleton (values only for verbs
//! observed present). Exit code: 0 if all critical verbs present; 1 otherwise; 2 config error.

use std::process::ExitCode;
use zsol_wallet::{Base, Cfg, WalletError};

#[derive(Debug)]
enum VerbState {
    Present,
    Missing,
    ParamShape(String),
    Unreachable(String),
    NoAuth,
}

fn classify(e: &WalletError) -> VerbState {
    match e {
        WalletError::MissingVerb(_) => VerbState::Missing,
        WalletError::Timeout => VerbState::Unreachable("timeout".into()),
        WalletError::Auth(_) => VerbState::NoAuth,
        WalletError::Transport(t) => VerbState::Unreachable(t.clone()),
        WalletError::Rpc { code, message } => {
            // param-validation errors PROVE the verb exists (bad input ≠ absent method)
            if matches!(code, -8 | -5 | -3 | -32602) {
                VerbState::ParamShape(message.clone())
            } else {
                VerbState::Unreachable(format!("rpc {code}: {message}"))
            }
        }
        WalletError::EnvMissing(k) => VerbState::Unreachable(format!("env {k}")),
    }
}

async fn probe(
    label: &'static str,
    fut: impl std::future::Future<Output = Result<serde_json::Value, WalletError>>,
    out: &mut Vec<(&'static str, VerbState)>,
) {
    let st = match fut.await {
        Ok(_) => VerbState::Present,
        Err(e) => classify(&e),
    };
    out.push((label, st));
}

#[tokio::main]
async fn main() -> ExitCode {
    let cfg = match Cfg::from_env() {
        Ok(c) => c,
        Err(e) => {
            println!(r#"{{"doctor":"config-error","error":"{e}"}}"#);
            return ExitCode::from(2);
        }
    };
    let client = match zsol_wallet::Client::new(cfg.clone()) {
        Ok(c) => c,
        Err(e) => {
            println!(r#"{{"doctor":"client-error","error":"{e}"}}"#);
            return ExitCode::from(2);
        }
    };

    let zebra_base = Base::Zebra;

    let mut results: Vec<(&'static str, VerbState)> = Vec::new();

    // safe probes only — never executes a spend; view/import probes use obviously-invalid keys
    probe("getblockchaininfo", client.info(), &mut results).await;
    probe(
        "z_listaccounts",
        client.call(Base::Zallet, &cfg.methods.list_accounts, serde_json::json!([])),
        &mut results,
    )
    .await;
    probe(
        "z_getaddressforaccount",
        client.addr_for_account(cfg.attest_account, &["orchard", "sapling"]),
        &mut results,
    )
    .await;
    let balance_state = match client.balance().await {
        Ok(_) => VerbState::Present,
        Err(e) => classify(&e),
    };
    results.push(("balance-family", balance_state));
    probe(
        "z_sendmany-params-probe",
        client.call(Base::Zallet, &cfg.methods.send, serde_json::json!([null])),
        &mut results,
    )
    .await;
    probe(
        "z_getoperationstatus",
        client.op_status(),
        &mut results,
    )
    .await;
    probe(
        "z_getoperationresult",
        client.op_result("nonexistent-op-probe"),
        &mut results,
    )
    .await;
    probe(
        "z_viewtransaction",
        client.view_tx(&"00".repeat(32)),
        &mut results,
    )
    .await;
    probe(
        "z_importkey",
        client.import_key("zsol-doctor-invalid-key-probe"),
        &mut results,
    )
    .await;
    probe(
        "getblock-0",
        client.call(zebra_base, &cfg.methods.get_block, serde_json::json!([0])),
        &mut results,
    )
    .await;

    let emit_pins = std::env::args().any(|a| a == "--emit-pins");

    let mut any_missing = false;
    for (label, st) in &results {
        let status = match st {
            VerbState::Present => "present",
            VerbState::Missing => "missing",
            VerbState::ParamShape(d) => {
                eprintln!("param-shape detail [{label}]: {d}");
                "param-shape (verb exists; check arg shape)"
            }
            VerbState::Unreachable(d) => {
                any_missing = true;
                eprintln!("unreachable detail [{label}]: {d}");
                "unreachable"
            }
            VerbState::NoAuth => "no-auth",
        };
        println!(r#"{{"verb":"{label}","status":"{status}"}}"#);
        if matches!(st, VerbState::Missing) {
            any_missing = true;
        }
    }

    if emit_pins {
        let present = |p: &str| -> bool {
            results.iter().any(|(lbl, st)| *lbl == p && matches!(st, VerbState::Present | VerbState::ParamShape(_)))
        };
        println!("--- suggested .env.zallet-pins (fill from real probes) ---");
        for m in [
            ("ZSOL_RPC_METHOD_INFO", "getblockchaininfo", "getblockchaininfo"),
            ("ZSOL_RPC_METHOD_LISTACCOUNTS", "z_listaccounts", "z_listaccounts"),
            ("ZSOL_RPC_METHOD_ADDRFORACCT", "z_getaddressforaccount", "z_getaddressforaccount"),
            ("ZSOL_RPC_METHOD_SEND", "z_sendmany", "z_sendmany"),
            ("ZSOL_RPC_METHOD_OPSTATUS", "z_getoperationstatus", "z_getoperationstatus"),
            ("ZSOL_RPC_METHOD_OPRESULT", "z_getoperationresult", "z_getoperationresult"),
            ("ZSOL_RPC_METHOD_VIEWTX", "z_viewtransaction", "z_viewtransaction"),
            ("ZSOL_RPC_METHOD_IMPORTKEY", "z_importkey", "z_importkey"),
            ("ZSOL_RPC_METHOD_GETBLOCK", "getblock", "getblock"),
        ] {
            if present(m.2) {
                println!("{0}={0}", m.0);
            } else {
                println!("# {} MISSING — pin an override or degrade (T-04 rule)", m.0);
            }
        }
        println!("# balance family resolution:");
        if present("balance-family") {
            println!("# balance family: at least one pinned method served a value");
        }
    }

    if any_missing {
        ExitCode::from(1)
    } else {
        ExitCode::from(0)
    }
}
