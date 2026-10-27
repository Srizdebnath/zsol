//! `zsol` — operator CLI for the light-client backend. Every command talks to the real chain
//! through a lightwalletd endpoint (`ZSOL_LWD`, default testnet.zec.rocks:443). Output is JSON,
//! except `zsol up`, which supervises the local service stack (see `up.rs`).

mod up;

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context as _, anyhow};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use zsol_lightwallet::{Wallet, attest, chain};
use zsol_types::{AttestationObject, Challenge, b64u_encode, now_epoch_secs};
use zsol_verifier::{Presentation, VerifierConfig};

#[derive(Parser)]
#[command(name = "zsol", about = "zsol: live solvency attestations on Zcash")]
struct Cli {
    /// lightwalletd endpoint host:port
    #[arg(long, env = "ZSOL_LWD", default_value = chain::DEFAULT_TESTNET_ENDPOINT, global = true)]
    lwd: String,
    /// "test" or "main"
    #[arg(long, env = "ZSOL_NETWORK", default_value = "test", global = true)]
    network: String,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Endpoint boot check (SRS FR-2.1)
    Chain,
    /// Wallet management
    #[command(subcommand)]
    Wallet(WalletCmd),
    /// Attestor operations (SPEC §5, §7)
    #[command(subcommand)]
    Attest(AttestCmd),
    /// Verifier operations (SPEC §6, §8)
    #[command(subcommand)]
    Verify(VerifyCmd),
    /// Start registry, Credit Desk, public verifier gateway and web locally (SRS FR-8.1; no tunnel)
    Up(Box<up::UpArgs>),
}

#[derive(Subcommand)]
enum WalletCmd {
    /// Create a spending wallet. attestor = accounts commerce + zsol-attn; underwriter = treasury
    Init {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long, value_parser = ["attestor", "underwriter"])]
        role: String,
        #[arg(long)]
        birthday: Option<u32>,
    },
    Address {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        account: String,
        /// Orchard-only receiver (payments land in the Ironwood pool)
        #[arg(long)]
        ironwood: bool,
    },
    Sync {
        #[arg(long)]
        dir: PathBuf,
    },
    Balance {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long, default_value_t = 1)]
        min_conf: u32,
    },
    /// Shielded payment with optional hex memo
    Send {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        #[arg(long)]
        zat: u64,
        #[arg(long)]
        memo_hex: Option<String>,
        #[arg(long, default_value_t = 1)]
        min_conf: u32,
    },
}

#[derive(Subcommand)]
enum AttestCmd {
    /// Anchor a claim: self-spend in zsol-attn carrying the frame digest
    Anchor {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        agent_id: String,
        #[arg(long)]
        claim_zat: u64,
        #[arg(long, default_value_t = 24)]
        max_age_hours: u32,
        #[arg(long, default_value_t = 1)]
        min_conf: u32,
        #[arg(long, default_value_t = 900)]
        mine_timeout_secs: u64,
    },
    /// List this wallet's attestations (object b64u included)
    List {
        #[arg(long)]
        dir: PathBuf,
    },
    /// Export the UFVK of zsol-attn (never commerce)
    ShareUfvk {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long, default_value = attest::ATTEST_ACCOUNT)]
        account: String,
    },
    /// Build a full presentation for a verifier challenge (JSON file from `verify challenge`)
    Present {
        #[arg(long)]
        dir: PathBuf,
        /// object b64u
        #[arg(long)]
        object: String,
        #[arg(long)]
        challenge: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
}

#[derive(Subcommand)]
enum VerifyCmd {
    /// Issue a fresh challenge (SPEC §7)
    Challenge {
        #[arg(long)]
        verifier_id: String,
        #[arg(long, default_value_t = 600)]
        ttl_secs: u64,
        #[arg(long)]
        out: PathBuf,
    },
    /// Run C1–C8 on a presentation JSON file in the verifier's own wallet dir
    Run {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        verifier_id: String,
        #[arg(long)]
        presentation: PathBuf,
        #[arg(long, default_value_t = 1)]
        min_conf: u32,
    },
    /// One SPEC §8 watch tick for an object already verified in this wallet
    Watch {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        object: String,
    },
}

fn print(v: Value) {
    println!("{}", serde_json::to_string_pretty(&v).expect("json"));
}

