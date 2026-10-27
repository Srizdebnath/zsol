# zsol Protocol Specification

**Version:** 0.2.0 · **Asset:** ZEC (Ironwood and Sapling shielded pools), unit **zatoshi** = 10⁻⁸ ZEC
**Network of record:** Zcash **testnet, post-NU7** (activation height 4,465,026, consensus branch
`0x77190ad9`, 25-second target spacing) · **Status:** implementation-reference
**Supersedes:** 0.1.0. See [`FINDINGS.md`](FINDINGS.md) for why 0.1 was unsound.

---

## 0. Purpose

An attestor agent **A** proves to a verifier agent **V** that

> *"the holder of session key `K` controls unspent shielded notes worth ≥ B zatoshi, and will
> keep controlling them until you see otherwise"*

with these guarantees:

- **Chain observers learn nothing.** The anchor is an ordinary fully-shielded transaction.
- **V learns only the dedicated attestation account.** V never sees A's commerce account.
- **V can detect the moment the attested funds move**, so the proof is live rather than a
  snapshot.

## 1. Principles

| # | Principle | Prevents |
|---|-----------|----------|
| P1 | **Shielded-only.** Anchors are fully shielded. No transparent address appears in any protocol path. | Public linkage. |
| P2 | **Non-custodial.** Spending keys stay in A's own wallet. The registry and underwriters never hold keys. | Server compromise turning into theft. |
| P3 | **Standard primitives only.** Zcash consensus (spend authorization, nullifiers), BLAKE2b-256, ZIP-302 memos, ZIP-316 UFVKs, Ed25519 (RFC 8032). Nothing bespoke. | Unimplementable or unaudited cryptography. |
| P4 | **Dedicated attestation account.** Anchors live in a ZIP-32 account used only for attestation. | Commerce history leaking through a shared key. |
| P5 | **Recompute everything.** The verifier derives every fact from chain data plus its own view-only scan. The registry is evidence *about* claims, never the truth. | Trusting a server. |
| P6 | **Integers on the wire.** Amounts are zatoshi `u64`. Conversion to ZEC or USD happens only at the UI edge. | Float drift. |

## 2. Actors

- **Attestor (A).** An agent with its own wallet. Account `commerce` is never shared. Account
  `zsol-attn` is the dedicated attestation account.
- **Verifier (V).** Any counterparty with chain access. In the Credit Desk (§11) the verifier is
  the **underwriter**.
- **Registry (R).** An optional public, append-only, digest-chained store of claim-tier objects.
- **Chain access.** A lightwalletd-protocol endpoint (public `testnet.zec.rocks:443`, or local
  Zaino) or a local Zebra node. Every party chooses its own.

## 3. Account model

```
A's wallet (seed held by A only)
├── account "commerce"   ← payments, repayments; never shared, never attested
└── account "zsol-attn"  ← receives funding; makes anchor self-spends; nothing else
       UFVK(zsol-attn) is what V receives
```

**What a UFVK reveals (normative disclosure).** Every note received by `zsol-attn` since its
birthday, including value, memo and spent-state. It also reveals every output *sent* by
`zsol-attn` (via its outgoing viewing key), including recipient and value. It reveals nothing
about any other account derived from the same seed. Operational rule: `zsol-attn` only receives
funding and only makes anchors.

Version 0.1 shared an incoming viewing key (UIVK). That key cannot see spends, so a balance check
based on it is meaningless. A full viewing key (UFVK) is the *minimum* key that can prove
liveness. See FINDINGS F-1.

## 4. Attestation object (wire)

Canonical form is JSON with keys sorted and no whitespace. It is transported as base64url without
padding (`b64u`).

```jsonc
{
  "protocol":    "zsol/0.2",
  "network":     "test",                 // "test" | "main"
  "agentId":     "agent-0001",           // public label, not an address
  "claimZat":    10000000,               // B, the claimed amount (u64)
  "pool":        "ironwood",             // "ironwood" | "sapling"
  "sessionKey":  "<b64u 32-byte Ed25519 public key>",
  "birthday":    4540000,                // zsol-attn birthday height (scan start for V)
  "issuedAt":    1793000000,             // unix seconds
  "maxAgeHours": 24,
  "anchorTxid":  "<64 hex, display byte order>",
  "anchorHeight":4544800,
  "memoBinding": "<b64u of the 37 significant memo bytes>"
}
```

**Frame** = the object without `anchorTxid`, `anchorHeight` and `memoBinding`. These are exactly
the fields knowable before the anchor is broadcast.

**Frame digest**: `D = BLAKE2b-256(personal = "zsol/0.2-frame\0\0", canonical(frame))`. The
personalization string is 16 bytes, NUL-padded.

