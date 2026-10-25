//! zsol-wallet — JSON-RPC 1.0 clients for Zallet + Zebra, with the boot-step-0 method
//! "pin table" (docs/OPERATIONS.md §1b). Honesty rule (SRS FR-2.1): this crate never
//! assumes a verb exists — `zsol-wallet doctor` prints reality, Cfg carries the pins.
//! No PII in errors: all address-bearing text stays inside server reply payloads that
//! callers must not log above debug (SRS NF-6.2).

use base64::Engine as _;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WalletError {
    #[error("transport: {0}")]
    Transport(String),
    #[error("timeout")]
    Timeout,
    #[error("rpc {code}: {message}")]
    Rpc { code: i64, message: String },
    #[error("auth rejected (http 401) — fix ZALLET_RPC_AUTH")]
    Auth(String),
    #[error("method missing: {0}")]
    MissingVerb(String),
    #[error("env missing or empty: {0}")]
    EnvMissing(String),
}

/// Which RPC surface a call targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Base {
    Zebra,
    Zallet,
}

/// Pinned RPC method names. Defaults are the zcashd-shaped names; ANY of them can be
/// overridden by env at boot so a parity gap never blocks a launch (T-04 pin artifact).
#[derive(Debug, Clone)]
pub struct Methods {
    pub info: String,
    pub list_accounts: String,
    pub addr_for_account: String,
    /// Tried in order: pinned first, then documented fallbacks (FR-2.x parity-aware).
    pub balance: Vec<String>,
    /// uivk export family (share_scope); parity differs across Zallet betas.
    pub export_key: Vec<String>,
    /// Zebra raw transaction lookup (mined-height check for anchors).
    pub raw_tx: String,
    pub send: String,
    pub op_status: String,
    pub op_result: String,
    pub view_tx: String,
    pub import_key: String,
    pub get_block: String,
}

impl Default for Methods {
    fn default() -> Self {
        Self {
            info: "getblockchaininfo".into(),
            list_accounts: "z_listaccounts".into(),
            addr_for_account: "z_getaddressforaccount".into(),
            balance: vec!["z_getbalanceforaccount".into(), "z_getbalance".into()],
            export_key: vec!["z_exportkey".into(), "z_exportaccount".into()],
            raw_tx: "getrawtransaction".into(),
            send: "z_sendmany".into(),
            op_status: "z_getoperationstatus".into(),
            op_result: "z_getoperationresult".into(),
            view_tx: "z_viewtransaction".into(),
            import_key: "z_importkey".into(),
            get_block: "getblock".into(),
        }
    }
}

