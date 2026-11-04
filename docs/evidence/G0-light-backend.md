# G0 — light-client backend live on testnet (2026-10-26/27)

All output below is from the real `zsol` binary (crates/zsol-cli) against the public
endpoint `testnet.zec.rocks:443`. librustzcash pinned at rev `eb38d7a` (same as zcash-devtool).

## Boot check (SRS FR-2.1)
`zsol chain` → chainName `test`, consensusBranchId `77190ad9`, tip 4,544,854,
vendor ECC LightWalletD v0.5.4. The branch is re-derived locally with
`BranchId::for_height(TestNetwork, tip)` and must match, so a stale build or a wrong
endpoint is refused at boot.

## Wallets (SRS FR-2.2)
| Wallet | Accounts | Birthday |
|---|---|---|
| borrower-a | commerce, zsol-attn | 4,544,845 |
| borrower-b | commerce, zsol-attn | 4,544,845 |
| underwriter | treasury | 4,544,846 |

Each directory is mode 0700; `seed.age` (age-armored BIP-39) and `identity.txt` are 0600.
Data lives in `~/.zsol/`, outside the repo.

## Sync (SRS FR-2.3)
`zsol wallet sync --dir ~/.zsol/borrower-a` → fully scanned 4,544,857 = tip, 8 s.

## Funding: pending (no public testnet faucet reachable on 2026-10-26)
