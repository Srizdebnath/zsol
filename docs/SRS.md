# Zsol — Software Requirements Specification

**Version:** 1.0 · **Status:** baseline (deviation discipline applies, §11)
**Companions:** [`SPEC.md`](SPEC.md) (wire protocol) · [`ARCHITECTURE.md`](ARCHITECTURE.md) · [`OPERATIONS.md`](OPERATIONS.md) · [`TODOS.md`](TODOS.md) (50-item implementation contract)

> **How to use:** every functional requirement has an ID and machine-checkable acceptance criteria
> (AC). Work closes by proving ACs, never by claiming intent. §11 invariants are bans — violating
> any of them is a process failure. TODO IDs in `TODOS.md` map to FR IDs in §3.

---

## 1. Product definition

**One-liner.** zsol is a proof-of-funds attestation protocol for AI agents on Zcash: an agent
cryptographically attests "I control ≥ B zatoshi at height H" to counterparties, publicly
indistinguishable on chain, non-custodial, zero leagues from a facilitator, with commerce
history never disclosed.

**Hard requirements from product owner (abstracted):**
0 displayed mocks/fixtures anywhere in runtime; end-to-end real loop on **Zcash testnet**;
every tool available to agents via **MCP**; solo-strong-Rust build; web surface Next.js 16
(`src/`, App Router) with **neobrutalism** design; protocol engineering-grade quality.

**Users:** U1 attesting agent operator (owns a funded Zallet); U2 verifying/merchant agent
(prices counterparties); U3 public observer (registry readers); U4 human operator of either
side; U5 protocol-engineer judges.

## 2. Scope

**In:** attestation protocol (SPEC §1–10) end-to-end on testnet; MCP gateway stdio+HTTP;
registry public claims service; Next.js 16 control plane reading real runtime; CI; runbook.
**Out:** mainnet, custodial wrappers, zk-circuit proofs of aggregated holdings (future),
orchard-specific tests beyond proxy (Ironwood-era pool nuance — SPEC §10), exchange integrations.

## 3. Functional requirements (abbreviated IDs; full AC per FR)

### FR-1 Protocol core (`zsol-types`)
- FR-1.1 Attestation object serde + canonical form + b64u round-trip precisely per SPEC §4. AC: round-trip test + golden vector equal in CI.
- FR-1.2 BLAKE2b-256 `D(object)` w/ domain separation. AC: golden vector test committed.
- FR-1.3 Anchor memo codec per SPEC §5 (`zs01(4)‖ver(1)‖height_LE64(8)‖digest(32)`). AC: parse+build tests; fuzz ≥1k iterations passes.
- FR-1.4 Reason code taxonomy (SPEC §9) as enums with exhaustive conversions. AC: compile-time exhaustive match enforced (`#[deny(unreachable_patterns)]`).
- FR-1.5 Registry chain digest formula (SPEC §7). AC: tamper test (row mutation) flips digest.

### FR-2 Attestor wallet ops (`zsol-wallet`)
- FR-2.1 Zallet config loader (env-driven; boot-pinned verbs honored; refusal modes). AC: `.env.zallet-pins` read at boot; missing verb ⇒ hard refuse with actionable error.
- FR-2.2 JSON-RPC 1.0 client w/ timeouts, idempotent retry, no PII in logs. AC: fault-injection test (timeout) returns typed error; trace shows no addresses.
- FR-2.3 `z_getaddressforaccount` diversified UA derivation. AC: runbook G1 — address re-derives deterministically per tracker.
- FR-2.4 Self-spend anchor composer (`z_sendmany`, 1 output to own UA, ≥ B zat, memo = anchor bytes, FullyShielded policy). AC: dry-run emits exact RPC JSON; live test produces mined tx visible in wallet view.
- FR-2.5 Operation lifecycle: submit → status poll (timeout) → result → confirms-wait. AC: anchor reaches N=1 conf within 35s budget p95 on testnet; timeout path typed.
- FR-2.6 `z_viewtransaction` parser: per-output sums; memo bytes per output. AC: golden tx fixture from testnet replayed in unit test (masked txid ok).
- FR-2.7 Verifier-side uivk import (`z_importkey`) + wallet account scan state summary. AC: import command succeeds; scan count reported (no漏水).
- FR-2.8 View-key scope guard: exports limited to attestation accounts only; refuses `account #0` + master keys. AC: negative test — attempt export of acct 0 ⇒ typed refusal.

