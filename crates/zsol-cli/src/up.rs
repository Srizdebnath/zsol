//! `zsol up` (SRS FR-8.1, without the tunnel): runs the laptop stack as supervised child
//! processes — claims registry, Credit Desk, the public-verifier MCP gateway and the web app —
//! health-waits each one in order, prints where they listen, and tears all of them down
//! together. Public exposure (the FR-8.1 cloudflared tunnel) is out of scope by owner decision:
//! every service binds exactly where its flag says and nothing else is opened.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::{Duration, Instant};

use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::signal::unix::{Signal, SignalKind, signal};
use zsol_lightwallet::wallet::Meta;
use zsol_lightwallet::{Wallet, WalletKind, chain};

const REGISTRY_BIN: &str = "zsol-registry";
const CREDIT_BIN: &str = "zsol-credit";
const MCP_BIN: &str = "zsol-mcp";
/// Grace period between the polite SIGINT and the SIGKILL of every child process group.
const STOP_GRACE: Duration = Duration::from_secs(8);
const POLL: Duration = Duration::from_millis(500);

#[derive(clap::Args)]
#[command(
    after_help = "No tunnel: services bind only to the addresses given here. The \
cloudflared part of FR-8.1 is out of scope; expose ports yourself if you need public URLs.\n\
NEXT_PUBLIC_* URLs are inlined by `next build`; pass --build-web when the bound URLs differ \
from the ones the existing web build was made with."
)]
pub struct UpArgs {
    /// Repo root (config/, web/, .env). Default: derived from the binary's target/<profile>
    /// location, else the current directory
    #[arg(long, env = "ZSOL_ROOT")]
    root: Option<PathBuf>,
    /// Directory holding zsol-registry / zsol-credit / zsol-mcp. Default: next to this binary
    #[arg(long, env = "ZSOL_BIN_DIR")]
    bin_dir: Option<PathBuf>,
    /// Seconds each service gets to answer its health probe
    #[arg(long, default_value_t = 180)]
    health_timeout_secs: u64,

    #[arg(long, env = "ZSOL_REGISTRY_BIND", default_value = "127.0.0.1:2730")]
    registry_bind: String,
    /// Registry SQLite file. Default: ~/.zsol/registry.sqlite
    #[arg(long, env = "ZSOL_REGISTRY_DB")]
    registry_db: Option<PathBuf>,

    /// Do not start the Credit Desk (web /desk will show it as down)
    #[arg(long)]
    no_credit: bool,
    #[arg(long, env = "ZSOL_CREDIT_BIND", default_value = "127.0.0.1:8090")]
    credit_bind: String,
    /// Underwriter wallet dir (`zsol wallet init --role underwriter`). Default: ~/.zsol/underwriter
    #[arg(long, env = "ZSOL_CREDIT_DIR")]
    credit_dir: Option<PathBuf>,
    /// Default: <root>/config/credit-policy.json
    #[arg(long, env = "ZSOL_CREDIT_POLICY")]
    credit_policy: Option<PathBuf>,
    /// File BITREFILL_API_KEY is read from when it is not in the environment. Default: <root>/.env
    #[arg(long)]
    env_file: Option<PathBuf>,

    /// Public verifier gateway (zsol-mcp, HTTP only, spending disabled)
    #[arg(long, env = "ZSOL_MCP_HTTP_BIND", default_value = "127.0.0.1:8099")]
    mcp_bind: String,
    /// View-only verifier wallet; created at the current tip if missing.
    /// Default: ~/.zsol/public-verifier
    #[arg(long, env = "ZSOL_PUBLIC_VERIFIER_DIR")]
    verifier_dir: Option<PathBuf>,
    /// Identity the gateway issues challenges under. Default: the wallet dir name
    #[arg(long, env = "ZSOL_VERIFIER_ID")]
    verifier_id: Option<String>,

