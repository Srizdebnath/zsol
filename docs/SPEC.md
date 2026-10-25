# zsol Protocol Specification

**Version:** 0.1.0 · **Asset:** ZEC (Sapling/Ironwood shielded pools), unit **zatoshi** = 10⁻⁸ ZEC
**Stack of record:** Z3 testnet (Zebra 7, Zaino, Zallet beta) · **Status:** implementation-reference

---

## 0. Purpose

Define a **proof-of-funds attestation** primitive for AI agents: agent A proves to counterparty
agent B that *"I control ≥ B zatoshi at height H"* — with **public chain indistinguishability**
and **no disclosure of identity/commerce history**. Companion: [`SRS.md`](SRS.md) (requirements),
[`ARCHITECTURE.md`](ARCHITECTURE.md) (internals).

## 1. Non-negotiable principles

| # | Principle | Prevents |
|---|-----------|----------|
| P1 | **Shielded-only payment/attestation path.** Anchors are fully-shielded z→z transfers. No transparent addresses anywhere in protocol paths. | Public linkage of attestation identity. |
| P2 | **Non-custodial.** All keys live in the agent's own Zallet wallet. Nothing else holds key material. Registry stores claims only, **never** keys. | Registry/server compromise = fund theft. |
| P3 | **No invented cryptography.** Zcash has no unified message-signing standard; zsol does NOT introduce signatures outside consensus. Binding is achieved with *existing* primitives: memos inside encrypted outputs (encrypted at rest; readable only by viewing-key holders) + one-way digests + viewing-key scans. | Spec hallucination → unimplementable. |
| P4 | **Dedicated attestation account.** All anchors live in a ZIP-32 sub-account (`zsol-attn`, default #9) separate from the agent's commerce account (#0). Scope of any shared viewing key is bounded to attestation history only. | Commerce graph leak via over-broad key sharing. |
| P5 | **Compute-what-you-claim.** Verification recomputes every fact from chain + wallet data; the registry is evidence *about claims*, never the source of truth. | Trusting a server. |
| P6 | **Amounts are zatoshi integers** everywhere on the wire; ZEC strings only at UI boundaries. | Float drift / footguns. |

## 2. Actors

- **Attestor agent (A)** — holds a Zallet with accounts #0 (commerce) and `zsol-attn` (#9 default).
- **Verifier agent (B)** — counterparty; holds its own Zallet on some node; receives scoped keys.
- **Registry (C)** — public service storing **claim-tier** objects only (append-only, digest-chained).
- **Operator (H)** — human supervising A (and optionally B).

## 3. Account model (critical to the privacy promise)

```
Zallet wallet of agent A
├── account #0            ← commerce (never attested, never view-key-shared)
└── account "zsol-attn"   ← attestation account (default #9)
      └── anchors: self-spends of ≥ B zatoshi to itself
```

Sharing the uivk of `zsol-attn` reveals: its anchor self-spends, its balances over time,
its addresses' internal history. It reveals **nothing** about account #0. Per-verifier
finer scoping is possible (account `zsol-attn-v1|-v2|...`) at the cost of more scanning —
the MCP tooling supports it (SRS FR-3.4).

## 4. Attestation object (wire)

JSON, compact, base64url-transportable (`b64u`). Canonical form = key-sorted, no whitespace.

```jsonc
{
  "protocol": "zsol/0.1",
  "agentId": "agent-0001",                // public label; NOT an address
  "claimZat": 100000000,                  // "≥ B" in zatoshi (integer)
  "anchorHeight": 4510000,                // chain height of anchor
  "anchorTxid": "<64-hex>",
  "accountUa": "u1<unified address of zsol-attn primary diversified UA>",
  "memoBinding": "b64u(<raw memo bytes>)", // anchor memo, see §5
  "issuedAt": 1769337600,
  "maxAgeHours": 24                       // continuity horizon (T2)
}
```

**Object digest** `D = BLAKE2b-256("zsol/0.1" ‖ 0x1f ‖ canonical(object \ {memoBinding}))` —
canonical form of the object with the `memoBinding` field **excluded**. The exclusion makes the
anchor-memo fixed point computable (the memo carries D, so D cannot depend on the memo). Binding
completeness is preserved: every field except `memoBinding` is bound through D, and `memoBinding`
itself is verified by direct byte-equality against the on-chain scanned memo (§5/§6).

**t0 claim tier:** object published to registry alone. Registry freshness/tamper-evidence only;
**no solvency established yet** (this is what public claim means — honest by design).

## 5. Anchor transaction (the cryptographic core)

The anchor is produced via Zallet `z_sendmany` **by the agent's own wallet**:

- Recipients: 1 output, **the same account's own diversified UA** (self-spend), value ≥ B zatoshis.
- Memo of that output: raw anchor-memo bytes, layout `zs01(4) ‖ ver(1) ‖ height_LE64(8) ‖ digest(32)`
*right-padded* to 512 bytes by the wallet (or as far as the wallet requires; the exact zero-pad
border is the wallet's ZIP-302/ memo policy — verifier parses the first 45 bytes only).
- Amounts: encrypted on chain (z→z). Fee: standard shielded fee, paid *from* the attestation account.
- **Balance appears unchanged** (in == out), so an anchor never "spends" the attested funds.

Wait — self-spend input/output sums: the transfer spends notes worth ≥ B and returns ≥ B to the same
account; thermodynamically the attested balance remains. Verifier checks current balance ≥ B *at
scan time* (§6), not just at anchor.

**Binding:** `memoBinding` inside the object *is* the anchor memo — an attacker cannot produce
`{object, memoBinding}` for another agent's anchor because re-computing D over a *forged* object
never matches the real memo bytes, and reading/writing anchor memos requires spend/scan authority
of *that* account (P3). Replay/second-use: anchors are naturally one-shot per claim level — an
anchor with outputs ≥ B satisfies any claim ≤ B; two different claims of different sizes need
different anchors; claim-specific digests prevent an attacker from editing the claim while
reusing an anchor memo.

## 6. Verification (tiers) — performed by the verifier's own stack

Verifier (human or agent) holds per-agent material sent over a **direct channel** (MCP tool
`zsol_share_scope` transfers object + scoped uivk). Verifier imports uivk into its own Zallet
(`z_importkey`), scans to tip, and evaluates:

| Check (all must pass for T1 "verified") | Source of truth |
|---|---|
| anchor txid is mined, confirms ≥ 1 | Zebra/registry chain RPC |
| tx appears **within the imported uivk's account** as a self-spend (all outputs are the account's own diversifiers) | Zallet scan result / `z_viewtransaction` |
| sum of shielded outputs of the anchor tx ≥ `claimZat` | wallet view of tx |
| memo bytes of the anchor **re-parse & match**: `tag=zs01, ver=01, height=anchorHeight, digest=D(object)` | wallet view (decrypt with uivk) |
| account balance ≥ claimZat **at scan time** | Zallet balance scan |
| `issuedAt` fresh w.r.t. `maxAgeHours` (T2 continuity) | wall clock |

Failure ⇒ tier stays "claim" with a machine-readable reason (SRS FR-1.x lists error codes).

**Revocation honesty:** viewing keys cannot be un-shared. zsol treats scope hygiene as *process*
(small dedicated account, per-verifier sub-accounts) rather than pretending crypto revocation.
The registry never sees keys, so mass surveillance has no single choke point (P2/P4).

## 7. Registry wire (public claims only)

- `POST /v0/attestations` body: `{object: b64u}` → `201 {entryId, chainDigest}`; dedupe unique on
 `(agentId, anchorTxid)`.
- `GET /v0/attestations/:agentId` → latest entries ascending by issuedAt with running `chainDigest`
 (`chainDigestₙ = BLAKE2b-256(chainDigestₙ₋₁ ‖ canonical(objectₙ))`, genesis = zero digest).
- `GET /v0/health` → `{height?, ok}`.
- Registry MUST reject: non-canonical objects, `claimZat < dust (10000)`, unknown `protocol`,
 duplicate `(agentId, anchorTxid)`.

## 8. MCP gateway tools (zsol-mcp; stdio + streamable HTTP)

| Tool | Side | Args → Result |
|---|------|----------------|
| `zsol_health` | both | stack status: zebra ready / zallet verbs pinned / registry reachable / height |
| `zsol_prepare_anchor` | attestor | `claimZat` → checks account balance ≥ claim + floor, returns planned anchor shape (dry run) |
| `zsol_anchor` | attestor | `claimZat` → performs self-spend via Zallet, awaits op result, returns txid + height (waits confirmations) |
| `zsol_make_attestation` | attestor | `claimZat` → derives object incl. memo binding from last anchor; returns object b64u |
| `zsol_publish` | attestor | object → `POST` registry, returns entryId |
| `zsol_share_scope` | attestor | peer label → exports scoped uivk of the dedicated account (never account #0; SRS invariant 5) |
| `zsol_import_scope` | verifier | uivk + label → imports into verifier Zallet, returns scan summary (counts only) |
| `zsol_verify` | verifier | object b64u → runs §6 checks; returns tier + per-check results |
| `zsol_registry_fetch` | verifier | agentId → registry entries (claim tier) |

## 9. Reason codes (machine-readable)

`ok · claim-only · anchor-not-mined · not-self-spend · claim-exceeds-outputs · memo-digest-mismatch ·
balance-below-claim · stale-attestation · uivk-unavailable · protocol-unknown · registry-unreachable ·
wallet-unreachable · dust-claim`.

## 10. Threat model & what we intentionally do NOT claim

| Adversary | zsol defense |
|---|---|
| Chain observer | anchors are indistinguishable z→z; agents' label is off-chain (`agentId` only in registry/objects). |
| Curious registry | sees public claims only; by design cannot compute balances. |
| Malicious attestor | every claim is re-verified from chain + wallet scan (P5); memo digest prevents object editing. |
| Compromised verifier | leaking a scoped uivk exposes attestation history **of the dedicated account only** (documented, bounded). |
| Over-sharing operator | MCP refuses any tool that exports material outside+zsol-attn scope (FR-3.4). |

**Not claimed:** atomic escrow, inheritance semantics, multi-asset attestation, mainnet promotion.
**Fungibility caveat (documented):** claims are per-account and per-pool; cross-pool sum
inference (Sapling vs the new Ironwood-era pool) is out of scope for v0.1 and listed as
[future work in `TODOS.md`] with honest disclosure in the SRS.

## 11. References

ZIP-32 (account derivation), ZIP-316 (UA/uivk), ZIP-302/623 (memos) · Zallet RPC (zcashd-shaped;
**boot-step-0 parity pin required** — [`OPERATIONS.md`](OPERATIONS.md)) · Z3 (Zebra/Zaino/Zallet) ·
BaFin notices (Aug 19 2026; proof-of-ownership > €1,000), FinCEN withdrawal (Oct 5 2026, US).
