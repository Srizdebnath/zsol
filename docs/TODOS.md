# zsol Implementation Todos (50) — the 0-mock contract

> Rules echoing SRS §11: everything must end up LIVE on Zcash testnet. Nothing is "done" until its
> gate produces evidence in `docs/evidence/`. Every todo maps to SRS FR ids (parent doc: [`SRS.md`](SRS.md)).
> Fixture files are legal ONLY inside `#[cfg(test)]` or `tests/` — a "mock/fixture/sample" literal in
> runtime paths fails CI (FR-7.2).

## A. Infra & environment (T-01…T-08)

- [ ] **T-01** Clone Z3 & pin SHA; record in `docs/evidence/G0-z3-sha.txt` (SRS FR-7.1).
- [ ] **T-02** Testnet Zebra boot + readiness poller green; record height & time-to-ready (Fr-7.1, OPS §1).
- [ ] **T-03** Full stack incl. Zallet up; cookie auth reality-checked from registry container (OPS §1).
- [ ] **T-04** Boot-step-0 parity pin: verify 11 pinned RPC verbs; write `.env.zallet-pins`; typed-refusal for missing (FR-7.3, FR-2.1).
- [ ] **T-05** Faucet ladder rung 1–3 executed; attestor acct 9 ≥ 0.2 TAZ; all txids in evidence (NF-7.1, OPS §2).
- [ ] **T-06** Verifier Zallet instance + wallet funded 0.05 TAZ (SRS open-question 2, FR-5.2).
- [ ] **T-07** Confirm testnet NU7/Ironwood active on zebra; Sapling + new-pool notes in evidence (SPEC §10 caveat).
- [ ] **T-08** Block-diff guard harness: simulate height regression; registry refuses with 503 chain-descended (FR-3.4).

## B. Protocol core `zsol-types` (T-09…T-16)

- [ ] **T-09** `AttestationObject` serde, canonical(), b64u round-trip + golden vector (FR-1.1).
- [ ] **T-10** BLAKE2b-256 `digest_of` w/ domain sep + length prefix; committed golden vector (FR-1.2).
- [ ] **T-11** Anchor memo codec build/parse (`zs01‖ver‖height_LE64‖digest`), wallet-pad tolerance; fuzz ≥1k iters (FR-1.3).
- [ ] **T-12** `ReasonCode` + tier enums exhaustive; `#[deny(unreachable_patterns)]` enforced (FR-1.4).
- [ ] **T-13** Registry entry + chainDigest genesis continuity formula + tamper test (FR-1.5, FR-3.2).
- [ ] **T-14** Amount validation helpers (dust floor, sanity caps, overflow guards) + tests (P6).
- [ ] **T-15** Fixtures corpus dir `tests/fixtures/*.json` loaded by all serde tests (FR-7.2 integrity bar... wait, fixtures live in tests only) ✅.
- [ ] **T-16** Cross-check: types field names vs SPEC §4 table asserted by a test that reads SPEC and fails on drift (FR-7.2).

## C. Wallet ops `zsol-wallet` (T-17…T-24)

- [ ] **T-17** `ZalletCfg` loader (env + pins file); refuse-with-actionable-error on missing verbs (FR-2.1).
- [ ] **T-18** JSON-RPC 1.0 client: 5s timeouts, idempotent retry, zero PII logging; fault-injection test (FR-2.2).
- [ ] **T-19** `z_getaddressforaccount` diversified UA + tracker dedupe (deterministic re-derive) (FR-2.3).
- [ ] **T-20** Self-spend composer: z_sendmany 1-output to own UA, ≥ B zat, memo anchor bytes, FullyShielded (FR-2.4).
- [ ] **T-21** Op lifecycle: submit→status-poll→result→confirm-wait (N=1), 35s p95 budget, typed timeout (FR-2.5).
- [ ] **T-22** `z_viewtransaction` parser: outputs sums + per-output memo bytes (FR-2.6).
- [ ] **T-23** uivk import (`z_importkey`) + account-scan state summary + counts (FR-2.7).
- [ ] **T-24** Scope guard: export limited to `zsol-attn*` accts; negative test refuses acct-0/master export (FR-2.8, invariant 5).

