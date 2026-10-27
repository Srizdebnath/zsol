# zsol Architecture (v2, protocol 0.2)

## 1. Component map

Everything runs on the operator's laptop as **separate processes with separate data
directories**. The borrower, the underwriter and the second borrower never share key material
or wallet databases. The demo topology is honest about being one machine.

```
                     Zcash TESTNET (post-NU7, branch 0x77190ad9)
                ┌───────────────────────────┬─────────────────────────────┐
                │ primary: testnet.zec.rocks:443 (lightwalletd gRPC/TLS)  │
                │ fallback: local Zebra 7.0.0 + Zaino (z3, synced)        │
                └───────────────▲───────────────────────▲─────────────────┘
                                │ compact blocks,       │
                                │ GetTransaction,       │
                                │ SendTransaction       │
┌───────────── BORROWER A ──────┴───────┐   ┌───────────┴──── UNDERWRITER U ───────────────┐
│ data/a/  seed.age  wallet.sqlite       │   │ data/u/  seed.age  wallet.sqlite              │
│  accounts: commerce, zsol-attn         │   │  accounts: treasury (receives repayments)     │
│ zsol-mcp (stdio) ◄── Claude agent A    │   │  view-only: imported UFVKs of borrowers       │
│  anchor · make · publish · share_ufvk  │   │ zsol-credit (HTTP :8090) + zsol-mcp (HTTP)    │
│  sign_challenge · credit_request/repay │   │  verifier engine C1–C8 · watch loop           │
└──────────┬──────────────────────┬──────┘   │  loans.sqlite · policy.json                   │
           │ object + UFVK +      │          │  Bitrefill client ──► api.bitrefill.com (v2)  │
           │ signed challenge     └─────────►│                       or /mcp (buy-products)  │
           │ (direct channel, HTTPS)         └───────────┬───────────────────────────────────┘
           ▼                                             │ SSE events
┌──────────────────────────┐                ┌────────────┴─────────────┐
│ zsol-registry (:8088)    │◄──── fetch ────│ web (Next 16, :3000)     │
│ claims only, digest chain│                │ / /explorer /verify /desk│
└──────────────────────────┘                └────────────┬─────────────┘
                                                         │ cloudflared tunnel
                                                  public URLs for judges
Borrower B2: same binary as A with an unfunded zsol-attn, used to show the claim-only path.
```

## 2. Crates

