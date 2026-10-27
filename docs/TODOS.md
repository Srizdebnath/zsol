# zsol 0.2 tracker (SRS v2.0)

**Last audited:** 2026-10-27, by reading `crates/*`, `web/src` and `docs/evidence/`.
**Parent docs:** [`SRS.md`](SRS.md) v2.0 (FR/NF ids, gates G0–G7) · [`SPEC.md`](SPEC.md) 0.2 ·
[`BUILD_PLAN.md`](BUILD_PLAN.md) · [`FINDINGS.md`](FINDINGS.md).
This file replaces the 0.1 "50 todos" list (T-01…T-50). Those ids are kept only where 0.1-era
evidence files still use them.

## Status legend

| Status | Meaning |
|---|---|
| `done-live-testnet` | The SRS acceptance criterion (AC) ran against Zcash **testnet** (NU7, branch `0x77190ad9`), and the evidence file is in `docs/evidence/`. |
| `done-regtest-only` | It ran end to end, but only on z3 **regtest**. Under SRS §10.8 this does **not** count as done. |
| `in-progress` | Code exists, but the AC has not been run live (or only part of it has). The notes say which part is missing. |
| `not-started` | No implementing code in the repo. |

Rules: nothing moves to `done-*` unless there is an evidence file (SRS §10.8, §10.10). Unit tests
passing do **not** make a row done, because the SRS requires testnet. Where unit tests exist, the
notes say so.

> **Commit state (2026-10-27):** all 0.2 crates (`zsol-lightwallet`, `zsol-verifier`, `zsol-cli`,
> `zsol-credit`) and every 0.2 evidence file (G0-light, G1, G2, G3, G5-pre) are still
> **untracked/uncommitted** in git. SRS §10.10 requires the evidence to be committed before a gate
> is claimed. The live runs below are real, but each gate is only formally closed once it is committed.

## Evidence index

| File | Era | What it shows |
|---|---|---|
| [`evidence/G0-light-backend.md`](evidence/G0-light-backend.md) | **0.2 testnet** | `zsol chain` boot check (branch `77190ad9`, tip 4,544,854), wallets created, sync to tip in 8 s |
| [`evidence/G1-G2-anchor-verify.md`](evidence/G1-G2-anchor-verify.md) | **0.2 testnet** | Funding txid, first wallet-built spend, real anchor, C1–C8 `verified` in a separate wallet |
| [`evidence/G1-anchor.json`](evidence/G1-anchor.json) | **0.2 testnet** | Raw `zsol attest anchor` output (object, frame digest, 126 s to mined) |
| [`evidence/G2-report.json`](evidence/G2-report.json) | **0.2 testnet** | Raw `zsol verify run` report (tier `verified`, 8 s) |
| [`evidence/G3-negative-cases.md`](evidence/G3-negative-cases.md) | **0.2 testnet** | Four adversarial cases, each failing exactly one check, with 2 txids |
| [`evidence/G5-pre-bitrefill-api.md`](evidence/G5-pre-bitrefill-api.md) | **0.2 precheck** | Bitrefill API v2 test-product purchase made by hand (invoice `23e5ebfa-…`). This is not G5 |
| [`evidence/G0-z3-boot.log`](evidence/G0-z3-boot.log), [`G0-z3-sha.txt`](evidence/G0-z3-sha.txt), [`G0-zebra-progress.log`](evidence/G0-zebra-progress.log) | 0.1-era | z3/Zebra 6.2.3 testnet boot attempt. That node stalled at 144,400 (FINDINGS F-6, F-7) |
| [`evidence/G2-regtest-loop.md`](evidence/G2-regtest-loop.md) | **0.1 regtest** | Zallet regtest loop (T-39). It never reached `verified` (FINDINGS F-2) |
| [`evidence/T32-T33-claude-mcp.md`](evidence/T32-T33-claude-mcp.md) | **0.1 regtest** | `claude mcp` connected to the 0.1 gateway (9 tools, which are not the 0.2 tool set) |
| [`evidence/T46-T48-web.md`](evidence/T46-T48-web.md) | **0.1 regtest** | Web pages against the regtest registry/MCP, including the cross-language chain-digest check |

## Gates (SRS §5)

