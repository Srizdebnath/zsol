# zsol OPERATIONS — testnet runbook (no tribal steps, no fake steps)

> Truths: Z3 README (two-phase testnet boot), Z3 attach-your-own-service docs, SoChain zectest
> explorer, community faucets. Execute gates G0–G4 with real evidence to `docs/evidence/`.

## 0) Toolchain state (verified on build box; CI mirrors)

```
rustc 1.98+  cargo 1.98+        node 24 + pnpm 12            docker 29 + compose 5.5
gh ≥ 2.100 (SSH auth)           git ≥ 2.55                   4 vCPU / 8 GB / ≥ 40 GB free
```

⚠️ Docker root lives on `/` (~29 GB only). Z3 chain data MUST be directed to `/home`
(≥ 350 GB free): `Z3_CHAIN_DATA_PATH=/home/ansh/zcash/z3-data` + `fix-permissions.sh`. Never skip.

## 1) Z3 testnet stack (one-time + start; Z3 repo lives OUTSIDE the zsol repo)

```bash
Z3_ROOT=/home/ansh/zcash/z3
git clone https://github.com/ZcashFoundation/z3 $Z3_ROOT && cd $Z3_ROOT   # T-01 pins a SHA
./scripts/setup-network.sh testnet
export Z3_CHAIN_DATA_PATH=/home/ansh/zcash/z3-data
./scripts/fix-permissions.sh zebra $Z3_CHAIN_DATA_PATH
docker compose --env-file .env.testnet up -d zebra            # phase 1: node only
./scripts/check-zebra-readiness.sh 18080                       # exits when synced (2–12 h)
docker compose --env-file .env.testnet up -d                   # phase 2: full stack + Zallet
```

**Boot rule (never violate):** Zallet before Zebra `/ready` → restart-loop. The poller gates it.

### 1b) Boot-step-0 — Zallet RPC parity pin (G0; hard blocker, SRS FR-7.3)

Curl every verb below against Zallet RPC (`Z3_ZALLET_HOST_RPC_PORT` from `.env.testnet`),
record supported/unsupported in `.env.zallet-pins` (git-ignored), services refuse to start without it:

```
z_getbalance  z_listaccounts  z_getaddressforaccount  z_sendmany
z_getoperationstatus  z_getoperationresult  z_viewtransaction
z_importkey  z_exportkey(offline pin)  getblockchaininfo  getrawtransaction
```

Zallet is beta: some verbs may be msshipped or shape-shift. zsol degrades gracefully & never
guesses a missing verb (ARCHITECTURE A6, SRS FR-2.1).

## 2) Funding ladder (perfect honesty: faucets ARE the path; no shortcuts)

- Rung 1: **ZECpages testnet faucet** (browser PoW → 0.1 TAZ, z-address canned; fine via `zsol-attn`).
- Rung 2: **ZecFaucet.com** community faucet (transparent or z; if transparent received,
  *shielding via normal z_sendmany from that own wallet* is fine; registry of transparent outputs
  is honestly annotated in evidence).
- Rung 3: **Zcash Discord** `#support | #devs` — paste attestor zsol-attn UA; politeness caps; log asks.

Targets (SRS NF-7.1): attestor account 9 ≥ 0.2 TAZ *att* buffer; account 0 untouched by zsol; plus
verifier wallet scan dust + merchant float 0.05 TAZ. All funding txids recorded in evidence.

## 3) zsol services (against running Z3)

```bash
cp .env.example .env # fill from §1-2 artefacts + choice of BINDs
cargo build --release
docker compose -f docker/compose.zsol.yml up -d        # registry (+ cert all attached to z3-testnet net)
cargo run -p zsol-mcp --release    # stdio (default) or:
cargo run -p zsol-mcp --release -- --transport http
claude mcp add zsol --target local -- /home/ansh/zcash/zsol/target/release/zsol-mcp --transport stdio
pnpm -C web install && pnpm -C web build && pnpm -C web start
```

Per SRS open-question 2 (two Zallet instances): spin verifier Zallet with its own `ZALLET_RPC_URL`
(e.g. `verifier-zallet` compose set) — never reuse the attestor's instance.

## 4) Verification gates — G0…G4 (evidence required; nothing verbal)

```
G0  parity pin file + zebra ready + zallet ops green + funding ≥ target   → evidence .env.zallet-pins + balace screenshot
G1  anchor live: zsol-mcp zsol_anchor(claimZat) succeeds; txid on sochain.com/zectest
G2  registry loop: curl POST claim (canonical object b64u) → GET by agent shows entry + chain digest
G3  claude-code agent loop: single prompt triggers prepare→anchor→make→publish unassisted; log it
G4  verifier demo: import_scope + verify call → tier verified; claim-only path reasons typed
     (evidence: full transcript; testnet txids links; pricing diff in merchant policy transcript)
```

Keep in `docs/evidence/`: gates G0–G4 screenshots + txid lines + CI links; image names prefixed
`G0-…`, `G4-…`; **no logs containing addresses/secrets at evidence-level privacy** (CONTRIBUTING).

## 5) Demo hygiene (arena)

- Pre-sync Zebra BEFORE event; pin docker images (`Z3_*_IMAGE` hash pins; no surprise pulls).
- Faucet-queue budget: refresh attestor to ≥ 0.25 TAZ the morning of demo.
- Dry-run one anchor and one publish pre-demo daily (wallet warm-up; registry health 200).
- Never present regtest as testnet. Regtest exists solely for CI-safe fast tests (T-39).

## 6) Forbidden ops repo-wide (mirrors SRS §11)

No transparent payment path; no custody scripts; no engine test scaffolds returning canned data in
runtime crates (`#[cfg(test)]` + `tests/` only); no raw account-0 key export tools; no "temporarily
fake a mined tx" utilities. Anything suspicious: stop, raise issue with SRS thread reference.