### FR-3 Registry (`zsol-registry`)
- FR-3.1 `POST /v0/attestations` (claim tier only; dedupe unique `(agentId, anchorTxid)`). AC: duplicate POST ⇒ 409 with stable reason; first wins.
- FR-3.2 `GET /v0/attestations/:agentId` ascending w/ running chainDigest. AC: GET for 3 entries yields chain-ordered digests matching formula.
- FR-3.3 Validation: canonical JSON only, dust floor, `protocol = "zsol/0.1"` else `protocol-unknown`; well-formed memo binding (never stores uivk). AC: 4 invalid-input negative tests each return the right reason code.
- FR-3.4 Block-diff guard + rate limit + size caps. AC: simulate height regression ⇒ service refuses mutations (`503 chain-descended`); > N req/s ⇒ 429.
- FR-3.5 Persistence SQLite WAL; crash-safe resume; never logs keys. AC: kill -9 during POST; restart; registry consistent; no uivk material anywhere in DB/logs.

### FR-4 MCP gateway (`zsol-mcp`)
- FR-4.1 MCP stdio server: initialize / ping / tools/list / tools/call (full list SPEC §8). AC: official `claude mcp add zsol -- <bin>` -> tools visible; one `tools/call` flow executes.
- FR-4.2 Streamable HTTP server at `ZSOL_MCP_HTTP_BIND` serving same toolset via JSON-RPC 2.0. AC: remote client lists tools and calls `zsol_health`.
- FR-4.3 Attestor tool wiring: prepare → anchor → make → publish (real RPC calls only). AC: integration test on regtest stack produces registry entry + object b64u.
- FR-4.4 Verifier tool wiring: import_scope → verify (all §6 checks) → registry_fetch. AC: verify of valid anchor = verified tier; mismatched claim = claim-exceeds-outputs etc.
- FR-4.5 Dry-run & low-permission default: no spends without explicit `ZSOL_MCP_DRY_RUN=0`; every spend logged at info w/o addresses. AC: default-first run cannot send; explicit run does.

### FR-5 Agents & demo flows
- FR-5.1 Attestor agent completes full loop on testnet (fund → prepare → anchor → make → publish). AC: evidence pack (txid links; sochain zectest snapshot) in `docs/evidence/`.
- FR-5.2 Verifier agent prices two counterparties differently per tier (A verified, B claim-only). AC: policy object in repo; demo transcript shows two different quotes tied to tier outputs.
- FR-5.3 Scope demonstration: per-verifier sub-accounts `zsol-attn-v1/v2`; revocation notes honest. AC: two uivks exported; verifier1 cannot see v2 history (scan count zero).

### FR-6 Web (Next.js 16 App Router, neobrutalism)
- FR-6.1 Scaffold with `src/`, App Router, strict TS; tokens in `globals.css` (3px black borders, hard shadows, mono type). AC: design tokens referenced by every page; build passes.
- FR-6.2 `/` system status: registry health, zebra readiness, zallet pin status, last height. AC: each figure traces to a real endpoint fetch (grep-guarded; no fixture imports).
- FR-6.3 `/explorer`: registry entries via `NEXT_PUBLIC_REGISTRY_URL` (claims only). AC: build-time grep finds zero `mock|fixture|sample` data.
- FR-6.4 `/verify`: pastes object b64u → calls local zsol-mcp HTTP `zsol_verify`; shows tier + per-check list. AC: invalid object → claim-only + reason; valid → verified.

### FR-7 Cross-cutting
- FR-7.1 Single-command dev start (`make up`) booting z3 via Z3 repo + zsol services; .env-driven. AC: fresh-clone runbook completes on dev box call documenting each manual step.
- FR-7.2 CI: fmt/clippy/test workspace, `cargo build --release`, `pnpm -C web build`, docs-lint (bans literals `mock|fixture|sample` under runtime crates/src), plus optional integration profile on regtest. AC: green on push.
- FR-7.3 Boot-step-0 parity pin artifact `.env.zallet-pins` mechanism documented + validated in runbook. AC: runbook step yields the file; services refuse sans it.
- FR-7.4 Evidence pack: G0–G4 gate evidence (txids, screenshots, test links). AC: directory committed (images allowed; no secrets).