| Crate | Status | Role |
|---|---|---|
| `zsol-types` | update | Object 0.2, frame digest, memo codec, presentation (ed25519-dalek), reason codes |
| `zsol-chain` | **new** | lightwalletd client (tonic + `zcash_client_backend` proto); branch/network boot check |
| `zsol-lightwallet` | **new** | `zcash_client_sqlite` wallet: seed (age), accounts, sync, anchor and payment builder, UFVK export guard, view-only import |
| `zsol-verifier` | **new** | C1–C8 engine on a view-only account; watch loop emitting events |
| `zsol-credit` | **new** | Underwriter service: policy, loans, Bitrefill client, repayment matcher, risk link |
| `zsol-wallet` | keep | Zallet JSON-RPC backend behind the same `AttestorBackend` trait (enabled once PR #898 lands) |
| `zsol-registry` | update | Protocol 0.2, key-field rejection |
| `zsol-mcp` | update | Tool set per SRS FR-5.1 and FR-6.1 |

**Pinned dependencies** are the same versions as `zcash/zcash-devtool` (which builds on
librustzcash `main`): `zcash_client_backend =0.25.0-pre.2` (features `orchard`, `pczt`,
`lightwalletd-tonic-tls-webpki-roots`, `unstable`), `zcash_client_sqlite =0.23.0-pre.2`,
`zcash_keys =0.17.0-pre.2`, `orchard 0.16`. Ironwood decryption goes through
`decrypt_transaction`, which handles `ValuePool::Ironwood` using the UFVK's Orchard component.

## 3. Life of a Credit Desk loan

1. **Fund.** A faucet sends TAZ to A's `zsol-attn` UA, and A's operator separately funds
   `commerce` for repayments.
2. **Anchor.** `zsol_anchor(claimZat)` builds the frame (with a fresh session key), computes D,
   and sends the self-spend with memo `0xFF‖zs02‖D`. It waits for the tx to be mined, typically
   one 25-second block.
3. **Publish.** `zsol_make_attestation` then `zsol_publish` post the claim-tier object to the
   registry.
4. **Request.** Agent A calls `credit_request{product}`. The MCP tool fetches U's challenge,
   signs it, and POSTs `{object, ufvk, sig, product}` to U.
5. **Verify.** U imports the UFVK view-only at `birthday`, syncs (typically under 2,000 compact
   blocks) and runs C1–C8. The result is `verified`.
6. **Purchase.** U checks the limit against policy, then calls Bitrefill `POST /v2/invoices` with
   a test product and `payment_method=balance`. It receives the invoice ID and redemption payload.
7. **Deliver.** U returns `{loanId, redemption, principalZat, dueHeight, repayTo, memo}`. The
   loan is now `open`, and U's watch loop subscribes to A's anchored nullifiers.
8. **Repay.** `credit_repay(loanId)` sends a shielded payment from A's `commerce` account. U's
   treasury scan matches the memo and the loan becomes `repaid`.
9. **Risk path (shown live).** A spends its anchored notes. U's watch emits `released`, open
   loans become `at-risk`, and the `/desk` board turns red within about one block.

## 4. Data stores

| Store | Contents | Owner | Notes |
|---|---|---|---|
| `data/a/seed.age` | A's BIP-39 seed | A | Encrypted with an age identity kept outside the repo |
| `data/a/wallet.sqlite` | A's notes and txs | A | `zcash_client_sqlite` schema |
| `data/u/wallet.sqlite` | U's treasury plus imported view-only accounts | U | UFVKs live **only** here |
| `data/u/loans.sqlite` | Loans, Bitrefill invoice IDs, events | U | Redemption codes encrypted at rest |
| `data/registry.sqlite` | Claim objects and digest chain | Registry | Never stores keys |

## 5. Failure handling

| Failure | Detection | Behaviour |
|---|---|---|
| Endpoint on the wrong branch or network | Boot check | Refuse to start (`chain-unreachable`) |
| Endpoint unreachable mid-run | gRPC timeout (10 s) | Bounded retry, then switch to the fallback endpoint if configured |
| Verifier not synced to tip | Lag > 1 block | `scan-incomplete`; never reports `verified` |
| Bitrefill error or `*-fail` product | HTTP status / invoice status | Loan never opened; `purchase-failed` |
| Repayment memo malformed | Memo parse | Ignored and logged at debug; the loan stays open |
| Reorg past the anchor | Watch sees the anchor tx missing | `anchor-not-mined`; loans go `at-risk` |

## 6. Decisions (ADR digest)

- **D1. UFVK over UIVK.** Liveness needs nullifiers, and UFVK is the minimum key that provides
  them (FINDINGS F-1).
- **D2. Light-client first.** No Zallet release supports NU7 on testnet, and a local resync takes
  more than 24 h (F-7, F-8). Zallet stays as a backend behind a trait.
- **D3. Spend authority from consensus (C3), not from memos.** F-3.
- **D4. Ed25519 session key committed in D.** This is standard cryptography used only for
  presentation freshness; no new primitive is introduced.
- **D5. Watch instead of escrow.** It needs no consensus change, and the attestor can always exit
  visibly.
- **D6. Bitrefill test products in testnet mode.** The economy (TAZ) is worthless, so real-value
  purchases are opt-in and capped (`mode=live`).
- **D7. Laptop plus tunnel hosting.** Chosen by the owner. The mitigations are a local node
  fallback and a hotspot.

## 7. Roadmap (after the hackathon)

1. **Zero-disclosure proof.** A signed but unbroadcast PCZT spending the anchored notes, verified
   with proof and signature checks plus a nullifier non-membership check. No viewing key is
   shared at all.
2. Upstream Zallet support for importing an Orchard/Ironwood UFVK, so the verifier can run on
   Zallet (contributed to zcash/wallet).
3. Mainnet mode with real Bitrefill purchases, plus underwriter reputation published to the
   registry.
