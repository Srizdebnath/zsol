//! zsol light-client backend (SRS FR-2, ARCHITECTURE D2): wallet + chain access built on
//! librustzcash (`zcash_client_backend`/`zcash_client_sqlite`) against a lightwalletd endpoint.

pub mod attest;
pub mod cache;
pub mod chain;
pub mod seed;
pub mod wallet;

pub use wallet::{Wallet, WalletKind, is_shareable_account};