## D. Registry `zsol-registry` (T-25…T-31)

- [ ] **T-25** Axum endpoints: POST /v0/attestations, GET /v0/attestations/:agentId, GET /v0/health (FR-3.1–3.2).
- [ ] **T-26** SQLite WAL store + unique (agent,txid) + chain digest in transaction (FR-3.1).
- [ ] **T-27** Strict validation: canonical-only, dust floor, protocol tag, memo well-formedness; **never persist uivk** (FR-3.3, invariant 6).
- [ ] **T-28** Digest chain continuity; mutation-tamper test flips chain digest (FR-3.2).
- [ ] **T-29** JSON logs w/ req_id; no PII/addresses/amounts at info (NF-6.2).
- [ ] **T-30** Rate limit + size caps + block-diff guard wiring (FR-3.4).
- [ ] **T-31** `docker/compose.zsol.yml` attach to `z3-testnet` network (read-only cookie volume; OPS §3) (FR-7.1).

## E. MCP gateway `zsol-mcp` (T-32…T-40)

- [ ] **T-32** stdio JSON-RPC 2.0 MCP: initialize/ping/tools/list/tools/call; agent-visible from Claude Code (FR-4.1).
- [ ] **T-33** Streamable HTTP bind; remote client calls tools over JSON-RPC (FR-4.2).
- [ ] **T-34** Attestor tools wired real: prepare/anchor/make/publish → registry POST (FR-4.3).
- [ ] **T-35** Verifier tools wired real: import_scope/verify/registry_fetch (all §6 checks) (FR-4.4).
- [ ] **T-36** `share_scope` guard: refuses acct-0/master; per-verifier sub-account derivation; honest revocation notes (FR-4.4, invariant 5, FR-5.3).
- [ ] **T-37** Dry-run default `ZSOL_MCP_DRY_RUN=1` surfaces plans w/o sends (later explicit=0) (FR-4.5).
- [ ] **T-38** ReasonCode passthrough + no-panic paths; timeout taxonomy (FR-4.3/4.4 ops).
- [ ] **T-39** Regtest-profile integration test (z3 `regtest-init.sh`) exercising full MCP loop in CI (DOCUMENTED regtest; never marketed as testnet) (FR-4.3).
- [ ] **T-40** Agent runbook: `claude mcp add` JSON + prompt batch (FR-5.2 demo), versioned in repo.

## F. Agents & demo (T-41…T-45)

- [ ] **T-41** Demo topology doc: attestor/verifier/operator mapping to two Zallets (FR-5.2, OPS §3 note).
- [ ] **T-42** Merchant pricing policy object (verified-tier discount vs claim-only) as repo config (FR-5.2).
- [ ] **T-43** LIVE testnet demo run fully executed; G4 evidence incl. two txid links + quotes (FR-5.1–5.2).
- [ ] **T-44** Scope demo: `zsol-attn-v1`/`v2` per-verifier keys; verifier1 cannot see v2 history (FR-5.3).
- [ ] **T-45** Honesty annex: leak-surface doc (what uivk sharing exposes; what registry cannot know) shipped in docs (SPEC §10).

## G. Web control plane (T-46…T-48)

- [ ] **T-46** Next 16 scaffold: `src/` App Router, strict TS, neobrutalism tokens in `globals.css` (FR-6.1).
- [ ] **T-47** `/` dashboard + `/explorer` fetching ONLY real endpoints (`NEXT_PUBLIC_REGISTRY_URL`, zebra ready path, pins status) (FR-6.2, FR-6.3).
- [ ] **T-48** `/verify` console → zsol-mcp HTTP `zsol_verify`; tier + per-check list display; no fake data (FR-6.4).

## H. Ship & evidence (T-49…T-50)

- [ ] **T-49** CI on push: fmt/clippy/test/build (`cargo`), `pnpm -C web build`, docs-lint banning mock literals in runtime crates (FR-7.2).
- [ ] **T-50** Tag `v0.1.0` + evidence pack committed (G0–G4 artifacts) + one-pager (README addendum) ready for judges/ZCG follow-up (FR-7.4, M6).