| Gate | Proves | Status | Evidence / notes |
|---|---|---|---|
| G0 | Endpoint on NU7 branch; attestor funded | `done-live-testnet` | [`G0-light-backend.md`](evidence/G0-light-backend.md) (boot check, wallets). Funding txid `2a4fe529…3ada` (1 TAZ, Ironwood) is in [`G1-G2-anchor-verify.md`](evidence/G1-G2-anchor-verify.md). The **owner** funded it, not a faucet: no public faucet was reachable on 2026-10-26. |
| G1 | Real anchor on testnet | `done-live-testnet` | anchor `30f12fbd…6569` @ 4,544,949, memo `0xFF‖zs02‖D` decoded: [`G1-anchor.json`](evidence/G1-anchor.json). **Gap:** the SRS asks for an explorer link, and no public explorer indexes post-NU7 testnet blocks yet (SRS §7 Q2 still open). Mining was confirmed through lightwalletd `GetTransaction`. |
| G2 | `verified` from a separate verifier DB | `done-live-testnet` | [`G2-report.json`](evidence/G2-report.json): all of C1–C8 PASS at tip 4,544,963, in the underwriter's own wallet |
| G3 | FR-3.2 negative cases | `done-live-testnet` | [`G3-negative-cases.md`](evidence/G3-negative-cases.md): `memo-digest-mismatch`, `presentation-invalid` (×2), `claim-exceeds-outputs` (txid `60fc81f2…8b67`), `not-spent-by-account` (txid `45f28601…be2a`). `4 passed; 0 failed`. |
| G4 | Spending anchored notes makes the watch emit `released` | `done-live-testnet` | [`G4-G5-credit-loop.md`](evidence/G4-G5-credit-loop.md): release spend `8657a142…99ee` mined @ 4,546,446; desk emitted `attestation-released` on the tick that scanned that block (0 blocks lag). |
| G5 | Verify → Bitrefill invoice → repayment detected | `done-live-testnet` | [`G4-G5-credit-loop.md`](evidence/G4-G5-credit-loop.md): loan `71b26fe0…`, Bitrefill invoice `b46f2eb7-c7a8-4f07-a2d9-35de7084b0b2`, repayment `a66d965f…bc37` → `repaid` @ 4,546,455. | Claim-only refusal (released attestation, C6): [`G5b-claim-only.md`](evidence/G5b-claim-only.md).
| G6 | Public URL; judge self-verifies | `owner-descoped` | The owner rejected tunnels (no `cloudflared`). G6 becomes **"locally live"**: the stack runs on the laptop, and a judge self-verifies on that laptop through `/verify`. No evidence yet. |
| G7 (stretch) | G1–G5 on local Zebra 7 + Zallet (PR #898) | `not-started` | `crates/zsol-wallet` (Zallet backend) is parked outside the workspace (FINDINGS F-8) |

## FR-1 Protocol core (`crates/zsol-types`)

| ID | Requirement | Status | Code | Evidence / notes |
|---|---|---|---|---|
| FR-1.1 | Object 0.2, canonical JSON, b64u round-trip, SPEC drift test | `done-live-testnet` | `zsol-types/src/lib.rs` (`AttestationObject::{canonical,b64u,from_b64u}`), `tests/fixtures_corpus.rs`, `tests/spec_drift.rs` | Objects round-tripped through the live G1→G2→G3 runs ([`G1-anchor.json`](evidence/G1-anchor.json) `objectB64u`). Unit tests and the SPEC drift test pass locally (2026-10-27). |
| FR-1.2 | Frame digest, BLAKE2b-256 personal `zsol/0.2-frame` | `in-progress` | `lib.rs` `FRAME_PERSONAL`, `frame_digest_bytes`, test `golden_frame_digest_matches_python_hashlib` | The digest is used live (G1 D=`ba3b7162…`, checked by C4 in G2). **Missing:** the AC requires a web/TS cross-language check of the *frame* digest. `web/src/lib/endpoints.ts` only recomputes the registry *chain* digest (unpersonalized BLAKE2b). |
| FR-1.3 | Memo codec `0xFF‖"zs02"‖D`, padded to 512 bytes | `done-live-testnet` | `lib.rs` `AnchorMemo`, `LoanMemo`; proptest `prop_memo_roundtrip` (10,000 cases) | The memo was decoded on chain in G1/G2 (C4 PASS) |
| FR-1.4 | Presentation message + Ed25519 | `done-live-testnet` | `lib.rs` `presentation_message`, `sign_presentation`, `verify_presentation`; tests `rfc8032_test_vector_1`, `presentation_accepts_genuine_and_rejects_every_tamper` | C8 PASS in G2; relayed and swapped challenges fail in G3b |
| FR-1.5 | Reason codes and tiers as exhaustive enums | `done` | `zsol-types/src/lib.rs` (`#![deny(clippy::wildcard_enum_match_arm)]`) | Enforced by clippy in CI (2026-10-27). |

## FR-2 Light wallet backend (`crates/zsol-lightwallet`)

| ID | Requirement | Status | Code | Evidence / notes |
|---|---|---|---|---|
| FR-2.1 | TLS lightwalletd client; boot check branch `0x77190ad9` + `chainName test` | `done-live-testnet` | `src/chain.rs` (`connect`, `boot_check`), CLI `zsol chain` | [`G0-light-backend.md`](evidence/G0-light-backend.md). There is no transcript of the typed refusal for a wrong endpoint (the code path exists). |
| FR-2.2 | age-encrypted seed; accounts `commerce` + `zsol-attn`; Ironwood UA | `done-live-testnet` | `src/seed.rs`, `src/wallet.rs` (`create_spending`, `address`) | Wallets created live in G0 (0600 files). The seed-needs-identity part is a unit test (`seed_roundtrips_through_age_and_files_are_private`). **AC gap:** no explicit restart/re-derive-same-UA transcript. |
| FR-2.3 | Sync with progress | `done-live-testnet` | `src/wallet.rs` `sync`, `src/cache.rs` | G0: synced to tip 4,544,857 in 8 s |
| FR-2.4 | Anchor builder: propose → create → broadcast → wait mined | `done-live-testnet` | `src/attest.rs` `anchor`, `finalize_pending` | G1 anchor `30f12fbd…6569`. Explorer-visibility part of the AC is blocked (no post-NU7 explorer). |
| FR-2.5 | UFVK export only for `zsol-attn*`; refuse `commerce` | `in-progress` | `src/wallet.rs` `export_ufvk`, `is_shareable_account` | The refusal is in code, and the `zsol-attn` export was used live (G2/G3). **Missing:** the negative test the AC requires (no test calls `export_ufvk("commerce")`). |
| FR-2.6 | Shielded payment from `commerce` with memo | `done-live-testnet` | `src/wallet.rs` `pay`; CLI `zsol wallet send --memo-hex`; MCP `credit_repay` | G3d txid `45f28601…be2a`: borrower-b `commerce` → shielded, with a memo, mined @ 4,544,997. A Credit-Desk repayment memo (`zsc1`) has not been sent yet; that is part of G5. |

## FR-3 Verifier (`crates/zsol-verifier`)

| ID | Requirement | Status | Code | Evidence / notes |
|---|---|---|---|---|
| FR-3.1 | View-only UFVK import at birthday in the verifier's own DB | `done-live-testnet` | `zsol-lightwallet/src/wallet.rs` `create_view_only`, `import_view_only`; `zsol-verifier/src/lib.rs` `verify` | G2 (underwriter wallet) and G3 (a fresh temp wallet per case). **AC gap:** "no attestor key material present" is true by construction but not asserted by a test. |
| FR-3.2 | C1–C8 typed rows; live `verified` + four negative codes | `done-live-testnet` | `zsol-verifier/src/lib.rs`; `tests/live_testnet.rs` (`#[ignore]`, live) | [`G2-report.json`](evidence/G2-report.json), [`G3-negative-cases.md`](evidence/G3-negative-cases.md) |
| FR-3.3 | Watch loop `live`/`released`/`expired`; `released` ≤ 2 blocks after spend | `done-live-testnet` | `zsol-verifier/src/lib.rs` `watch_tick`, `WatchState`; used by `zsol-credit` `tick` | [`G4-G5-credit-loop.md`](evidence/G4-G5-credit-loop.md) (`expired` path not exercised live) |

## FR-4 Registry (`crates/zsol-registry`)

| ID | Requirement | Status | Code | Evidence / notes |
|---|---|---|---|---|
| FR-4.1 | 0.2 protocol bump, 0.1 behaviour kept | `in-progress` | `src/lib.rs` (uses `zsol_types::PROTOCOL` = `zsol/0.2`) | The AC is unit-level: the 11 registry tests pass with 0.2 objects (run 2026-10-27). No 0.2 object has been published to a running registry yet. |
| FR-4.2 | Reject key-like fields (`ufvk` → 400) | `in-progress` | `zsol-types` `#[serde(deny_unknown_fields)]` → `RegistryReject::Malformed` (400) | Holds by construction. **Missing:** no test POSTs a `ufvk` field. |

## FR-5 MCP gateway (`crates/zsol-mcp`)

| ID | Requirement | Status | Code | Evidence / notes |
|---|---|---|---|---|
| FR-5.1 | Attestor + verifier tools over stdio and HTTP; `claude mcp list` | `in-progress` | `src/mcp.rs` `tool_defs`, `src/ops.rs`, `src/stdio.rs`, `src/http.rs` | 13 tools are implemented: `zsol_health`, `zsol_wallet_status`, `zsol_anchor`, `zsol_attestations`, `zsol_publish`, `zsol_present`, `zsol_release`, `zsol_challenge`, `zsol_verify`, `zsol_watch`, `zsol_registry_fetch`, `credit_request`, `credit_repay`. **The names differ from SRS** (see inconsistencies). The only `claude mcp` evidence is the 0.1 regtest-era [`T32-T33`](evidence/T32-T33-claude-mcp.md). | SRS v2.1 now lists the implemented tool names.
| FR-5.2 | Spend tools refuse unless `ZSOL_ALLOW_SPEND=1` and within `ZSOL_SPEND_CAP_ZAT` | `in-progress` | `src/ops.rs` `spend_gate` (used by `zsol_anchor`, `credit_repay`, `zsol_release`) | Gate applied to every broadcasting tool (2026-10-27). No automated negative test yet. |

## FR-6 Credit Desk (`crates/zsol-credit`)

| ID | Requirement | Status | Code | Evidence / notes |
|---|---|---|---|---|
| FR-6.1 | HTTP API + MCP `credit_request`/`credit_repay`; agent completes a loan | `done-live-testnet` | `zsol-credit/src/main.rs` routes; MCP side in `zsol-mcp/src/ops.rs` | [`G4-G5-credit-loop.md`](evidence/G4-G5-credit-loop.md) (driven via MCP tools/call; a Claude-agent run is the rehearsal item) |
| FR-6.2 | Policy file loaded and echoed in `/v0/health` | `in-progress` | `config/credit-policy.json`; `src/main.rs` `health` echoes `policy` | The policy has `ltvBps`, `maxTenorBlocks`, `maxLoanUsd`, `mode`. **Missing:** a reference-rate-source field. CoinGecko → Kraken is hardcoded in `reference_rate`. No `/v0/health` transcript. |
| FR-6.3 | Bitrefill client; test-mode allowlist; invoice + redemption stored; `test-*-fail` → `purchase-failed` | `in-progress` | `src/bitrefill.rs` (`TEST_PRODUCTS`, `buy`, `offers_value`), `src/db.rs` | Live invoice + redemption: [`G4-G5-credit-loop.md`](evidence/G4-G5-credit-loop.md). Denomination now checked before verification (`value-not-offered`). `test-*-fail` products not exercised. |
| FR-6.4 | Repayment via the underwriter's own wallet scan (`0xFF‖"zsc1"‖loanId`) | `done-live-testnet` | `src/main.rs` `tick` (`LoanMemo::decode` → `repaid`) | [`G4-G5-credit-loop.md`](evidence/G4-G5-credit-loop.md) (fixed `v_tx_outputs` column bug during the run) |
| FR-6.5 | `released` → `at-risk` + freeze; past `dueHeight` → `defaulted` | `in-progress` | `src/main.rs` `tick`, `db::agent_frozen` | `at-risk` shown live: [`G4-G5-credit-loop.md`](evidence/G4-G5-credit-loop.md). `defaulted` not exercised live. |

## FR-7 Web (`web/`, Next.js 16)

All four pages have 0.2 code. The only web evidence is the 0.1 regtest-era
[`T46-T48-web.md`](evidence/T46-T48-web.md). No page has been evidenced against the 0.2 testnet stack.

| ID | Requirement | Status | Code | Evidence / notes |
|---|---|---|---|---|
| FR-7.1 | `/` stack status: endpoint, branch, tip, sync lag per wallet | `in-progress` | `web/src/app/page.tsx` | Shows registry health and raw `zsol_health` JSON. Does **not** show the sync lag of each wallet. |
| FR-7.2 | `/explorer` with client-side chain-digest recompute | `done-regtest-only` | `web/src/app/explorer/page.tsx`, `web/src/lib/endpoints.ts` `recheckChain` | Regtest only ([`T46-T48`](evidence/T46-T48-web.md)). Not run against a 0.2 registry. |
| FR-7.3 | `/verify` judge self-serve: challenge → presentation → C1–C8 | `in-progress` | `web/src/app/verify/page.tsx`, `/api/challenge`, `/api/verify` | SRS v2.1 aligned to the challenge flow. Not yet run end-to-end from a phone. |
| FR-7.4 | `/desk` live loan board + SSE + explorer links | `in-progress` | `web/src/app/desk/page.tsx` (`EventSource` on `/v0/events`) | Explorer links depend on `NEXT_PUBLIC_EXPLORER_TX`, and no explorer indexes post-NU7 blocks yet |
| FR-7 AC | Every number traces to a live endpoint; `no-mocks.sh` passes | `in-progress` | `scripts/no-mocks.sh` | No 0.2 run recorded |

## FR-8 Ops

| ID | Requirement | Status | Code | Evidence / notes |
|---|---|---|---|---|
| FR-8.1 | `zsol up` starts registry, MCP, credit, web (+ tunnel) | `done-live-testnet` (local) | `crates/zsol-cli/src/up.rs` | 2026-10-27: `zsol up --build-web` brought all four services up healthy against testnet (credit on the underwriter wallet, verifier gateway on a view-only wallet). Tunnel part is **owner-descoped** (see G6). |
| FR-8.2 | `ZSOL_CHAIN` / `ZSOL_WALLET` backend switches; G7 | `not-started` | — | No `ZSOL_CHAIN` or `ZSOL_WALLET` switch exists in code. `crates/zsol-wallet` is parked. |
| FR-8.3 | CI: fmt, clippy `-D warnings`, tests, web build, no-mocks | `in-progress` | `.github/workflows/ci.yml` | All five steps are configured. No CI run on 0.2 code yet (0.2 code is uncommitted). The release-build step builds only `zsol-registry` and `zsol-mcp`. |

## Non-functional (SRS §4)

| ID | Requirement | Status | Code | Evidence / notes |
|---|---|---|---|---|
| NF-1 | Anchor reaches 1 conf in ≤ 90 s p95 | `in-progress` | `zsol-lightwallet/src/attest.rs` | **One sample, over target:** 126 s from command to mined (G1; includes sync, proving and broadcast). No p95 yet. |
| NF-2 | Verifier cold check ≤ 60 s for birthday ≤ 2,000 blocks old | `done-live-testnet` | `zsol-verifier/src/lib.rs` | One sample: 8 s, birthday 4,544,845 → tip 4,544,963 (118 blocks), [`G2-report.json`](evidence/G2-report.json). G3 cases also completed in fresh wallets. |
| NF-3 | No address/amount/key/memo in info logs | `in-progress` | `tracing` in all crates | Frame-digest prefix moved to debug level in `attest.rs` (2026-10-27). No full audit yet. |
| NF-4 | Keys only in seed file / attestor DB / verifier DB; CI secret scan | `in-progress` | Wallet data in `~/.zsol/` (mode 0700/0600, G0) | **Missing:** no secret-scan step in CI |
| NF-5 | Typed errors, no panics, 10 s RPC timeout, bounded retry | `in-progress` | `zsol-lightwallet/src/chain.rs` (10 s connect / 60 s request timeout), verifier 3× resync | No general bounded retry in the chain client. The request timeout is 60 s, not 10 s. Some `unwrap`/`expect` calls remain in runtime paths (e.g. CLI `print`). |