## 5. Anchor transaction

A builds and broadcasts a transaction from `zsol-attn` to `zsol-attn`:

- **Inputs:** notes of `zsol-attn` in the declared `pool`.
- **Binding output:** one output to a `zsol-attn` address in `pool` carrying the memo
  `0xFF ‖ "zs02" ‖ D`, right-padded with zeros to 512 bytes. The leading `0xFF` marks
  "arbitrary data, not text" under ZIP-302.
- **Anchored notes** are all outputs of this transaction received by `zsol-attn`. Their sum must
  be ≥ `claimZat`.
- **Fee** follows ZIP-317 and is paid from `zsol-attn`. The anchored value is the tracked
  quantity, so the fee does not affect the claim.

The anchor costs one fee and roughly one block (≈25 s on post-NU7 testnet). A can attest any
amount ≤ the anchored sum to any number of verifiers from a single anchor (§7). A needs a new
anchor only to change the frame, for example a new `sessionKey` or a larger claim.

## 6. Verification (performed entirely by V's own stack)

V receives `{object, ufvk}` over a direct channel. The registry never carries a UFVK. V then
imports the UFVK as a **view-only account** with `birthday` as the scan start, syncs to tip, and
evaluates:

| # | Check | Fails with |
|---|-------|-----------|
| C1 | `protocol`, `network` and `pool` recognised; `claimZat ≥ 10_000` | `protocol-unknown` / `network-mismatch` / `dust-claim` |
| C2 | `anchorTxid` is mined at `anchorHeight` with depth ≥ `minConf` (default 1) | `anchor-not-mined` |
| C3 | **Spend authority.** At least one nullifier revealed by the anchor tx equals the nullifier of a note V's scan attributes to `zsol-attn`. Consensus requires a valid spend-authorization signature for every spend, so only the holder of `zsol-attn`'s spending key could have produced this transaction. | `not-spent-by-account` |
| C4 | **Binding.** Exactly one anchor output decrypts under the UFVK with memo bytes `0xFF‖"zs02"‖D'`, where `D'` is recomputed from the presented object, `D' = D`, and the bytes equal `memoBinding` | `memo-digest-mismatch` |
| C5 | Sum of anchored notes (in `pool`) ≥ `claimZat` | `claim-exceeds-outputs` |
| C6 | **Liveness.** No anchored note's nullifier appears on chain up to V's synced tip | `anchored-note-spent` |
| C7 | `now − issuedAt ≤ maxAgeHours·3600`, and `issuedAt ≤ blocktime(anchorHeight) + 7200` | `stale-attestation` |
| C8 | **Presentation (§7).** A valid Ed25519 signature by `sessionKey` over V's challenge | `presentation-invalid` |

Tiers:

- **`verified`**: C1–C8 all pass.
- **`claim-only`**: anything less, reported with every failing check's code.
- **`live`**: `verified` while V keeps watching (§8). It drops to `released` when C6 starts to
  fail.

**Why C3 matters.** Anyone who knows a `zsol-attn` address can *send* it a note with any memo.
Version 0.1 wrongly assumed that writing a memo required spend authority (FINDINGS F-3). Without
C3, a past verifier holding the UFVK could donate B to the account and bind *its own*
`sessionKey`. C3 closes that because the anchor must consume `zsol-attn`'s own notes.

## 7. Presentation (challenge-response)

This step stops relay attacks, where V₁ re-presents A's object and UFVK to V₂ (FINDINGS F-4).

1. V → A: `challenge = {nonce: 32 random bytes, verifierId, expiresAt}`.
2. A → V: `sig = Ed25519.sign(sessionSecret, msg)`, where
   `msg = "zsol/0.2-present" ‖ D ‖ anchorTxid(32, internal order) ‖ nonce ‖ u64le(expiresAt) ‖ utf8(verifierId)`.
3. V checks the signature against `sessionKey` from the object. Because `sessionKey` is inside
   `D`, and C3 proves `D` was committed by the spending-key holder, a valid signature proves the
   presenter is the party that committed the anchor.

The session secret lives with A's agent process. Rotating it requires a new anchor.

## 8. Live solvency watch

While V maintains interest (an open quote, an outstanding credit line), it keeps its view-only
account synced and re-evaluates C6 on every new block. Events:

- `attestation.live` `{entryId, height}` is emitted on each block where C1–C8 still hold.
- `attestation.released` `{entryId, height, reason: "anchored-note-spent"}` is emitted the first
  block an anchored nullifier is seen.
- `attestation.expired` `{entryId}` is emitted when C7 fails.