## 4. Wire contract truth
`docs/SPEC.md` is the single source of truth. `zsol-types` must round-trip it exactly;
exhaustive serde tests assert field names against a fixtures corpus loaded at test time.

## 5. Performance & capacity (NFR)
- NF-5.1 Anchor round trip (z_sendmany → 1 conf) ≤ 35 s p95 on testnet.
- NF-5.2 Registry p95 < 50 ms claim ops (SQLite, tiny data).
- NF-5.3 Verifier scan cost is bounded by wallet sync; expected figures are measured from real
  runs and recorded in evidence (no fabricated numbers).
- NF-5.4 All services tolerate agent-parallel loop load (qps realistically ≤ 10 for demo sizing).

## 6. Security & privacy (NFR)
- NF-6.1 Threat model (SPEC §10) tasks observable; uivk never stored server-side.
- NF-6.2 No address/amount at info logs; debug gated `ZSOL_DEBUG=1`.
- NF-6.3 Registry rejects proportionate abuse: size caps, rate limits, canonical-only inputs.
- NF-6.4 Secrets from env/`age`-encrypted wallet DB only; zero keys in repo (CI secret scan).
- NF-6.5 Scope/promotion of any shared viewing key documented in every relevant doc.

## 7. Reliability (NFR)
- NF-7.1 Faucet-ladder funding paths; ≥ 0.2 TAZ attestor buffer; ≥ 0.05 TAZ others.
- NF-7.2 Block-diff guard + honest reason codes; SoChain "balances may be wiped" warning absorbed.
- NF-7.3 Crash-safe registry; MCP typed failure modes (never panics).

## 8. Milestones w/ gates
| M | Scope | Gate |
|---|-------|------|
| M0 | Z3 testnet boot + parity pins + funding | G0 evidence (SRS FR-7.3; RUNBOOK §4) |
| M1 | FR-1 complete (types + battery) | cargo test green incl golden vectors |
| M2 | FR-2 complete up to anchor mined | G1 real anchor txid on sochain |
| M3 | FR-3 registry live | G2 claim entry via curl ↔ GET consistent |
| M4 | FR-4 MCP complete both transports | G3 claude-code agent loop runs unassisted |
| M5 | FR-5 demo flows + pricing policy | G4 full live testnet demo transcript |
| M6 | FR-6 web + evidence + tag | v0.1.0 tag; all prior gates stay green |

## 9. Risks
| Risk | Mitigation |
|---|---|
| Zallet beta method drift | boot-step-0 pins + typed refusals |
| Faucet flakiness | 3-rung ladder + Discord ask path |
| Testnet reset | SoChain warning absorbed: regtest for CI-only fast paths; testnet itself is the acceptance chain; storage via z3 named volumes |
| Scope-creep into competitor features (payments/facilitators!) | route away; only attestation scope |
| Solo dev time | docs-only milestone first (this push); todo mapping is the control surface |

## 10. Open questions register (answered 2026-10-25, product owner)
1. Registry exposure: public internet vs local — **answered: local/LAN default; public only with rate limit + size caps** (FR-3.4).
2. Wallet topology for demo: one Zallet multi-account vs two Zallet instances — **answered: two independent Zallet instances for honest multi-party realism** (FR-5.2, demo topology).
3. Demo claim sizes: B = 0.01 TAZ (demo-tier), rescale documented — **answered**.
4. GitHub visibility — **answered: public**.

## 11. Architectural invariants — hard bans (any violation = failed review, not debate)
1. **No transparent addresses** in any protocol path, doc example, or test fixture.
2. **No custodial custody**; Zallet-only key management; registry must never see/hold keys.
3. **No mock/fixture/demo data in runtime paths** — everything real or absent; fixtures only
   inside `#[cfg(test)]` or `tests/`.
4. **No invented cryptography** (no ad-hoc "signatures"); chain+memo+uivk+digest primitives only.
5. **Never export account #0 / master spend keys** via any tool or doc; scoped uivk only.
6. **Registry must not mutate or** trust claims: append-only, dedupe-unique, chain-digest verifiable.
7. **Amounts as zatoshi integers** on wire; UI converts at edges only.
8. **Runs before it's explained:** no FR counts as done without its AC executed on testnet (CI green where applicable).
9. **No consensus changes / no dependency forks**; upstream-first contribution policy.
10. **Every gate G0–G4 documented in `docs/evidence/`** before closure claim; gates run live, never "will run later".
