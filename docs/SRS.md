# zsol: Software Requirements Specification

**Version:** 2.0 (2026-10-26) · **Supersedes:** 1.0 (see [`FINDINGS.md`](FINDINGS.md))
**Companions:** [`SPEC.md`](SPEC.md) (wire protocol 0.2) · [`ARCHITECTURE.md`](ARCHITECTURE.md) ·
[`BUILD_PLAN.md`](BUILD_PLAN.md) · [`OPERATIONS.md`](OPERATIONS.md)

> Every requirement has an ID and an acceptance criterion (AC). Work closes only when its AC has
> been executed **on Zcash testnet** and the evidence is in `docs/evidence/`. §10 lists bans, not
> guidelines.

## 1. Product

**One-liner.** zsol lets an AI agent prove *live* solvency to a counterparty without revealing who
it is or what else it owns. The Credit Desk turns that proof into pay-later credit for agents
that hold only ZEC, buying through Bitrefill.

**Target.** Zcash Free Society Hackathon (WebZero × Zcash × Bitrefill, Devcon, Mumbai). The build
day is Wed 2026-11-04; submissions close at 00:00 IST on Nov 5; demo day is Nov 5.
- **Privacy track (Zcash):** the zsol protocol, i.e. anchor + UFVK verification + liveness +
  challenge-response.
- **Agentic Commerce track (Bitrefill):** the Credit Desk, an underwriter agent that buys through
  Bitrefill's MCP/API against a verified attestation and is repaid in shielded ZEC.
- Work done before the event is **disclosed** in the submission (BUILD_PLAN §4).

**Users.** U1 is the borrower/attestor agent and its operator. U2 is the underwriter/verifier
agent and its operator. U3 is a public observer of the registry and explorer. U4 is a judge, who
must be able to run a verification themselves.

## 2. Scope

**In.** Protocol 0.2 end to end on testnet. A light-client backend (primary). A Zallet backend
(secondary, once Zallet supports NU7). Registry. MCP gateway (stdio + streamable HTTP). Credit
Desk with Bitrefill. Live watch. Web UI. Laptop deployment with a public tunnel.
**Out.** Mainnet funds, escrow or locking, zk proofs that avoid key sharing (the roadmap item:
an unbroadcast PCZT proof), cross-pool aggregation, a hosted VPS.

## 3. Functional requirements

### FR-1 Protocol core (`zsol-types`)
- **FR-1.1** Object 0.2, canonical JSON, b64u round-trip. *AC:* golden vectors in `tests/`; the
  SPEC drift test reads SPEC §4 field names.
- **FR-1.2** Frame digest with BLAKE2b-256 personalization `zsol/0.2-frame`. *AC:* a committed
  golden vector, and a cross-language check in which web/TS produces the same digest.
- **FR-1.3** Memo codec `0xFF‖"zs02"‖D`, padded to 512 bytes; the parser reads 37 bytes. *AC:*
  build/parse tests and ≥10k proptest cases.
- **FR-1.4** Presentation message and Ed25519 sign/verify (SPEC §7). *AC:* RFC 8032 test vector
  passes; a tampered nonce, txid or verifierId each fail.
- **FR-1.5** Reason codes (SPEC §10) and tiers as exhaustive enums. *AC:* no wildcard match arms
  (clippy `wildcard_enum_match_arm` denied).

### FR-2 Light wallet backend (`zsol-lightwallet`), primary
- **FR-2.1** Chain client against a lightwalletd-protocol endpoint (TLS). At boot it checks
  `GetLightdInfo.consensusBranchId == 0x77190ad9` and `chainName == "test"` and refuses to start
  otherwise. *AC:* boot log shows the branch; a wrong endpoint produces a typed refusal.
- **FR-2.2** Attestor wallet: seed generated locally and stored `age`-encrypted; ZIP-32 accounts
  `commerce` and `zsol-attn`; Ironwood-receiving UA derivation. *AC:* the seed file is unreadable
  without the identity; restarting the wallet re-derives the same UA.
- **FR-2.3** Sync (compact blocks from birthday to tip) with progress reporting. *AC:* a cold
  sync of `zsol-attn` finishes and reports the height.
- **FR-2.4** Anchor builder: propose → create → broadcast (`SendTransaction`) the self-spend of
  SPEC §5; then wait until mined. *AC:* the txid is visible on a public testnet explorer; the
  memo is decodable by the wallet.
- **FR-2.5** UFVK export, limited to `zsol-attn*` accounts. Requests for `commerce` are refused.
  *AC:* negative test.