Withdrawal is A's revocation primitive. A can end an attestation at any time by spending the
anchored notes, and every watching verifier sees it within one block. **This is not escrow**:
zsol makes solvency *observable*, not *locked*.

## 9. Registry wire (public claims only)

- `POST /v0/attestations {object: b64u}` returns `201 {entryId, chainDigest}`. Entries are
  unique on `(agentId, anchorTxid)`.
- `GET /v0/attestations/:agentId` returns entries ascending, each with a running `chainDigest`.
  The formula is `chainDigestₙ = BLAKE2b-256(chainDigestₙ₋₁ ‖ canonical(objectₙ))`, where the
  genesis digest is 32 zero bytes. `entryId = hex(chainDigest)`.
- `GET /v0/health` returns `{ok, tipHeight, watermarkHeight, protocol}`.
- Rejections: non-canonical body; unknown `protocol` or `network`; `claimZat < 10_000`;
  duplicate; any field named like a key (`ufvk`, `uivk`, `seed`, `sk`). Transport slugs:
  `413`; `429 rate-limited`; `503 chain-descended`.

## 10. Reason codes

`ok · claim-only · protocol-unknown · network-mismatch · dust-claim · anchor-not-mined ·
not-spent-by-account · memo-digest-mismatch · claim-exceeds-outputs · anchored-note-spent ·
stale-attestation · presentation-invalid · ufvk-invalid · scan-incomplete · chain-unreachable ·
registry-unreachable`

## 11. Credit Desk extension (`zsol-credit/0.1`)

The Credit Desk is a concrete market for §6. **U** is an underwriter agent holding a Bitrefill
account balance and its own Zcash wallet. **A** is a borrower agent that holds only ZEC (Bitrefill
does not accept ZEC as payment).

1. `credit.open` (A → U): `{object, ufvk, product: {id, value, currency}}`.
2. U issues a challenge (§7). A signs it. U runs C1–C8. Anything other than `verified` is
   refused with the reason codes.
3. **Limit.** `limitZat = floor(Σ anchored × ltvBps / 10_000) − outstandingZat(A)`, where
   `ltvBps` comes from the policy file. Pricing in USD uses a recorded reference rate, and the
   source and timestamp are stored with the loan.
4. U buys the product through Bitrefill (MCP `buy-products` or REST v2 `POST /invoices`). In
   **testnet mode** only Bitrefill's official test products are allowed (`payment_method=balance`,
   no money moves). **Live mode** requires an operator flag plus a per-loan USD cap.
5. U → A: `{loanId, redemption, principalZat, dueHeight, repayTo: U's UA, memo: 0xFF‖"zsc1"‖loanId}`.
6. A repays from `commerce` (never `zsol-attn`) with a shielded transfer carrying that memo. U's
   wallet scan matches the memo, and the loan becomes `repaid`.
7. While the loan is open U watches A's attestation (§8). On `attestation.released` the loan
   becomes `at-risk` and A's line is frozen. Past `dueHeight` without repayment, the loan becomes
   `defaulted`.

Loan states: `open → repaid` · `open → at-risk → repaid | defaulted` · `open → defaulted`.

## 12. Threat model

| Adversary | Outcome |
|---|---|
| Chain observer | Sees one shielded transaction. No label, no amount, no linkage. |
| Curious registry | Sees claims (agentId, claimZat, txid). Cannot compute any balance. Never receives keys. |
| Attestor who spends after attesting | Caught by C6 within one block of V's sync (§8). |
| Third party holding the UFVK (e.g. an earlier verifier) | Cannot pass C3 (needs the spending key) or C8 (needs the session secret). |
| Relaying verifier | Cannot pass C8 for a fresh nonce. |
| Malicious verifier | Learns the `zsol-attn` history, which is bounded and disclosed (§3). Cannot spend. Cannot be revoked; the mitigation is a fresh account per relationship. |
| Malicious underwriter | Can refuse credit or mis-report its own Bitrefill purchase. Can never move A's funds. |

**Not claimed:** escrow, atomic delivery-versus-payment, cross-pool aggregation, mainnet
readiness, or that `agentId` identifies a legal person.

## 13. References

ZIP-32 (accounts), ZIP-302 (memos), ZIP-316 (UFVK), ZIP-317 (fees), ZIP-218 (NU7 block spacing),
RFC 8032 (Ed25519), RFC 7693 (BLAKE2), lightwalletd `CompactTxStreamer` protocol, Bitrefill API
v2 and eCommerce MCP (`https://api.bitrefill.com/mcp`). Context: the Orchard counterfeiting bug
disclosed in June 2026 was found with AI-assisted auditing, which motivated the Ironwood pool
(NU6.3).
