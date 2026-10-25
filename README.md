# zsol — Provably solvent agents, privately.

> **Proof-of-funds attestation protocol for AI agents, native to Zcash.**
> An agent proves *"I control ≥ B zatoshi at height H"* to a counterparty **without revealing
> its identity, its commerce history, or even the attested amount to the public chain.**

**Status:** `pre-alpha` — protocol finalized, implementation underway. See [`docs/SRS.md`](docs/SRS.md).

---

## The problem (real, current, verified-empty lane)

AI agents are becoming economic actors: they hire each other, buy API access, and will soon
hold budgets. But **there is no way for an agent to prove it can pay**:

- Publishing a transparent address leaks the whole history (every inflow/outflow linkable).
- Exchanges demanding "proof of ownership" (EU: above **€1,000** since Aug 2026 — BaFin) today
  force humans into screenshots/custody theater. Agents have no equivalent primitive.
- Counterparty risk between agents is unpriceable: a merchant agent cannot distinguish a
  solvent buyer from a broke one pre-pay.

**Verified-empty lane** (searched Oct 2026): every Zcash x402/MPP facilitator (CipherPay, zpay,
Rill), every FROST custody SDK, and every explorer-MCP solves *payments* or *custody*.
**No privacy-native proof-of-funds primitive exists anywhere** — until zsol.

The naive toolkit (publish balances, reveal full viewing keys, custodial wrappers) all
*destroy* Zcash's core promise. zsol keeps it:

> **Publicly indistinguishable, provably solvent.**

## How it works (protocol summary — full spec: [`docs/SPEC.md`](docs/SPEC.md))

1. An agent keeps a **dedicated attestation sub-account** (`zsol-attn`) inside its own
   non-custodial Zallet wallet — separate from commerce funds.
2. To attest *"≥ B zatoshi at height H"*, the agent performs a **self-spend anchor**: a fully
   shielded `z → z` transfer of ≥ B zatoshi **to the same account**, anchored at H.
   The chain sees just another opaque shielded transaction. Nothing leaks publicly.
3. The agent publishes a signed-off **attestation object** (claim + anchor txid + memo digest).
   The memo inside the anchor carries `b"zs01" + digest(object)` — a one-way cryptographic
   binding produced by the *wallet that controls the notes itself*, impossible to forge
   without spending authority.
4. A verifying agent receives (direct channel) a **scoped viewing key** for the dedicated
   attestation account only, imports it into its own Zallet, and independently scans:
   anchor mined ✓, self-spend shape ✓, outputs ≥ B ✓, live balance ≥ B ✓ → **verified tier**.
   The verifier sees *nothing* about the agent's other accounts or commerce.
5. A public **registry** keeps claim-tier objects (no keys, no balances) with a tamper-evident
   digest chain — discoverability without surveillance.

That is Zcash's core idea — *financial privacy by default, selective transparency on demand* —
turned into a **counterparty-trust primitive for the agentic economy**.

## What we ship

| Component | Crate / App | Purpose |
|---|---|---|
| Protocol spec | [`docs/SPEC.md`](docs/SPEC.md) | Attestation object, anchor shape, scoping tiers, bindings, threat model |
| `zsol-types` | `crates/zsol-types` | Shared serde types + digest/binding logic (golden tests) |
| `zsol-wallet` | `crates/zsol-wallet` | Zallet JSON-RPC client: accounts, UA derivation, self-spend, view-key ops |
| `zsol-registry` | `crates/zsol-registry` | Public append-only claims store (SQLite, digest chain) — real HTTP API |
| `zsol-mcp` | `crates/zsol-mcp` | **MCP gateway for agents** (the agentic core): attest/verify/share tools over stdio **and** streamable HTTP |
| Control plane | `web/` | Next.js 16 (`src/`, App Router), neobrutalism; reads only **real** runtime state |
| Ops | [`docs/OPERATIONS.md`](docs/OPERATIONS.md) + `scripts/` | Z3 testnet bootstrap, faucet ladder, verification gates |

Stack: Rust 1.98 + axum + tokio + rusqlite (crates), Z3 testnet stack (Zebra 7 + Zaino + Zallet)
via Docker, Next.js 16 (Node 24, pnpm). MCP is JSON-RPC 2.0, hand-implemented (stdio + HTTP),
no prototype glue: every path goes to a real Zallet/RPC endpoint (SRS invariant: **no mocks**).

## The demo (all live on Zcash testnet)

- Agent **A** (funded wallet) and agent **B** (empty wallet) request quotes from merchant agent **M**.
- M calls `zsol_verify` on each attestation: A → verified tier (prices a real discount),
  B → claim-only tier (full price) — **priced from cryptographic attestations, not KYC**.
- Operator opens `web/` explorer: registry tier objects only — no amounts, no keys.

## Why the judges (where this came from)

In May 2026 an AI model found the 4-year-old Orchard counterfeit vulnerability; Zcash's
biggest crisis of 2026 was *discovered and resolved with agentic tooling*, and 2026 discourse
largely assumes agents will interact with Zcash. The missing agentic primitive isn't another
payment rail — it's **trust without disclosure**. zsol is that primitive.

- EU proof-of-ownership requirements (BaFin, Aug/Oct 2026) and agent-economy growth make this
  a real-world need, not a toy.
- Built on the Foundation's new stack (Zebra+Zaino+Zallet — `zcashd` is end-of-support),
  zero consensus changes, zero forks.

## License

MIT — see [`LICENSE`](LICENSE).