- **FR-2.6** Shielded payment from `commerce` with a memo (Credit Desk repayment). *AC:* a mined
  testnet transaction.

### FR-3 Verifier (`zsol-verifier`)
- **FR-3.1** Import a UFVK as a view-only account at `birthday`, in a wallet DB that belongs to the
  verifier. *AC:* this works in a separate data directory with no attestor key material present
  (asserted by test).
- **FR-3.2** Checks C1–C8 (SPEC §6), each returning a typed row. *AC:* a live testnet run
  produces `verified`; each of these is reproduced live and returns its code:
  wrong claim (`claim-exceeds-outputs`), edited agentId (`memo-digest-mismatch`), wrong nonce
  (`presentation-invalid`), UFVK-holder donation forgery (`not-spent-by-account`).
- **FR-3.3** Watch loop (SPEC §8). It emits `live`, `released` and `expired`. *AC:* the attestor
  spends its anchored notes on testnet and the verifier emits `released` within ≤2 blocks of
  the spend being mined.

### FR-4 Registry (`zsol-registry`)
- **FR-4.1** Bump to the 0.2 protocol; the rest of the 0.1 behaviour (dedupe, digest chain,
  watermark, rate limits, size caps) is kept. *AC:* the existing tests pass with 0.2 objects.
- **FR-4.2** Reject key-like fields. *AC:* a POST containing a `ufvk` field returns 400.

### FR-5 MCP gateway (`zsol-mcp`)
- **FR-5.1** Tools. Attestor side: `zsol_health`, `zsol_wallet_status`, `zsol_receive_address`,
  `zsol_anchor`, `zsol_make_attestation`, `zsol_publish`, `zsol_share_ufvk`, `zsol_sign_challenge`.
  Verifier side: `zsol_challenge`, `zsol_verify`, `zsol_watch_status`, `zsol_registry_fetch`.
  *AC:* `claude mcp list` shows all of them over stdio and over HTTP.
- **FR-5.2** Spend tools refuse unless `ZSOL_ALLOW_SPEND=1` and the requested amount is within
  `ZSOL_SPEND_CAP_ZAT`. *AC:* the default run cannot send.

### FR-6 Credit Desk (`zsol-credit`)
- **FR-6.1** Underwriter HTTP API (`/v0/credit/open`, `/challenge`, `/loans/:id`) and MCP tools
  `credit_request` and `credit_repay`. *AC:* a borrower agent completes a loan with no human
  steps beyond spend approval.
- **FR-6.2** Policy file: `ltvBps`, `maxTenorBlocks`, `maxLoanUsd`, `mode` (`test` | `live`),
  reference-rate source. *AC:* the policy is loaded and echoed in `/v0/health`.
- **FR-6.3** Bitrefill client (REST v2 with Bearer key; MCP as an alternative). In test mode it
  allows only `test-gift-card-code`, `test-gift-card-link` and `test-phone-refill` with
  `payment_method=balance`. *AC:* a real Bitrefill invoice ID and redemption payload are stored;
  `test-*-fail` products map to a typed `purchase-failed` error.
- **FR-6.4** Repayment detection through the underwriter's own wallet scan (memo
  `0xFF‖"zsc1"‖loanId`). *AC:* a live testnet repayment moves the loan to `repaid`.
- **FR-6.5** Risk link: `released` moves open loans to `at-risk` and freezes new credit;
  passing `dueHeight` moves them to `defaulted`. *AC:* a live run demonstrates `at-risk`.

### FR-7 Web (`web/`, Next.js 16, neobrutalism)
- **FR-7.1** `/` shows stack status (endpoint, branch, tip, sync lag of each wallet).
- **FR-7.2** `/explorer` lists registry entries with the chain digest recomputed client-side.
- **FR-7.3** `/verify` is a judge self-serve page: paste an object + UFVK and the server-side
  verifier runs C1–C7. C8 is shown as "requires the attestor's live presentation".
- **FR-7.4** `/desk` shows a live loan board and watch events (SSE), with a link to every txid
  on an explorer.
- *AC (all):* every number on screen traces to a live endpoint; `no-mocks.sh` passes.

### FR-8 Ops
- **FR-8.1** A single `zsol up` binary subcommand that starts registry, MCP, credit and web on the
  laptop, plus a `cloudflared` tunnel for the public URLs. *AC:* a cold start to a public URL
  works from a phone on mobile data.