    /// Do not start the web app
    #[arg(long)]
    no_web: bool,
    #[arg(long, env = "ZSOL_WEB_PORT", default_value_t = 3000)]
    web_port: u16,
    /// Default: <root>/web
    #[arg(long)]
    web_dir: Option<PathBuf>,
    /// Run `pnpm build` with NEXT_PUBLIC_* set to the URLs bound here before starting the web app
    #[arg(long)]
    build_web: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum UpError {
    #[error(
        "cannot locate the zsol repo root (no Cargo.toml near the binary or in the cwd); pass --root"
    )]
    NoRoot,
    #[error("HOME is unset; pass the directory flags explicitly")]
    NoHome,
    #[error("{bin} not found at {} (build it: cargo build --release -p {bin})", .path.display())]
    MissingBinary { bin: &'static str, path: PathBuf },
    #[error("bind address {0:?} is not host:port")]
    BadBind(String),
    #[error("no underwriter wallet in {} (create one: zsol wallet init --role underwriter --dir {}), or pass --no-credit", .0.display(), .0.display())]
    NoUnderwriter(PathBuf),
    #[error("credit policy not found: {}", .0.display())]
    NoPolicy(PathBuf),
    #[error("BITREFILL_API_KEY is neither in the environment nor in {}", .0.display())]
    NoBitrefillKey(PathBuf),
    #[error("{} holds a spending wallet; the public verifier must be view-only", .0.display())]
    VerifierNotViewOnly(PathBuf),
    #[error("verifier wallet in {} is for network {found:?}, not {expected:?}", .dir.display())]
    VerifierNetwork {
        dir: PathBuf,
        found: String,
        expected: String,
    },
    #[error("unreadable wallet metadata in {}: {detail}", .dir.display())]
    VerifierMeta { dir: PathBuf, detail: String },
    #[error("no Next.js build in {}; build it first (or pass --build-web / --no-web):\n  {cmd}", .dir.display())]
    NoWebBuild { dir: PathBuf, cmd: String },
    #[error("web build failed ({0})")]
    WebBuild(String),
    #[error("cannot start {name}: {source}")]
    Spawn {
        name: &'static str,
        source: std::io::Error,
    },
    #[error("{name} exited during startup ({status})")]
    ExitedEarly { name: &'static str, status: String },
    #[error("{name} was not healthy at {url} within {secs}s")]
    HealthTimeout {
        name: &'static str,
        url: String,
        secs: u64,
    },
    #[error("{name} exited ({status}); the rest of the stack was stopped")]
    ChildExited { name: &'static str, status: String },
    #[error("signal handler: {0}")]
    Signals(std::io::Error),
}

/// How a service proves it is serving.
enum Probe {
    /// GET must answer 2xx.
    Get(String),
    /// POST a JSON-RPC `ping` to the MCP endpoint; a `result` member means alive. Unlike the
    /// `zsol_health` tool this never touches the wallet or the chain.
    McpPing(String),
}

impl Probe {
    fn url(&self) -> &str {
        match self {
            Probe::Get(u) | Probe::McpPing(u) => u,
        }
    }

    async fn ok(&self, http: &reqwest::Client) -> bool {
        match self {
            Probe::Get(u) => http
                .get(u)
                .send()
                .await
                .is_ok_and(|r| r.status().is_success()),
            Probe::McpPing(u) => {
                let body = json!({"jsonrpc": "2.0", "id": "zsol-up", "method": "ping"});
                match http.post(u).json(&body).send().await {
                    Ok(r) if r.status().is_success() => r
                        .json::<serde_json::Value>()
                        .await
                        .is_ok_and(|v| v.get("result").is_some()),
                    _ => false,
                }
            }
        }
    }
}

/// One supervised child; it leads its own process group so grandchildren (pnpm → node) are
/// signalled with it and a terminal Ctrl-C reaches only `zsol up`, which then stops everyone.
struct Proc {
    name: &'static str,
    child: Child,
    pgid: i32,
    url: String,
    probe: String,
}

struct Signals {
    int: Signal,
    term: Signal,
}

enum Event {
    Healthy,
    Signal(&'static str),
    Exited(&'static str, ExitStatus),
    TimedOut,
}

/// `host:port` → (URL clients on this machine should use). Wildcard binds map to loopback.
fn local_url(bind: &str) -> Result<String, UpError> {
    let (host, port) = bind
        .rsplit_once(':')
        .ok_or_else(|| UpError::BadBind(bind.into()))?;
    port.parse::<u16>()
        .map_err(|_| UpError::BadBind(bind.into()))?;
    let host = match host {
        "0.0.0.0" | "" => "127.0.0.1",
        "[::]" | "::" => "[::1]",
        h => h,
    };
    Ok(format!("http://{host}:{port}"))
}

fn home() -> Result<PathBuf, UpError> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .ok_or(UpError::NoHome)
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(Path::to_path_buf)
}

fn find_root() -> Result<PathBuf, UpError> {
    let is_root = |p: &Path| p.join("Cargo.toml").is_file() && p.join("crates/zsol-cli").is_dir();
    // <root>/target/<profile>/zsol
    let from_exe = exe_dir()
        .as_deref()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .filter(|r| is_root(r))
        .map(Path::to_path_buf);
    from_exe
        .or_else(|| std::env::current_dir().ok().filter(|d| is_root(d)))
        .ok_or(UpError::NoRoot)
}

/// Minimal dotenv reader: `KEY=VALUE` lines, optional `export `, `#` comment lines, optional
/// surrounding quotes. Values are returned, never logged.
fn read_env_file(path: &Path) -> HashMap<String, String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let (k, v) = l.strip_prefix("export ").unwrap_or(l).split_once('=')?;
            let v = v.trim();
            let v = v
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .or_else(|| v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
                .unwrap_or(v);
            Some((k.trim().to_string(), v.to_string()))
        })
        .collect()
}

fn bitrefill_key(env_file: &Path) -> Result<String, UpError> {
    std::env::var("BITREFILL_API_KEY")
        .ok()
        .or_else(|| read_env_file(env_file).remove("BITREFILL_API_KEY"))
        .filter(|k| !k.trim().is_empty())
        .ok_or_else(|| UpError::NoBitrefillKey(env_file.to_path_buf()))
}

fn sibling(bin_dir: &Path, bin: &'static str) -> Result<PathBuf, UpError> {
    let path = bin_dir.join(bin);
    if path.is_file() {
        Ok(path)
    } else {
        Err(UpError::MissingBinary { bin, path })
    }
}

/// Opens (or creates at the current tip) the gateway's view-only wallet. An existing dir is
/// judged by meta.json alone so a spending wallet's database is never opened here.
async fn ensure_verifier_wallet(dir: &Path, lwd: &str, network: &str) -> anyhow::Result<()> {
    let params = chain::parse_network(network)?;
    if Wallet::exists(dir) {
        let meta: Meta = std::fs::read(dir.join("meta.json"))
            .map_err(|e| e.to_string())
            .and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string()))
            .map_err(|detail| UpError::VerifierMeta {
                dir: dir.into(),
                detail,
            })?;
        if meta.kind != WalletKind::ViewOnly {
            return Err(UpError::VerifierNotViewOnly(dir.into()).into());
        }
        if meta.network != chain::network_name(&params) {
            return Err(UpError::VerifierNetwork {
                dir: dir.into(),
                found: meta.network,
                expected: chain::network_name(&params).into(),
            }
            .into());
        }
        return Ok(());
    }
    // Per-account birthdays come from each imported UFVK; the wallet's own birthday is the tip.
    let mut c = chain::connect(lwd).await?;
    let info = chain::boot_check(&mut c, &params, lwd).await?;
    Wallet::create_view_only(dir, params, info.tip_height)?;
    println!(
        "zsol up: created view-only verifier wallet in {} (birthday {})",
        dir.display(),
        info.tip_height
    );
    Ok(())
}

