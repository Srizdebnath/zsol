//! lightwalletd-protocol chain access (SRS FR-2.1). The endpoint is untrusted for privacy but
//! every consensus-relevant fact (branch, heights, tx bytes) is re-derived locally.

use std::time::Duration;

use anyhow::{Context as _, anyhow, bail};
use tonic::transport::{Channel, ClientTlsConfig};
use zcash_client_backend::proto::service::{
    self, BlockId, ChainSpec, Empty, RawTransaction, TxFilter,
    compact_tx_streamer_client::CompactTxStreamerClient,
};
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::{BlockHeight, BranchId, Network};

pub type Client = CompactTxStreamerClient<Channel>;

/// Default public testnet endpoint (verified on NU7 branch 0x77190ad9, 2026-10-26).
pub const DEFAULT_TESTNET_ENDPOINT: &str = "testnet.zec.rocks:443";

pub fn network_name(params: &Network) -> &'static str {
    match params {
        Network::MainNetwork => "main",
        Network::TestNetwork => "test",
    }
}

pub fn parse_network(s: &str) -> anyhow::Result<Network> {
    match s {
        "test" => Ok(Network::TestNetwork),
        "main" => Ok(Network::MainNetwork),
        other => bail!("unknown network {other:?} (expected \"test\" or \"main\")"),
    }
}

/// Several dependencies compile rustls with different crypto backends; pick one explicitly.
fn install_crypto_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// Connects to `host:port`; TLS for every host except loopback (local Zaino).
pub async fn connect(endpoint: &str) -> anyhow::Result<Client> {
    install_crypto_provider();
    let (host, port) = endpoint
        .rsplit_once(':')
        .ok_or_else(|| anyhow!("endpoint must be host:port, got {endpoint:?}"))?;
    let port: u16 = port.parse().context("endpoint port")?;
    let local = matches!(host, "localhost" | "127.0.0.1" | "::1");
    let uri = format!("{}://{host}:{port}", if local { "http" } else { "https" });
    let mut ch = Channel::from_shared(uri)?
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60));
    if !local {
        ch = ch.tls_config(
            ClientTlsConfig::new()
                .domain_name(host.to_string())
                .assume_http2(true)
                .with_webpki_roots(),
        )?;
    }
    let channel = ch
        .connect()
        .await
        .with_context(|| format!("connecting to {endpoint}"))?;
    Ok(CompactTxStreamerClient::new(channel).max_decoding_message_size(64 * 1024 * 1024))
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainInfo {
    pub endpoint: String,
    pub chain_name: String,
    pub consensus_branch_id: String,
    pub tip_height: u32,
    pub vendor: String,
    pub version: String,
}

/// SRS FR-2.1 boot check: the endpoint must serve the expected network, and the branch it
/// reports must equal the branch our own consensus parameters derive for its tip.
pub async fn boot_check(
    client: &mut Client,
    params: &Network,
    endpoint: &str,
) -> anyhow::Result<ChainInfo> {
    let info = client.get_lightd_info(Empty {}).await?.into_inner();
    let want_chain = network_name(params);
    if info.chain_name != want_chain {
        bail!(
            "endpoint serves chain {:?}, expected {want_chain:?}",
            info.chain_name
        );
    }
    let tip: u32 = info.block_height.try_into().context("tip height")?;
    let reported = u32::from_str_radix(info.consensus_branch_id.trim_start_matches("0x"), 16)
        .context("consensusBranchId")?;
    let ours = u32::from(BranchId::for_height(params, BlockHeight::from_u32(tip)));
    if reported != ours {
        bail!(
            "endpoint branch {reported:#010x} != local consensus branch {ours:#010x} at height {tip} \
             (endpoint or this build is on the wrong network upgrade)"
        );
    }
    Ok(ChainInfo {
        endpoint: endpoint.to_string(),
        chain_name: info.chain_name,
        consensus_branch_id: format!("{reported:08x}"),
        tip_height: tip,
        vendor: info.vendor,
        version: info.version,
    })
}

pub async fn tip_height(client: &mut Client) -> anyhow::Result<u32> {
    let b = client
        .get_latest_block(ChainSpec::default())
        .await?
        .into_inner();
    Ok(b.height.try_into()?)
}

pub async fn block_time(client: &mut Client, height: u32) -> anyhow::Result<u32> {
    let b = client
        .get_block(BlockId {
            height: height.into(),
            ..Default::default()
        })
        .await?
        .into_inner();
    Ok(b.time)
}

/// A full transaction as served by the endpoint, with its mined height (None = mempool).
pub struct FetchedTx {
    pub tx: Transaction,
    pub mined_height: Option<u32>,
}

/// `txid_internal` is in internal byte order (see `zsol_types::txid_internal`).
pub async fn fetch_tx(
    client: &mut Client,
    params: &Network,
    txid_internal: [u8; 32],
) -> anyhow::Result<Option<FetchedTx>> {
    let resp = client
        .get_transaction(TxFilter {
            hash: txid_internal.to_vec(),
            ..Default::default()
        })
        .await;
    let raw = match resp {
        Ok(r) => r.into_inner(),
        Err(s) if s.code() == tonic::Code::NotFound => return Ok(None),
        Err(s) if s.message().to_lowercase().contains("not found") => return Ok(None),
        Err(s) => return Err(s.into()),
    };
    if raw.data.is_empty() {
        return Ok(None);
    }
    // lightwalletd reports 0 (or u64::MAX on some servers) for mempool transactions.
    let mined_height = match raw.height {
        0 | u64::MAX => None,
        h => Some(u32::try_from(h)?),
    };
    let branch_height = match mined_height {
        Some(h) => BlockHeight::from_u32(h),
        None => BlockHeight::from_u32(tip_height(client).await? + 1),
    };
    let tx = Transaction::read(&raw.data[..], BranchId::for_height(params, branch_height))
        .context("parsing transaction bytes")?;
    Ok(Some(FetchedTx { tx, mined_height }))
}

pub async fn send_tx(client: &mut Client, tx: &Transaction) -> anyhow::Result<()> {
    let mut raw = RawTransaction::default();
    tx.write(&mut raw.data)?;
    let resp = client.send_transaction(raw).await?.into_inner();
    if resp.error_code != 0 {
        bail!(
            "endpoint rejected transaction (code {}): {}",
            resp.error_code,
            resp.error_message
        );
    }
    Ok(())
}

pub async fn tree_state(client: &mut Client, height: u32) -> anyhow::Result<service::TreeState> {
    Ok(client
        .get_tree_state(BlockId {
            height: height.into(),
            ..Default::default()
        })
        .await?
        .into_inner())
}
