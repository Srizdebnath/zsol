//! zsol light wallet (SRS FR-2): one directory per party, `zcash_client_sqlite` storage,
//! sync via `zcash_client_backend::sync::run` against a lightwalletd endpoint.
//!
//! Layout of a wallet directory:
//!   meta.json      network, birthday, kind, account name → uuid
//!   identity.txt   age x25519 identity (0600)          [spending wallets only]
//!   seed.age       age-armored BIP-39 mnemonic (0600)  [spending wallets only]
//!   wallet.sqlite  zcash_client_sqlite database

#![allow(deprecated)] // StandardFeeRule is deprecated upstream but is still the ZIP-317 rule.

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::str::FromStr as _;

use anyhow::{Context as _, anyhow, bail};
use rand::{rand_core::UnwrapErr, rngs::SysRng};
use secrecy::{ExposeSecret as _, SecretVec};
use serde::{Deserialize, Serialize};
use zcash_address::ZcashAddress;
use zcash_client_backend::{
    data_api::{
        Account as _, AccountBirthday, AccountPurpose, WalletRead, WalletWrite,
        wallet::{
            ConfirmationsPolicy, SpendingKeys, create_proposed_transactions,
            decrypt_and_store_transaction,
            input_selection::{GreedyInputSelector, SpendPolicy},
            propose_transfer,
        },
    },
    fees::{DustOutputPolicy, StandardFeeRule, zip317::SingleOutputChangeStrategy},
    wallet::OvkPolicy,
};
use zcash_client_sqlite::{AccountUuid, WalletDb, util::SystemClock, wallet::init::init_wallet_db};
use zcash_keys::keys::{UnifiedAddressRequest, UnifiedFullViewingKey, UnifiedSpendingKey};
use zcash_proofs::prover::LocalTxProver;
use zcash_protocol::{
    ShieldedPool,
    consensus::{BlockHeight, Network},
    memo::MemoBytes,
    value::Zatoshis,
};
use zip321::{Payment, TransactionRequest};

use crate::{cache::MemBlockCache, chain, seed};

pub type Db = WalletDb<rusqlite::Connection, Network, SystemClock, UnwrapErr<SysRng>>;

/// Error type of `create_proposed_transactions` for this wallet's backend and fee rule.
type CreateErr = zcash_client_backend::data_api::error::Error<
    zcash_client_sqlite::error::SqliteClientError,
    zcash_client_sqlite::wallet::commitment_tree::Error,
    zcash_client_backend::data_api::wallet::input_selection::GreedyInputSelectorError,
    zcash_primitives::transaction::fees::zip317::FeeError,
    zcash_primitives::transaction::fees::zip317::FeeError,
    zcash_client_sqlite::ReceivedNoteId,
>;