fn pipe(name: &'static str, stream: Option<impl AsyncRead + Unpin + Send + 'static>) {
    let Some(stream) = stream else { return };
    tokio::spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            println!("{name:>8} | {line}");
        }
    });
}

fn spawn(
    name: &'static str,
    mut cmd: Command,
    url: String,
    probe: String,
) -> Result<Proc, UpError> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("ZSOL_ALLOW_SPEND")
        .process_group(0)
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|source| UpError::Spawn { name, source })?;
    let pgid = child.id().map_or(0, |id| id as i32);
    pipe(name, child.stdout.take());
    pipe(name, child.stderr.take());
    Ok(Proc {
        name,
        child,
        pgid,
        url,
        probe,
    })
}

/// Polls children and (optionally) a health probe until something happens.
async fn watch(
    procs: &mut [Proc],
    sig: &mut Signals,
    http: &reqwest::Client,
    probe: Option<(&Probe, Instant)>,
) -> Event {
    loop {
        for p in procs.iter_mut() {
            if let Ok(Some(status)) = p.child.try_wait() {
                return Event::Exited(p.name, status);
            }
        }
        if let Some((probe, deadline)) = probe {
            if probe.ok(http).await {
                return Event::Healthy;
            }
            if Instant::now() >= deadline {
                return Event::TimedOut;
            }
        }
        tokio::select! {
            _ = sig.int.recv() => return Event::Signal("SIGINT"),
            _ = sig.term.recv() => return Event::Signal("SIGTERM"),
            _ = tokio::time::sleep(POLL) => {}
        }
    }
}