impl Methods {
    /// Env overrides (single name except `ZSOL_RPC_METHOD_BALANCE` = comma-separated family).
    pub fn from_env() -> Self {
        let one = |k: &str, d: String| {
            std::env::var(k)
                .ok()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(d)
        };
        let family = |k: &str, d: Vec<String>| -> Vec<String> {
            std::env::var(k)
                .ok()
                .filter(|s| !s.trim().is_empty())
                .map(|s| {
                    s.split(',')
                        .map(|x| x.trim().to_string())
                        .filter(|x| !x.is_empty())
                        .collect()
                })
                .filter(|v: &Vec<String>| !v.is_empty())
                .unwrap_or(d)
        };
        Self {
            info: one("ZSOL_RPC_METHOD_INFO", "getblockchaininfo".into()),
            list_accounts: one("ZSOL_RPC_METHOD_LISTACCOUNTS", "z_listaccounts".into()),
            addr_for_account: one(
                "ZSOL_RPC_METHOD_ADDRFORACCT",
                "z_getaddressforaccount".into(),
            ),
            balance: family(
                "ZSOL_RPC_METHOD_BALANCE",
                vec!["z_getbalanceforaccount".into(), "z_getbalance".into()],
            ),
            export_key: family(
                "ZSOL_RPC_METHOD_EXPORTKEY",
                vec!["z_exportkey".into(), "z_exportaccount".into()],
            ),
            raw_tx: one("ZSOL_RPC_METHOD_RAWTX", "getrawtransaction".into()),
            send: one("ZSOL_RPC_METHOD_SEND", "z_sendmany".into()),
            op_status: one("ZSOL_RPC_METHOD_OPSTATUS", "z_getoperationstatus".into()),
            op_result: one("ZSOL_RPC_METHOD_OPRESULT", "z_getoperationresult".into()),
            view_tx: one("ZSOL_RPC_METHOD_VIEWTX", "z_viewtransaction".into()),
            import_key: one("ZSOL_RPC_METHOD_IMPORTKEY", "z_importkey".into()),
            get_block: one("ZSOL_RPC_METHOD_GETBLOCK", "getblock".into()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Cfg {
    pub zebra_url: String,
    pub zallet_url: String,
    /// Prebuilt Authorization header value, e.g. `Basic <base64(__cookie__:...)>` taken from
    /// the z3 cookie volume by the operator (never stored in-repo).
    pub auth_header: Option<String>,
    pub timeout: Duration,
    pub methods: Methods,
    pub attest_account: u64,
}

fn env_req(k: &str) -> Result<String, WalletError> {
    std::env::var(k)
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .ok_or_else(|| WalletError::EnvMissing(k.into()))
}

fn env_u64(k: &str, d: u64) -> u64 {
    std::env::var(k)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(d)
}

impl Cfg {
    pub fn from_env() -> Result<Self, WalletError> {
        Ok(Self {
            zebra_url: env_req("ZEBRA_RPC_URL")?,
            zallet_url: env_req("ZALLET_RPC_URL")?,
            auth_header: std::env::var("ZALLET_RPC_AUTH")
                .ok()
                .filter(|s| !s.trim().is_empty()),
            timeout: Duration::from_secs(env_u64("ZSOL_RPC_TIMEOUT_SECS", 5)),
            methods: Methods::from_env(),
            attest_account: env_u64("ZSOL_ATTEST_ACCOUNT", 9),
        })
    }
}

#[derive(Deserialize)]
struct RpcResp {
    result: Option<Value>,
    #[serde(default)]
    error: Option<RpcErr>,
}

#[derive(Debug, Deserialize, thiserror::Error)]
#[error("rpc {code}: {message}")]
pub struct RpcErr {
    pub code: i64,
    pub message: String,
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    cfg: Cfg,
}

impl Client {
    pub fn new(cfg: Cfg) -> Result<Self, WalletError> {
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .build()
            .map_err(|e| WalletError::Transport(e.to_string()))?;
        Ok(Self { http, cfg })
    }

    /// JSON-RPC 1.0 call. Returns raw `result`. RFC-consistent handling:
    /// `-32601` method-not-found ⇒ `WalletError::MissingVerb` (boot-pin reality check).
    pub async fn call(&self, base: Base, method: &str, params: Value) -> Result<Value, WalletError> {
        let url = match base {
            Base::Zebra => &self.cfg.zebra_url,
            Base::Zallet => &self.cfg.zallet_url,
        };
        let body = json!({"jsonrpc": "1.0", "id": "zsol", "method": method, "params": params});
        let mut req = self.http.post(url).json(&body);
        if let Some(a) = &self.cfg.auth_header {
            req = req.header(reqwest::header::AUTHORIZATION, a.clone());
        }
        let resp = req.send().await.map_err(|e| {
            if e.is_timeout() {
                WalletError::Timeout
            } else {
                WalletError::Transport(e.to_string())
            }
        })?;
        let status = resp.status();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| WalletError::Transport(e.to_string()))?;
        if !status.is_success() {
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err(WalletError::Auth(status.as_str().to_string()));
            }
            return Err(WalletError::Transport(format!("http {status}")));
        }
        let parsed: RpcResp = serde_json::from_slice(&bytes)
            .map_err(|_| WalletError::Transport("non-json rpc body".into()))?;
        if let Some(err) = parsed.error {
            if err.code == -32601 {
                return Err(WalletError::MissingVerb(method.to_string()));
            }
            return Err(WalletError::Rpc {
                code: err.code,
                message: err.message,
            });
        }
        Ok(parsed.result.unwrap_or(Value::Null))
    }

    // ---------- pinned, parity-aware helpers ----------

    /// Chain truth (Zebra).
    pub async fn info(&self) -> Result<Value, WalletError> {
        self.call(Base::Zebra, &self.cfg.methods.info, json!([])).await
    }

    pub async fn list_accounts(&self) -> Result<Value, WalletError> {
        self.call(Base::Zallet, &self.cfg.methods.list_accounts, json!([]))
            .await
    }

    /// Generic family probe (parity-aware, boot-pinned). Returns the method that WORKED
    /// + its raw result. A param-shape error on a present verb still counts as "present"
    /// (code -8/-5/-3/-32602 in zcashd-shaped RPCs) with Null result.
    pub async fn probe_family(
        &self,
        verbs: &[String],
        params: Value,
    ) -> Result<(String, Value), WalletError> {
        let mut last: Option<WalletError> = None;
        for m in verbs {
            match self.call(Base::Zallet, m, params.clone()).await {
                Ok(v) => return Ok((m.clone(), v)),
                Err(WalletError::MissingVerb(_)) => {
                    last = Some(WalletError::MissingVerb(m.clone()))
                }
                Err(WalletError::Rpc { code, .. })
                    if matches!(code, -8 | -5 | -3 | -32602) =>
                {
                    return Ok((m.clone(), Value::Null))
                }
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or(WalletError::MissingVerb("family".into())))
    }

    /// Balance family probe for ANY account (attestor's dedicated account and the
    /// verifier's imported scope both need this). SPEC §8 zsol_prepare_anchor / §6
    /// balance-at-scan.
    pub async fn balance_for(&self, account: u64) -> Result<(String, Value), WalletError> {
        self.probe_family(&self.cfg.methods.balance, json!([account]))
            .await
    }

    /// Balance probe for the attestor's dedicated account.
    pub async fn balance(&self) -> Result<(String, Value), WalletError> {
        self.balance_for(self.cfg.attest_account).await
    }

    /// uivk export family (share_scope). Target is the wallet's own address/account
    /// reference — export never executes spends and never leaves this wallet's uivk
    /// scope (scope guard lives in the zsol-mcp layer, FR-2.8).
    pub async fn export_key(&self, target: &str) -> Result<(String, Value), WalletError> {
        self.probe_family(&self.cfg.methods.export_key, json!([target]))
            .await
    }

    /// Zebra raw tx lookup (verbose), used by anchor lifecycles for mined-height checks.
    pub async fn raw_tx(&self, txid: &str) -> Result<Value, WalletError> {
        self.call(Base::Zebra, &self.cfg.methods.raw_tx, json!([txid, 1]))
            .await
    }

    /// Diversified same-account UA derivation (SPEC P5/§3 attest account).
    pub async fn addr_for_account(&self, account: u64, ua_types: &[&str]) -> Result<Value, WalletError> {
        self.call(
            Base::Zallet,
            &self.cfg.methods.addr_for_account,
            json!([account, ua_types, null]),
        )
        .await
    }

    pub async fn view_tx(&self, txid: &str) -> Result<Value, WalletError> {
        self.call(Base::Zallet, &self.cfg.methods.view_tx, json!([txid]))
            .await
    }

    pub async fn import_key(&self, key: &str) -> Result<Value, WalletError> {
        self.call(Base::Zallet, &self.cfg.methods.import_key, json!([key]))
            .await
    }

    /// Outgoing self-spend anchor composer per SPEC §5 (FR-2.4). Builds the exact
    /// z_sendmany params; NOTHING is broadcast unless the caller passes `dry_run=false`
    /// and the wallet RPC accepts. Memo binary must already be the encoded anchor bytes.
    pub fn build_self_spend_params(
        from_account: u64,
        to_ua: &str,
        amount_zat: u64,
        memo_bytes: &[u8],
        min_confirms: u32,
    ) -> Value {
        let memo_b64 = base64::engine::general_purpose::STANDARD.encode(memo_bytes);
        json!([
            from_account.to_string(),          // from: account alias "9"
            [json!({
                "address": to_ua,
                "amount": amount_zat as f64,   // zcashd-shaped: numeric zat? pinned at boot; kept explicit
                "memo": memo_b64
            })],
            min_confirms
        ])
    }

    pub async fn send(&self, params: Value) -> Result<Value, WalletError> {
        self.call(Base::Zallet, &self.cfg.methods.send, params).await
    }

    pub async fn op_status(&self) -> Result<Value, WalletError> {
        self.call(Base::Zallet, &self.cfg.methods.op_status, json!([]))
            .await
    }

    /// Targeted op status (FR-2.5 lifecycle: submit → poll → result).
    pub async fn op_status_for(&self, opid: &str) -> Result<Value, WalletError> {
        self.call(
            Base::Zallet,
            &self.cfg.methods.op_status,
            json!([opid.to_string()]),
        )
        .await
    }

    pub async fn op_result(&self, opid: &str) -> Result<Value, WalletError> {
        self.call(Base::Zallet, &self.cfg.methods.op_result, json!([opid]))
            .await
    }

    pub async fn get_block(&self, height: u64) -> Result<Value, WalletError> {
        self.call(Base::Zebra, &self.cfg.methods.get_block, json!([height]))
            .await
    }

    /// Deserialize helper (typed callers) — keeps `call` returning raw Value for doctor.
    pub async fn call_typed<T: DeserializeOwned>(
        &self,
        base: Base,
        method: &str,
        params: Value,
    ) -> Result<T, WalletError> {
        let v = self.call(base, method, params).await?;
        serde_json::from_value(v).map_err(|_| WalletError::Transport("shape mismatch".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Honest params builder: z_sendmany shape is *pinned*; test pins the current
    /// assumption so drift shows up as a test failure, not a runtime surprise (FR-2.4).
    #[test]
    fn ac_self_spend_params_shape() {
        let memo = zsol_types::AnchorMemo { height: 1, digest: [3u8; 32] }.encode();
        let params = Client::build_self_spend_params(9, "u1x", 1_000_000, &memo, 1);
        let arr = params.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0], json!("9"));
        let outs = arr[1].as_array().unwrap();
        assert_eq!(outs.len(), 1);
        assert_eq!(outs[0]["address"], json!("u1x"));
        assert_eq!(outs[0]["amount"], json!(1_000_000f64));
        assert!(outs[0]["memo"].is_string());
        assert_eq!(arr[2], json!(1));
    }

    #[test]
    fn ac_methods_env_override_single_and_family() {
        std::env::set_var("ZSOL_RPC_METHOD_INFO", "probeinfo");
        std::env::set_var("ZSOL_RPC_METHOD_BALANCE", "bA,bB");
        let m = Methods::from_env();
        assert_eq!(m.info, "probeinfo");
        assert_eq!(m.balance, vec!["bA".to_string(), "bB".to_string()]);
        std::env::remove_var("ZSOL_RPC_METHOD_INFO");
        std::env::remove_var("ZSOL_RPC_METHOD_BALANCE");
    }
}
