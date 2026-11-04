# zsol honesty annex (T-45, SPEC §10) — leak-surface truth

> **Superseded (2026-10-26):** this document describes protocol 0.1. See [`SRS.md`](SRS.md) v2.0, [`SPEC.md`](SPEC.md) 0.2, [`BUILD_PLAN.md`](BUILD_PLAN.md) and [`FINDINGS.md`](FINDINGS.md). It will be rewritten as each part is implemented.

This annex states, without varnish, what each shared artifact EXPOSES and what
the registry CANNOT know. Written for reviewers and for the demo audience.
Nothing in zsol was designed under the assumption that these facts are hidden.

## 1. What sharing a uivk exposes

A viewing key (uivk) for one account, once imported and scanned by the holder:

- **Full transaction history of that account** from the scan horizon onward —
  every incoming/outgoing note in that account's note tree, with amounts.
- **All memos** attached to notes it can decrypt (zsol memos are meant to be
  read by exactly this audience — the anchor memo is the binding).
- **Future activity** — a uivk sees NEW transactions arriving after the share.
  Scoping is per-ACCOUNT (per-verifier sub-accounts, zsol-attn-91/92), so the
  blast radius is bounded to that account's activity only.
- **It cannot be revoked.** There is no "unshare" in the protocol. The honest
  operating rule (and what `zsol_share_scope` returns verbatim): when a peer
  relationship degrades, mint them a FRESH sub-account scope and treat the
  old one as burned — old material stays visible to that peer forever.

## 2. What a uivk does NOT expose

- **Spend authority.** A viewing key cannot move funds; the spend authorizing
  key stays in the attestor wallet (SRS invariant 5: master spend keys never
  leave; account #0 is never shared; zsol exports only for `zsol-attn*`
  accounts, refused otherwise at the tool layer — T-24/T-36 negative tests).
- **Other accounts' activity.** Seed-level separation: a scope for account 91
  reveals nothing about account 9 (the master attest account) or 92.
- **Counterparty identity on the graph.** Shielded pools hide graph links; the
  holder sees the account's own note flow, not a global tx graph.

## 3. What the registry cannot know (it deliberately leans out)

- **Balances.** The registry stores CLAIMS about frames; it never scans, never
  holds keys, and cannot check solvency (P5: evidence ABOUT claims, never a
  source of truth for solvency).
- **Whether an anchor is real.** Only verifiers with a stack can check the
  §6 rows (mined-ness, note visibility, balance at scan). The registry's
  chain integrity (§7 digests) is a property of the LEDGER ONLY.
- **Which account belongs to which human.** An agentId is a self-declared
  string. Identity binding rests on the agent holding the matching spend
  authority to write anchor memos into its claimed account — enforced by the
  verifier's scan correspondences (§6), not by the registry.
- **Memo contents beyond the binding.** Objects reference `memoBinding`
  (b64u bytes). The registry reads the byte frame, never wallet context.

## 4. Demo claims in the open (T-43/T-44 honesty)

- Regen chips: testnet (TAZ) — always labeled TAZ, never marketed as ZEC.
- The regtest CI loop (T-39) is documented AS REGTEST. It is evidence that
  the wiring executes, not a solvency fact.
- `claim-only` results are shown with their failing check rows, not hidden
  behind a generic "unverified."