fn signal_group(pgid: i32, sig: i32) {
    if pgid > 0 {
        // SAFETY: plain syscall on a process group we created; ESRCH (already gone) is fine.
        unsafe {
            libc::kill(-pgid, sig);
        }
    }
}

/// SIGINT every group (newest first), wait up to STOP_GRACE, then SIGKILL whatever is left —
/// including grandchildren whose direct parent already exited.
async fn shutdown(procs: &mut [Proc]) {
    for p in procs.iter().rev() {
        signal_group(p.pgid, libc::SIGINT);
    }
    let deadline = Instant::now() + STOP_GRACE;
    for p in procs.iter_mut().rev() {
        let left = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, p.child.wait()).await {
            Ok(Ok(status)) => println!("zsol up: {} stopped ({status})", p.name),
            _ => println!(
                "zsol up: {} did not stop within {}s; killing",
                p.name,
                STOP_GRACE.as_secs()
            ),
        }
    }
    for p in procs.iter_mut() {
        signal_group(p.pgid, libc::SIGKILL);
        let _ = p.child.wait().await;
    }
}

fn web_build_cmd(web_dir: &Path, env: &[(&str, String)]) -> String {
    let vars: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{} pnpm --dir {} build", vars.join(" "), web_dir.display())
}

fn print_table(procs: &[Proc]) {
    println!();
    println!("{:<9} {:<8} {:<28} health probe", "service", "pid", "url");
    for p in procs {
        println!("{:<9} {:<8} {:<28} {}", p.name, p.pgid, p.url, p.probe);
    }
    if let Some(web) = procs.iter().find(|p| p.name == "web") {
        println!("\nverify page: {}/verify   desk: {}/desk", web.url, web.url);
    }
    println!(
        "local only: no tunnel is started (FR-8.1 tunnel out of scope). Ctrl-C stops everything.\n"
    );
}