- **FR-8.2** Backend switch `ZSOL_CHAIN=lightwalletd|zebra-zaino` and wallet switch
  `ZSOL_WALLET=light|zallet`. *AC (stretch):* the G-gates pass again on a local Zebra 7 + Zallet
  built from PR #898.
- **FR-8.3** CI: fmt, clippy `-D warnings`, tests, web build, no-mocks lint.

## 4. Non-functional requirements
- **NF-1** An anchor reaches 1 confirmation in ≤ 90 s at p95 on post-NU7 testnet (25 s target
  spacing). Measured values go into the evidence.
- **NF-2** Verifier cold check (import plus sync from birthday) ≤ 60 s for a birthday ≤ 2,000
  blocks old. Measured.
- **NF-3** No address, amount, key or memo in info-level logs; debug output is gated.
- **NF-4** Keys appear only in: the attestor's `age`-encrypted seed file, the attestor wallet DB
  and the verifier's view-only DB. A CI secret scan runs on the repo.
- **NF-5** Every tool fails with a typed error and never panics; there is a 10 s RPC timeout with
  bounded retry.

## 5. Gates (live evidence required)

| Gate | Proves | Evidence |
|---|---|---|
| G0 | Endpoint on NU7 branch; attestor funded from a faucet | `GetLightdInfo` dump; funding txid |
| G1 | Real anchor on testnet | txid plus explorer link; decoded memo |
| G2 | `verified` from a separate verifier DB | full check-row transcript |
| G3 | Negative cases (FR-3.2) each give the right code | four transcripts with txids |
| G4 | Spending the anchored notes makes the watch emit `released` | spend txid; event log with heights |
| G5 | Credit Desk loop: verify → Bitrefill invoice → repayment detected | Bitrefill invoice ID; repayment txid |
| G6 | Public URL reachable; judge self-verifies | tunnel URL screenshot taken from a phone |
| G7 (stretch) | G1–G5 on local Zebra 7 + Zallet (PR #898) | same artefacts |

## 6. Risks

| Risk | Mitigation |
|---|---|
| Testnet faucet unavailable after NU7 | Test the faucet ladder on day 1 (Oct 27). Fallbacks are a Zcash forum/Discord request and asking event mentors. Keep ≥ 1 TAZ buffered by Nov 2. |
| Public lightwalletd is down at the venue | Local Zebra 7 + Zaino on the laptop (synced by Oct 29), plus a phone hotspot. |
| Pre-release librustzcash API churn | Pin the exact `=0.x.y-pre.z` versions used by zcash-devtool; vendor `Cargo.lock`. |
| Bitrefill API key or approval delay | The user creates the account on Oct 27. Test products need only a normal API key and `payment_method=balance`. |
| Venue Wi-Fi blocks the tunnel | Hotspot. A pre-recorded backup video shows the *same* live run with its txids; it is labelled as a recording. |
| Judges dock points for pre-built work | Disclose openly; ship a clearly scoped on-day feature (BUILD_PLAN §4). |

## 7. Open questions
1. ~~Do Bitrefill test products satisfy "use Bitrefill"?~~ **Answered 2026-10-26 (owner): yes.**
2. Which testnet explorer indexes post-NU7 blocks? Verify on Oct 27.

## 8. Changes from 1.0
UIVK → UFVK; Zallet-only → light-first; added C3, C6, C8, the watch loop and the Credit Desk;
web `/desk`; testnet is now post-NU7; dropped unverifiable regulatory claims. Full rationale:
FINDINGS.md.

## 9. Glossary
**Anchor**: the self-spend transaction carrying the binding memo. **Anchored notes**: that
transaction's outputs back to `zsol-attn`. **UFVK**: unified full viewing key (ZIP-316).
**TAZ**: testnet ZEC.

## 10. Invariants (bans)
1. No transparent addresses in any protocol path or example.
2. No custody: the registry and underwriter never hold attestor keys.
3. No mock, fixture or sample data on runtime paths; fixtures live only in `tests/` or
   `#[cfg(test)]`.
4. No invented cryptography; only the primitives in SPEC P3.
5. Never export the `commerce` account's keys, or any spending key, through a tool.
6. The registry never stores or relays viewing keys.
7. Amounts are zatoshi integers on the wire.
8. No requirement counts as done without its AC executed on **testnet**. Regtest results are
   always labelled as regtest.
9. No consensus changes and no forks of dependencies (pinning a pre-release is allowed).
10. Every gate's evidence is committed before anyone claims the gate passed.