/// Accounts whose UFVK may leave the wallet (SPEC §3, SRS FR-2.5). `commerce` never.
pub fn is_shareable_account(name: &str) -> bool {
    name == "zsol-attn" || name.starts_with("zsol-attn-")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    pub network: String,
    pub kind: WalletKind,
    pub birthday: u32,
    pub accounts: BTreeMap<String, uuid::Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WalletKind {
    Spending,
    ViewOnly,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Balance {
    pub account: String,
    pub total_zat: u64,
    pub spendable_zat: u64,
    pub pending_zat: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub tip_height: u32,
    pub fully_scanned_height: Option<u32>,
}

pub struct Wallet {
    pub dir: PathBuf,
    pub params: Network,
    pub meta: Meta,
    pub db: Db,
}

/// Wallet directories hold key material: owner-only.
fn create_private_dir(dir: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn db_path(dir: &Path) -> PathBuf {
    dir.join("wallet.sqlite")
}

fn open_db(dir: &Path, params: Network) -> anyhow::Result<Db> {
    let mut db = WalletDb::for_path(db_path(dir), params, SystemClock, UnwrapErr(SysRng))
        .context("opening wallet.sqlite")?;
    init_wallet_db(&mut db, None).map_err(|e| anyhow!("wallet db migration: {e}"))?;
    Ok(db)
}

fn identity_path(dir: &Path) -> PathBuf {
    std::env::var_os("ZSOL_AGE_IDENTITY")
        .map(PathBuf::from)
        .unwrap_or_else(|| dir.join("identity.txt"))
}

/// The chain state as of the block before `height`: what a new account needs (ZIP-32 birthday).
async fn birthday_at(client: &mut chain::Client, height: u32) -> anyhow::Result<AccountBirthday> {
    let ts = chain::tree_state(client, height - 1).await?;
    AccountBirthday::from_treestate(ts, None)
        .map_err(|_| anyhow!("unusable tree state at {height}"))
}

impl Wallet {
    pub fn exists(dir: &Path) -> bool {
        dir.join("meta.json").exists()
    }

    pub fn open(dir: &Path) -> anyhow::Result<Self> {
        let meta: Meta = serde_json::from_slice(
            &std::fs::read(dir.join("meta.json"))
                .with_context(|| format!("no zsol wallet in {}", dir.display()))?,
        )?;
        let params = chain::parse_network(&meta.network)?;
        let db = open_db(dir, params)?;
        Ok(Self {
            dir: dir.to_path_buf(),
            params,
            meta,
            db,
        })
    }

    fn save_meta(&self) -> anyhow::Result<()> {
        let tmp = self.dir.join("meta.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&self.meta)?)?;
        std::fs::rename(tmp, self.dir.join("meta.json"))?;
        Ok(())
    }

    /// New spending wallet with the given ZIP-32 accounts (index order = slice order).
    /// Birthday defaults to `tip − 10`.
    pub async fn create_spending(
        dir: &Path,
        params: Network,
        client: &mut chain::Client,
        account_names: &[&str],
        birthday: Option<u32>,
    ) -> anyhow::Result<Self> {
        if Self::exists(dir) {
            bail!("a wallet already exists in {}", dir.display());
        }
        create_private_dir(dir)?;
        let tip = chain::tip_height(client).await?;
        let height = birthday.unwrap_or(tip.saturating_sub(10));
        let bday = birthday_at(client, height).await?;
        let id = seed::load_or_create_identity(&identity_path(dir))?;
        let seed = seed::create_seed(&dir.join("seed.age"), &id)?;
        let mut db = open_db(dir, params)?;
        let mut accounts = BTreeMap::new();
        for name in account_names {
            let (account_id, _usk) = db
                .create_account(name, &seed, &bday, None)
                .map_err(|e| anyhow!("create_account {name}: {e}"))?;
            accounts.insert(name.to_string(), account_id.expose_uuid());
        }
        let w = Self {
            dir: dir.to_path_buf(),
            params,
            meta: Meta {
                network: chain::network_name(&params).into(),
                kind: WalletKind::Spending,
                birthday: height,
                accounts,
            },
            db,
        };
        w.save_meta()?;
        Ok(w)
    }

    /// New wallet that will only ever hold imported view-only accounts (a verifier).
    pub fn create_view_only(dir: &Path, params: Network, birthday: u32) -> anyhow::Result<Self> {
        if Self::exists(dir) {
            return Self::open(dir);
        }
        create_private_dir(dir)?;
        let db = open_db(dir, params)?;
        let w = Self {
            dir: dir.to_path_buf(),
            params,
            meta: Meta {
                network: chain::network_name(&params).into(),
                kind: WalletKind::ViewOnly,
                birthday,
                accounts: BTreeMap::new(),
            },
            db,
        };
        w.save_meta()?;
        Ok(w)
    }

    pub fn account(&self, name: &str) -> anyhow::Result<AccountUuid> {
        self.meta
            .accounts
            .get(name)
            .map(|u| AccountUuid::from_uuid(*u))
            .ok_or_else(|| anyhow!("no account named {name:?} in this wallet"))
    }

    /// Imports a UFVK as a view-only account (SPEC §6, SRS FR-3.1). Idempotent per UFVK.
    pub async fn import_view_only(
        &mut self,
        client: &mut chain::Client,
        name: &str,
        ufvk_encoded: &str,
        birthday: u32,
    ) -> anyhow::Result<AccountUuid> {
        let ufvk = UnifiedFullViewingKey::decode(&self.params, ufvk_encoded.trim())
            .map_err(|e| anyhow!("UFVK decode: {e}"))?;
        if let Some(acct) = self
            .db
            .get_account_for_ufvk(&ufvk)
            .map_err(|e| anyhow!("{e}"))?
        {
            let id = acct.id();
            self.meta.accounts.insert(name.into(), id.expose_uuid());
            self.save_meta()?;
            return Ok(id);
        }
        let bday = birthday_at(client, birthday).await?;
        self.import_ufvk_at(name, &ufvk, &bday)
    }

    /// The offline half of [`Self::import_view_only`]: stores `ufvk` as a ViewOnly account at an
    /// already-resolved birthday. Never derives or stores spending material.
    fn import_ufvk_at(
        &mut self,
        name: &str,
        ufvk: &UnifiedFullViewingKey,
        bday: &AccountBirthday,
    ) -> anyhow::Result<AccountUuid> {
        let acct = self
            .db
            .import_account_ufvk(name, ufvk, bday, AccountPurpose::ViewOnly, None)
            .map_err(|e| anyhow!("import_account_ufvk: {e}"))?;
        let id = acct.id();
        self.meta.accounts.insert(name.into(), id.expose_uuid());
        self.save_meta()?;
        Ok(id)
    }

    /// UFVK export, restricted to attestation accounts (SRS FR-2.5).
    pub fn export_ufvk(&self, name: &str) -> anyhow::Result<String> {
        if !is_shareable_account(name) {
            bail!(
                "refusing to export a viewing key for account {name:?}: only zsol-attn* accounts are shareable"
            );
        }
        let id = self.account(name)?;
        let acct = self
            .db
            .get_account(id)
            .map_err(|e| anyhow!("{e}"))?
            .ok_or_else(|| anyhow!("account vanished"))?;
        let ufvk = acct.ufvk().ok_or_else(|| anyhow!("account has no UFVK"))?;
        ufvk.encode(&self.params)
            .map_err(|e| anyhow!("UFVK encode: {e:?}"))
    }

    /// Current shielded receive address. `orchard_only` yields an address whose payments land
    /// in the Ironwood pool (NU6.3+: Orchard receivers are accounted to Ironwood).
    pub fn address(&mut self, name: &str, orchard_only: bool) -> anyhow::Result<String> {
        let id = self.account(name)?;
        let req = if orchard_only {
            UnifiedAddressRequest::ORCHARD
        } else {
            UnifiedAddressRequest::SHIELDED
        };
        if let Some(ua) = self
            .db
            .get_last_generated_address_matching(id, req)
            .map_err(|e| anyhow!("{e}"))?
        {
            return Ok(ua.encode(&self.params));
        }
        let (ua, _) = self
            .db
            .get_next_available_address(id, req)
            .map_err(|e| anyhow!("{e}"))?
            .ok_or_else(|| anyhow!("no address available for {name}"))?;
        Ok(ua.encode(&self.params))
    }

    /// Scans to the endpoint's tip (compact blocks: received notes + spends via nullifiers).
    pub async fn sync(&mut self, client: &mut chain::Client) -> anyhow::Result<SyncReport> {
        let cache = MemBlockCache::default();
        zcash_client_backend::sync::run(client, &self.params, &cache, &mut self.db, 1_000)
            .await
            .map_err(|e| anyhow!("sync: {e}"))?;
        let tip = chain::tip_height(client).await?;
        let scanned = self
            .db
            .block_fully_scanned()
            .map_err(|e| anyhow!("{e}"))?
            .map(|m| u32::from(m.block_height()));
        Ok(SyncReport {
            tip_height: tip,
            fully_scanned_height: scanned,
        })
    }

    pub fn balances(&self, min_conf: u32) -> anyhow::Result<Vec<Balance>> {
        let policy = ConfirmationsPolicy::new_symmetrical(
            NonZeroU32::new(min_conf.max(1)).expect("nonzero"),
        );
        let summary = self
            .db
            .get_wallet_summary(policy)
            .map_err(|e| anyhow!("{e}"))?;
        let mut out = Vec::new();
        for (name, uuid) in &self.meta.accounts {
            let id = AccountUuid::from_uuid(*uuid);
            let (total, spendable) = summary
                .as_ref()
                .and_then(|s| s.account_balances().get(&id))
                .map(|b| (u64::from(b.total()), u64::from(b.spendable_value())))
                .unwrap_or((0, 0));
            out.push(Balance {
                account: name.clone(),
                total_zat: total,
                spendable_zat: spendable,
                pending_zat: total.saturating_sub(spendable),
            });
        }
        Ok(out)
    }

    fn seed(&self) -> anyhow::Result<SecretVec<u8>> {
        if self.meta.kind != WalletKind::Spending {
            bail!("view-only wallet cannot spend");
        }
        let id = seed::load_or_create_identity(&identity_path(&self.dir))?;
        seed::load_seed(&self.dir.join("seed.age"), &id)
    }

    /// Builds, proves, signs, stores and broadcasts a single shielded payment.
    /// Returns the txid in display order. `memo` is the raw memo field (≤ 512 bytes).
    pub async fn pay(
        &mut self,
        client: &mut chain::Client,
        from: &str,
        to: &str,
        value_zat: u64,
        memo: Option<&[u8]>,
        min_conf: u32,
    ) -> anyhow::Result<String> {
        let from_id = self.account(from)?;
        let acct = self
            .db
            .get_account(from_id)
            .map_err(|e| anyhow!("{e}"))?
            .ok_or_else(|| anyhow!("account vanished"))?;
        let derivation = acct
            .source()
            .key_derivation()
            .ok_or_else(|| anyhow!("{from} is view-only"))?
            .clone();
        let memo = memo
            .map(|m| MemoBytes::from_bytes(m).map_err(|e| anyhow!("memo: {e:?}")))
            .transpose()?;
        let payment = Payment::new(
            ZcashAddress::from_str(to).map_err(|e| anyhow!("recipient: {e}"))?,
            Some(Zatoshis::from_u64(value_zat).map_err(|_| anyhow!("amount out of range"))?),
            memo,
            None,
            None,
            vec![],
        )
        .map_err(|e| anyhow!("payment: {e:?}"))?;
        let request = TransactionRequest::new(vec![payment]).map_err(|e| anyhow!("{e}"))?;
        let change = SingleOutputChangeStrategy::new(
            StandardFeeRule::Zip317,
            None,
            ShieldedPool::Ironwood,
            DustOutputPolicy::default(),
        );
        let policy = ConfirmationsPolicy::new_symmetrical(
            NonZeroU32::new(min_conf.max(1)).expect("nonzero"),
        );
        let proposal = propose_transfer(
            &mut self.db,
            &self.params,
            from_id,
            &GreedyInputSelector::new(),
            &change,
            request,
            policy,
            &SpendPolicy::default(),
            None,
            None,
        )
        .map_err(|e: CreateErr| anyhow!("propose: {e}"))?;
        let seed = self.seed()?;
        let usk = UnifiedSpendingKey::from_seed(
            &self.params,
            seed.expose_secret(),
            derivation.account_index(),
        )
        .map_err(|e| anyhow!("usk: {e:?}"))?;
        let prover = LocalTxProver::bundled();
        let txids = create_proposed_transactions(
            &mut self.db,
            &self.params,
            &SystemClock,
            &mut UnwrapErr(SysRng),
            &prover,
            &prover,
            &SpendingKeys::from_unified_spending_key(usk),
            OvkPolicy::Sender,
            &proposal,
            None,
        )
        .map_err(|e: CreateErr| anyhow!("create: {e}"))?;
        if txids.len() != 1 {
            bail!(
                "proposal produced {} transactions; zsol requires exactly one",
                txids.len()
            );
        }
        let txid = *txids.first();
        let tx = self
            .db
            .get_transaction(txid)
            .map_err(|e| anyhow!("{e}"))?
            .ok_or_else(|| anyhow!("created transaction missing from db"))?;
        chain::send_tx(client, &tx).await?;
        Ok(txid.to_string())
    }

    /// Downloads a full transaction and stores what this wallet's keys can decrypt
    /// (memos, outgoing outputs). Returns the mined height, if mined.
    pub async fn enhance(
        &mut self,
        client: &mut chain::Client,
        txid_internal: [u8; 32],
    ) -> anyhow::Result<Option<Option<u32>>> {
        let Some(f) = chain::fetch_tx(client, &self.params, txid_internal).await? else {
            return Ok(None);
        };
        decrypt_and_store_transaction(
            &self.params,
            &mut self.db,
            &f.tx,
            f.mined_height.map(BlockHeight::from_u32),
        )
        .map_err(|e| anyhow!("store tx: {e}"))?;
        Ok(Some(f.mined_height))
    }

    /// Read-only connection for querying the public `v_*` views of zcash_client_sqlite.
    pub fn views(&self) -> anyhow::Result<rusqlite::Connection> {
        Ok(rusqlite::Connection::open_with_flags(
            db_path(&self.dir),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zcash_client_backend::data_api::chain::ChainState;
    use zcash_primitives::block::BlockHash;
    use zcash_protocol::consensus::{NetworkUpgrade, Parameters as _};

    const NET: Network = Network::TestNetwork;

    /// Owner-only scratch directory under the OS temp dir, removed on drop.
    struct TmpDir(PathBuf);
    impl TmpDir {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!(
                "zsol-lightwallet-test-{}-{tag}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&p);
            Self(p)
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Offline birthday: empty chain state just before Sapling activation (no lightwalletd).
    fn offline_birthday() -> AccountBirthday {
        let h = NET.activation_height(NetworkUpgrade::Sapling).unwrap() - 1;
        AccountBirthday::from_parts(ChainState::empty(h, BlockHash([0; 32])), None)
    }

    /// A spending wallet built offline: same account derivation as `create_spending`, but with
    /// a fresh random in-memory seed and no tree-state fetch (no seed.age is written).
    fn offline_spending_wallet(dir: &Path, names: &[&str]) -> Wallet {
        create_private_dir(dir).unwrap();
        let mut seed_bytes = vec![0u8; 32];
        getrandom::fill(&mut seed_bytes).unwrap();
        let seed = SecretVec::new(seed_bytes);
        let mut db = open_db(dir, NET).unwrap();
        let bday = offline_birthday();
        let mut accounts = BTreeMap::new();
        for name in names {
            let (id, _usk) = db.create_account(name, &seed, &bday, None).unwrap();
            accounts.insert(name.to_string(), id.expose_uuid());
        }
        let w = Wallet {
            dir: dir.to_path_buf(),
            params: NET,
            meta: Meta {
                network: chain::network_name(&NET).into(),
                kind: WalletKind::Spending,
                birthday: u32::from(bday.height()),
                accounts,
            },
            db,
        };
        w.save_meta().unwrap();
        w
    }

    /// FR-2.5: only `zsol-attn` and `zsol-attn-*` are shareable.
    #[test]
    fn shareable_account_rule() {
        for ok in ["zsol-attn", "zsol-attn-2", "zsol-attn-bob"] {
            assert!(is_shareable_account(ok), "{ok}");
        }
        for no in [
            "commerce",
            "treasury",
            "",
            "zsol-attn2",
            "zsol-att",
            "ZSOL-ATTN",
            " zsol-attn",
            "peer-zsol-attn",
        ] {
            assert!(!is_shareable_account(no), "{no:?}");
        }
    }

    /// FR-2.5 AC (negative test): exporting the `commerce` UFVK is refused even though the
    /// account exists and has a UFVK; `zsol-attn` exports fine from the same wallet.
    #[test]
    fn export_ufvk_refuses_commerce() {
        let tmp = TmpDir::new("export");
        let w = offline_spending_wallet(&tmp.0, &["zsol-attn", "commerce"]);
        for name in ["commerce", "treasury", ""] {
            let err = w.export_ufvk(name).unwrap_err().to_string();
            assert!(err.contains("refusing"), "{name:?}: {err}");
        }
        let ufvk = w.export_ufvk("zsol-attn").unwrap();
        assert!(UnifiedFullViewingKey::decode(&NET, &ufvk).is_ok());
        // the commerce account really exists — the refusal is policy, not a lookup miss
        assert!(w.account("commerce").is_ok());
    }

    /// FR-3.1 AC: a verifier wallet in its own data directory imports the attestor's UFVK as
    /// view-only and holds no attestor key material (no seed, no identity, no spending account).
    #[test]
    fn verifier_wallet_holds_no_key_material() {
        let att_tmp = TmpDir::new("attestor");
        let ver_tmp = TmpDir::new("verifier");
        let attestor = offline_spending_wallet(&att_tmp.0, &["zsol-attn", "commerce"]);
        let ufvk_s = attestor.export_ufvk("zsol-attn").unwrap();
        let ufvk = UnifiedFullViewingKey::decode(&NET, &ufvk_s).unwrap();

        let mut v = Wallet::create_view_only(&ver_tmp.0, NET, attestor.meta.birthday).unwrap();
        let id = v
            .import_ufvk_at("peer-agent", &ufvk, &offline_birthday())
            .unwrap();

        // separate directory, view-only kind, persisted as such
        assert_ne!(att_tmp.0, ver_tmp.0);
        assert_eq!(v.meta.kind, WalletKind::ViewOnly);
        assert_eq!(
            Wallet::open(&ver_tmp.0).unwrap().meta.kind,
            WalletKind::ViewOnly
        );

        // no seed / identity files in the verifier's directory
        assert!(!ver_tmp.0.join("seed.age").exists());
        assert!(!ver_tmp.0.join("identity.txt").exists());

        // every account in the verifier DB is ViewOnly; the imported one round-trips the UFVK
        let ids = v.db.get_account_ids().unwrap();
        assert_eq!(ids, vec![id]);
        for aid in ids {
            let acct = v.db.get_account(aid).unwrap().unwrap();
            assert_eq!(acct.purpose(), AccountPurpose::ViewOnly);
        }
        let acct = v.db.get_account(id).unwrap().unwrap();
        assert_eq!(acct.ufvk().unwrap().encode(&NET).unwrap(), ufvk_s);

        // the wallet refuses to produce spending material at all
        assert!(v.seed().is_err());
        // and the import never added the attestor's commerce account
        assert!(v.account("commerce").is_err());
    }
}
