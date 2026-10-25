# zsol Architecture

## 1. Component map (end-to-end, real processes)

```
┌──────────────────────────────────── Host (linux/docker) ────────────────────────────────────┐
│                                                                                            │
│  ── Z3 TESTNET STACK (ZcashFoundation/z3, compose; ext networks z3-testnet{,-payments}) ──  │
│  ┌────────────────┐     chain JSON-RPC :18232 (cookie)      ┌────────────────────────────┐ │
│  │     Zebra 7    │◄───────────────────────────────────────►│  Zallet ATTESTOR (beta)    ││
│  │   full node    │     getblockchaininfo / getrawtransaction│ accts: #0 commerce         ││
│  └────────────────┘                                          │        #9 zsol-attn        ││
│                                                              │  z_sendmany  z_importkey   ││
│                                                              │  z_getaddressforaccount    ││
│                                                              └──────────┬─────────────────┘│
│                                                              ┌──────────┴─────────────────┐│
│                                                              │  Zallet VERIFIER (2nd)     ││
│                                                              │ (separate instance/pki)    ││
│                                                              └──────────┬─────────────────┘│
│                                                                         │                  │
│  ┌────────────────────────┐  tools/call via JSON-RPC 2.0  ┌─────────────┴───────────────┐ │
│  │  Coding Agent (LLM)    │◄────────────────────────────► │  zsol-mcp (stdio + HTTP)   │ │
│  │  Claude Code / Codex   │                                │ tools: anchor/verify/share  │ │
│  └───────────────────────►│── z_sendmany (real shielded)──►│ (wraps zsol-wallet ops)     │ │
│                           │   through per-role zallet      └─────────────┬───────────────┘ │
│  ┌────────────────────────┴─────────────────────────┐                   │                 │
│  │  zsol-registry (axum + SQLite)                    │                   │                 │
│  │  POST /v0/attestations  GET /v0/attestations/:id  │◄── publish/fetch ─┘                 │
│  │  GET /v0/health        digest-chained, no keys    │                                     │
│  └────────────────────────────┬──────────────────────┘                                     │
│                               │ claim tier only                                            │
│                   ┌───────────┴──────────────┐                                              │
│                   │  web (Next.js 16)        │  /  /explorer  /verify                       │
│                   │  neobrutalism UI         │  reads registry + MCP /health                │
│                   └──────────────────────────┘                                              │
└─────────────────────────────────────────────────────────────────────────────────────────────┘
              evidence trail: docs/evidence/* (txid links + sochain screenshots)
```

**Trust boundaries (process → key/account access):**

| Process | Access | Trust ceiling |
|---|---|---|
| Zebra | consensus, chain truth | FULL — only authority on what is/mined |
| POSSESSED BY attestor Zallet | spender keys: acct 0 + 9 | never leaves; MCP just calls RPC |
| VERIFIER Zallet | its own keys + imported uivk | sees attestor acct 9 view only |
| zsol-mcp | Zallet RPC URLs + tokens | NO arbitrary authority; typed tools only |
| zsol-registry | its sqlite + HTTP | claim-tier storage; **never keys** |

## 2. Life of an attestation (canonical EN cycle)

1. OPERATOR sets `ZSOL_AGENT_ID=m-...`, funds acc 9 with TAZ (faucet ladder, OPERATIONS §2).
2. AGENT (via zsol-mcp): `zsol_prepare_anchor(claimZat)` → budget check vs `z_getbalance`.
3. AGENT: `zsol_anchor(claimZat)` → Zallet `z_sendmany` self-spend (account 9 → own UA, ≥ B,
   memo = zs01‖ver‖height‖digest; **FullyShielded** policy). Waiting for z_getoperationresult; confirms 1.
4. AGENT: `zsol_make_attestation(claimZat)` → builds object (SPEC §4) using the anchor data +
   recomputed `D(object)` binding against the *actual memo bytes* from `z_viewtransaction`.
5. AGENT: `zsol_publish(object)` → registry (claim tier).
6. AGENT lands peer: `zsol_share_scope(peer)` → per-verifier sub-account uivk only.
7. VERIFIER AGENT: `zsol_import_scope(uivk)` into its own Zallet → `zsol_verify(object)` —
   executes all SPEC §6 checks → tier/claims (docs/SRS.md FR-4.4).
8. Operator/merchant: quotes/tiers per VERIFY result (verified claim-only).
9. `zsol_registry_fetch(agentId)` for cross-sees claims w/o keys; web explorer shows tiers.

## 3. Data stores

| Store | Contents | Owner | Durability |
|---|---|---|---|
| Zebra state | chain, mempool, ledger | z3 volume | resyncable |
| Zallet attestor DB | (encrypted) spend keys, spend history accts 0&9 | z3 zallet volume | BACK UP ONLY |
| Zallet verifier DB | own keys + imported uivk state | separate zallet setup | BACK UP |
| zsol-registry DB | claims + chain digest | zsol volume | resume-safe (WAL) |
| MCP runtime | none persisted |— | stateless live |

## 4. Failure matrix (compressed)

| # | Failure | Detection | Recovery |
|---|---------|-----------|----------|
| F1 | Zallet unreachable | RPC timeout (5s) | typed refusal; agent retries w/ backoff |
| F2 | Zebra height regression (testnet bork) | getblockchaininfo policy | registry mutations refuse (503 chain-descended) |
| F3 | Faucet gap | z_getbalance < B+floor | OPERATIONS ladder, rung 2-3 |
| F4 | Agent tries to share acct 0 key | tool guard | typed refusal (FR-2.8) |
| F5 | Testnet restart during demo | evidence pack anchor | regtest fallback documented (never presented as testnet) |
| F6 | Registry DDoS | rate limiter | 429 + wait; demo sizing keeps q ≤ 10 |

## 5. Design decisions (ADR digest)

- **A1 attest=anchor(self-spend) not escrow.** Requires no consensus changes; works TODAY.
- **A2 binding via anchor memo digest not signatures.** Zcash has no message-signing standard;
  memos are wallet-authoritative bytes readable via uivk (SPEC P3).
- **A3 dedicated account for scoping.** uivk is per-account — separate scope bucket is the honest
  unit; per-verifier sub-accounts = finer ceiling (LEAST PRIVILEGE by account construction).
- **A4 two Zallet instances for demo honesty.** Same-wallet verification is degenerate (would
  trivially see its own object) — multi-party realism requires actual independent scan stacks.
- **A5 registry stores claim tier, verification is verifier-local.** No server-side truth.
- **A6 MCP hand-rolled JSON-RPC 2.0.** Zero-hallucination dependency surface; byte-cost small;
  full control over deadline/timeouts. (Upgrade path to `rmcp` crate noted as FUTURE work.)

## 6. Extension roadmap (post-v0.1; NOT current scope)

- zk-circuit proofs of aggregated holdings (avoid uivk sharing altogether)
- Per-pool fungibility nuance (Sapling vs Ironwood-pool) literature
- Attestation marketplaces / reputational aggregation across verifiers
- EVM-side cross-chain attestation relays (with/with no bridges).