async fn connect(cli: &Cli) -> anyhow::Result<(chain::Client, zsol_lightwallet::chain::ChainInfo)> {
    let params = chain::parse_network(&cli.network)?;
    let mut c = chain::connect(&cli.lwd).await?;
    let info = chain::boot_check(&mut c, &params, &cli.lwd).await?;
    Ok((c, info))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("ZSOL_LOG")
                .unwrap_or_else(|_| "warn,zsol=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    let params = chain::parse_network(&cli.network)?;
    match &cli.cmd {
        Cmd::Up(args) => up::run(args, &cli.lwd, &cli.network).await?,
        Cmd::Chain => {
            let (_, info) = connect(&cli).await?;
            print(serde_json::to_value(info)?);
        }
        Cmd::Wallet(w) => match w {
            WalletCmd::Init {
                dir,
                role,
                birthday,
            } => {
                let (mut c, info) = connect(&cli).await?;
                let names: &[&str] = if role == "attestor" {
                    &["commerce", attest::ATTEST_ACCOUNT]
                } else {
                    &["treasury"]
                };
                let w = Wallet::create_spending(dir, params, &mut c, names, *birthday).await?;
                print(
                    json!({"dir": dir, "birthday": w.meta.birthday, "accounts": w.meta.accounts, "chain": info}),
                );
            }
            WalletCmd::Address {
                dir,
                account,
                ironwood,
            } => {
                let mut w = Wallet::open(dir)?;
                print(json!({"account": account, "address": w.address(account, *ironwood)?}));
            }
            WalletCmd::Sync { dir } => {
                let (mut c, _) = connect(&cli).await?;
                let mut w = Wallet::open(dir)?;
                let t = std::time::Instant::now();
                let r = w.sync(&mut c).await?;
                print(json!({"sync": r, "seconds": t.elapsed().as_secs()}));
            }
            WalletCmd::Balance { dir, min_conf } => {
                let w = Wallet::open(dir)?;
                print(serde_json::to_value(w.balances(*min_conf)?)?);
            }
            WalletCmd::Send {
                dir,
                from,
                to,
                zat,
                memo_hex,
                min_conf,
            } => {
                let (mut c, _) = connect(&cli).await?;
                let mut w = Wallet::open(dir)?;
                w.sync(&mut c).await?;
                let memo = memo_hex
                    .as_deref()
                    .map(|h| hex::decode(h).context("memo_hex is not valid hex"))
                    .transpose()?;
                let txid = w
                    .pay(&mut c, from, to, *zat, memo.as_deref(), *min_conf)
                    .await?;
                print(json!({"txid": txid}));
            }
        },
        Cmd::Attest(a) => match a {
            AttestCmd::Anchor {
                dir,
                agent_id,
                claim_zat,
                max_age_hours,
                min_conf,
                mine_timeout_secs,
            } => {
                let (mut c, _) = connect(&cli).await?;
                let mut w = Wallet::open(dir)?;
                let out = attest::anchor(
                    &mut w,
                    &mut c,
                    &attest::AnchorRequest {
                        agent_id: agent_id.clone(),
                        claim_zat: *claim_zat,
                        max_age_hours: *max_age_hours,
                        min_conf: *min_conf,
                        mine_timeout: Duration::from_secs(*mine_timeout_secs),
                    },
                )
                .await?;
                print(serde_json::to_value(out)?);
            }
            AttestCmd::List { dir } => {
                let w = Wallet::open(dir)?;
                let list: Vec<Value> = attest::list_attestations(&w)?
                    .into_iter()
                    .map(|o| json!({"objectB64u": o.b64u().ok(), "object": o}))
                    .collect();
                print(Value::Array(list));
            }
            AttestCmd::ShareUfvk { dir, account } => {
                let w = Wallet::open(dir)?;
                print(json!({"account": account, "ufvk": w.export_ufvk(account)?}));
            }
            AttestCmd::Present {
                dir,
                object,
                challenge,
                out,
            } => {
                let w = Wallet::open(dir)?;
                let o = AttestationObject::from_b64u(object).map_err(|e| anyhow!("object: {e}"))?;
                let ch: Challenge = serde_json::from_slice(&std::fs::read(challenge)?)?;
                let p = Presentation {
                    signature: attest::sign_challenge(&w, &o, &ch)?,
                    ufvk: w.export_ufvk(attest::ATTEST_ACCOUNT)?,
                    object: o,
                    challenge: ch,
                };
                std::fs::write(out, serde_json::to_vec_pretty(&p)?)?;
                print(json!({"written": out}));
            }
        },
        Cmd::Verify(v) => match v {
            VerifyCmd::Challenge {
                verifier_id,
                ttl_secs,
                out,
            } => {
                let mut nonce = [0u8; 32];
                getrandom::fill(&mut nonce).map_err(|e| anyhow!("rng: {e}"))?;
                let ch = Challenge {
                    nonce: b64u_encode(&nonce),
                    verifier_id: verifier_id.clone(),
                    expires_at: now_epoch_secs() + ttl_secs,
                };
                std::fs::write(out, serde_json::to_vec_pretty(&ch)?)?;
                print(serde_json::to_value(ch)?);
            }
            VerifyCmd::Run {
                dir,
                verifier_id,
                presentation,
                min_conf,
            } => {
                let (mut c, info) = connect(&cli).await?;
                let p: Presentation = serde_json::from_slice(
                    &std::fs::read(presentation).context("reading presentation")?,
                )?;
                let mut w = if Wallet::exists(dir) {
                    Wallet::open(dir)?
                } else {
                    Wallet::create_view_only(dir, params, p.object.birthday)?
                };
                let cfg = VerifierConfig {
                    network: cli.network.clone(),
                    verifier_id: verifier_id.clone(),
                    min_conf: *min_conf,
                };
                let t = std::time::Instant::now();
                let report = zsol_verifier::verify(&mut w, &mut c, &cfg, &p).await?;
                print(json!({"report": report, "seconds": t.elapsed().as_secs(), "chain": info}));
            }
            VerifyCmd::Watch { dir, object } => {
                let (mut c, _) = connect(&cli).await?;
                let mut w = Wallet::open(dir)?;
                let o = AttestationObject::from_b64u(object).map_err(|e| anyhow!("object: {e}"))?;
                print(serde_json::to_value(
                    zsol_verifier::watch_tick(&mut w, &mut c, &o).await?,
                )?);
            }
        },
    }
    Ok(())
}