pub async fn run(args: &UpArgs, lwd: &str, network: &str) -> anyhow::Result<()> {
    chain::parse_network(network)?;
    let root = match &args.root {
        Some(r) => r.clone(),
        None => find_root()?,
    };
    let bin_dir = match &args.bin_dir {
        Some(b) => b.clone(),
        None => exe_dir().ok_or(UpError::NoRoot)?,
    };
    let zsol_home = || home().map(|h| h.join(".zsol"));
    let registry_url = local_url(&args.registry_bind)?;
    let credit_url = local_url(&args.credit_bind)?;
    let mcp_url = local_url(&args.mcp_bind)?;
    let web_url = format!("http://localhost:{}", args.web_port);

    // Pre-flight: every check that can fail without starting anything runs first.
    let registry_bin = sibling(&bin_dir, REGISTRY_BIN)?;
    let mcp_bin = sibling(&bin_dir, MCP_BIN)?;
    let registry_db = match &args.registry_db {
        Some(p) => p.clone(),
        None => zsol_home()?.join("registry.sqlite"),
    };
    let credit = if args.no_credit {
        None
    } else {
        let bin = sibling(&bin_dir, CREDIT_BIN)?;
        let dir = match &args.credit_dir {
            Some(d) => d.clone(),
            None => zsol_home()?.join("underwriter"),
        };
        if !Wallet::exists(&dir) {
            return Err(UpError::NoUnderwriter(dir).into());
        }
        let policy = args
            .credit_policy
            .clone()
            .unwrap_or_else(|| root.join("config/credit-policy.json"));
        if !policy.is_file() {
            return Err(UpError::NoPolicy(policy).into());
        }
        let key = bitrefill_key(&args.env_file.clone().unwrap_or_else(|| root.join(".env")))?;
        Some((bin, dir, policy, key))
    };
    let verifier_dir = match &args.verifier_dir {
        Some(d) => d.clone(),
        None => zsol_home()?.join("public-verifier"),
    };
    let web_dir = args.web_dir.clone().unwrap_or_else(|| root.join("web"));
    let mut web_env = vec![
        ("NEXT_PUBLIC_REGISTRY_URL", registry_url.clone()),
        ("NEXT_PUBLIC_MCP_URL", mcp_url.clone()),
    ];
    if credit.is_some() {
        web_env.push(("NEXT_PUBLIC_CREDIT_URL", credit_url.clone()));
    }
    if !args.no_web && !args.build_web && !web_dir.join(".next/BUILD_ID").is_file() {
        return Err(UpError::NoWebBuild {
            cmd: web_build_cmd(&web_dir, &web_env),
            dir: web_dir,
        }
        .into());
    }
    ensure_verifier_wallet(&verifier_dir, lwd, network).await?;

    let mut sig = Signals {
        int: signal(SignalKind::interrupt()).map_err(UpError::Signals)?,
        term: signal(SignalKind::terminate()).map_err(UpError::Signals)?,
    };
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()?;
    let timeout = Duration::from_secs(args.health_timeout_secs);
    let mut procs: Vec<Proc> = Vec::new();

    // Startup plan, in dependency order.
    let mut plan: Vec<(&'static str, Command, String, Probe)> = Vec::new();
    let mut c = Command::new(&registry_bin);
    c.current_dir(&root)
        .env("ZSOL_REGISTRY_BIND", &args.registry_bind)
        .env("ZSOL_REGISTRY_DB", &registry_db);
    plan.push((
        "registry",
        c,
        registry_url.clone(),
        Probe::Get(format!("{registry_url}/v0/health")),
    ));
    if let Some((bin, dir, policy, key)) = &credit {
        let mut c = Command::new(bin);
        c.current_dir(&root)
            .env("ZSOL_CREDIT_DIR", dir)
            .env("ZSOL_CREDIT_POLICY", policy)
            .env("ZSOL_CREDIT_BIND", &args.credit_bind)
            .env("BITREFILL_API_KEY", key)
            .env("ZSOL_LWD", lwd)
            .env("ZSOL_NETWORK", network);
        plan.push((
            "credit",
            c,
            credit_url.clone(),
            Probe::Get(format!("{credit_url}/v0/health")),
        ));
    }
    let mut c = Command::new(&mcp_bin);
    c.current_dir(&root)
        .env("ZSOL_WALLET_DIR", &verifier_dir)
        .env("ZSOL_MCP_HTTP_BIND", &args.mcp_bind)
        .env("ZSOL_MCP_NO_STDIO", "1")
        .env("ZSOL_REGISTRY_URL", &registry_url)
        .env("ZSOL_LWD", lwd)
        .env("ZSOL_NETWORK", network);
    match &credit {
        Some(_) => c.env("ZSOL_CREDIT_URL", &credit_url),
        None => c.env_remove("ZSOL_CREDIT_URL"),
    };
    if let Some(id) = &args.verifier_id {
        c.env("ZSOL_VERIFIER_ID", id);
    }
    plan.push((
        "mcp",
        c,
        mcp_url.clone(),
        Probe::McpPing(format!("{mcp_url}/mcp")),
    ));
    if !args.no_web {
        let mut c = Command::new("pnpm");
        c.arg("--dir")
            .arg(&web_dir)
            .arg("start")
            .env("PORT", args.web_port.to_string());
        c.envs(web_env.iter().map(|(k, v)| (*k, v.as_str())));
        if credit.is_none() {
            c.env_remove("NEXT_PUBLIC_CREDIT_URL");
        }
        plan.push((
            "web",
            c,
            web_url.clone(),
            Probe::Get(format!("http://127.0.0.1:{}/", args.web_port)),
        ));
    }

    if args.build_web && !args.no_web {
        let mut c = Command::new("pnpm");
        c.arg("--dir").arg(&web_dir).arg("build");
        c.envs(web_env.iter().map(|(k, v)| (*k, v.as_str())));
        let mut build = spawn("build", c, String::new(), String::new())?;
        let status = tokio::select! {
            s = build.child.wait() => s,
            _ = sig.int.recv() => { shutdown(std::slice::from_mut(&mut build)).await; return Ok(()) }
            _ = sig.term.recv() => { shutdown(std::slice::from_mut(&mut build)).await; return Ok(()) }
        };
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => return Err(UpError::WebBuild(s.to_string()).into()),
            Err(e) => return Err(UpError::WebBuild(e.to_string()).into()),
        }
    }

    for (name, cmd, url, probe) in plan {
        println!("zsol up: starting {name} …");
        procs.push(spawn(name, cmd, url, probe.url().into())?);
        let deadline = Instant::now() + timeout;
        match watch(&mut procs, &mut sig, &http, Some((&probe, deadline))).await {
            Event::Healthy => println!("zsol up: {name} healthy at {}", probe.url()),
            Event::Signal(s) => {
                println!("zsol up: {s} during startup; stopping");
                shutdown(&mut procs).await;
                return Ok(());
            }
            Event::Exited(who, status) => {
                shutdown(&mut procs).await;
                return Err(UpError::ExitedEarly {
                    name: who,
                    status: status.to_string(),
                }
                .into());
            }
            Event::TimedOut => {
                shutdown(&mut procs).await;
                return Err(UpError::HealthTimeout {
                    name,
                    url: probe.url().into(),
                    secs: timeout.as_secs(),
                }
                .into());
            }
        }
    }
    print_table(&procs);
    if !args.no_web && !args.build_web {
        println!(
            "note: NEXT_PUBLIC_* are inlined at build time; if web/.next was built with other URLs, rerun with --build-web\n"
        );
    }

    match watch(&mut procs, &mut sig, &http, None).await {
        Event::Exited(name, status) => {
            println!("zsol up: {name} exited ({status}); stopping the rest");
            shutdown(&mut procs).await;
            Err(UpError::ChildExited {
                name,
                status: status.to_string(),
            }
            .into())
        }
        Event::Signal(s) => {
            println!("zsol up: {s}; stopping");
            shutdown(&mut procs).await;
            Ok(())
        }
        Event::Healthy | Event::TimedOut => {
            shutdown(&mut procs).await;
            Ok(())
        }
    }
}
