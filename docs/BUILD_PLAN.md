# zsol build plan: Free Society Hackathon (Nov 4, 2026, Mumbai)

Hard dates: the build day is **Wed Nov 4, 11:00 IST**; **submissions close at 00:00 IST on
Nov 5**; top teams are announced Nov 5 at 11:00 and demos run at 12:00. Remote participation is
not allowed.

## 1. Daily plan (each day ends at a gate, with evidence committed)

| Date | Work | Gate |
|---|---|---|
| **Tue Oct 27** | Owner: create a Bitrefill account and API key; join the Zcash forum/Discord. Build: `zsol-chain` boot check against `testnet.zec.rocks:443`; `zsol-lightwallet` seed, accounts and sync compiling on the pinned pre-release crates; generate A's UA. Faucet ladder run; find a post-NU7 explorer. | **G0** |
| Wed Oct 28 | `zsol-types` 0.2 (frame, memo, Ed25519, vectors); anchor builder; first real anchor | **G1** |
| Thu Oct 29 | `zsol-verifier`: view-only import and C1–C8; separate verifier DB. Check that local Zebra 7 has synced and add Zaino. | **G2** |
| Fri Oct 30 | Live negative cases; watch loop; spend-to-release | **G3, G4** |
| Sat Oct 31 | `zsol-credit`: policy, loans, Bitrefill client (test products), repayment matcher, risk link | **G5** |
| Sun Nov 1 | MCP tools (FR-5.1, FR-6.1) wired for a Claude agent; web `/desk`, `/verify`; `zsol up`; tunnel | **G6** |
| Mon Nov 2 | Stretch: Zallet from PR #898 on local Zebra 7 (G7). Hardening, CI, README. Top up TAZ (≥ 1 TAZ). | G7 |
| Tue Nov 3 | Two full dress rehearsals on testnet; record the backup video of a real run; pitch (3 slides + live demo); submission text | rehearsal |
| **Wed Nov 4** | Event: on-day feature (§4), live run at the venue, submit **before 23:00 IST** (1 h buffer) | submit |
| Thu Nov 5 | Demo | — |

## 2. Already running (started 2026-10-26)
- Local testnet Zebra **7.0.0** is resyncing from genesis (z3 project `z3-testnet`; DNS override
  in `z3/docker-compose.testnet.override.yml`). At the measured rate (≈2,000 blocks/min early on)
  it needs roughly a day; check on Oct 28.

## 3. Owner-only actions
1. Create a Bitrefill account and API key at <https://www.bitrefill.com/account/developers>.
   Never commit the key; it goes in `.env`.
2. Confirm with the organizers (WebZero) or the Bitrefill mentors whether test-product purchases
   satisfy "use Bitrefill" (SRS §7 Q1).
3. Install `cloudflared` on the laptop; prepare a phone hotspot.

## 4. Disclosure and the on-day scope
Submission text states: *"Protocol, wallet, verifier and Credit Desk core were built before the
event (git history from 2026-10-25). Built on the day: \<feature\>."*
Candidate on-day features, chosen on Nov 3 according to what is finished:
- A Claude agent that shops conversationally ("buy me a ₹500 phone top-up") with the Credit Desk
  as its payment method, using Bitrefill MCP `search-products` and our `credit_request`.
- A per-verifier account rotation tool (`zsol_rotate_scope`), making UFVK burn-and-replace one
  call.
- An embeddable "verified solvent" badge widget for merchant pages, driven by watch events.

## 5. Demo script (≈3 minutes, all live)
1. `/` shows the stack is live: branch `0x77190ad9`, tip height.
2. Agent A: "I need a gift card, I only have ZEC." It anchors (≈25 s) and requests credit.
3. `/desk`: C1–C8 rows turn green → Bitrefill invoice ID → redemption delivered.
4. An agent whose proof is no longer backed (released, or over-claimed) requests credit →
   `claim-only` with reason codes and no invoice (live: `evidence/G5b-claim-only.md`).
5. A spends its anchored notes → the watch emits `released` → the loan goes `at-risk` (red)
   within one block.
6. A repays from `commerce` → `repaid`. Every step links to its txid.
7. A judge scans the QR code for `/verify` and checks the attestation on their own phone.
