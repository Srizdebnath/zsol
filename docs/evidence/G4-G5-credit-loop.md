# G4 + G5 — Credit Desk loop and solvency watch, live on testnet (2026-10-27)

One continuous run, every step driven through the borrower's MCP gateway (`tools/call` over
streamable HTTP, `zsol-mcp` on borrower-a's wallet, `ZSOL_ALLOW_SPEND=1`,
`ZSOL_SPEND_CAP_ZAT=100000000`) against the Credit Desk (`zsol-credit`, underwriter wallet
`~/.zsol/underwriter`, policy `config/credit-policy.json`, `mode: test`). Endpoint
`testnet.zec.rocks:443`, consensus branch `77190ad9` (NU7). Full machine output:
`G4-G5-credit-loop.json` (addresses and the gift-card code removed).

| # | Step (MCP tool) | Result | Height | UTC |
|---|---|---|---|---|
| 1 | `zsol_anchor` agent-a, claim 50,000,000 zat | anchor `20ec2cc846c78dc0c9a54b8486b4212101270d1d4211323852f275032d0300ef`, 157 s to mined | 4,546,439 | 05:24:44 → 05:27:35 |
| 2 | `credit_request` test-gift-card-code, $10 | desk verified C1–C8 in its own wallet → **verified**; Bitrefill invoice `b46f2eb7-c7a8-4f07-a2d9-35de7084b0b2`, order `6ae03697764a4d8e872c0d6c`, code delivered; loan `71b26fe08e382fa6544035ddf182b0ed` **open**, principal 820,931 zat at 1,218.13 USD/ZEC (`kraken:XZECZUSD/last`), due 4,549,898 | 4,546,442 | 05:28:56 |
| 3 | `zsol_release` (moves 89,970,000 zat zsol-attn → commerce) | spend `8657a142cf9feabd6efa4ab229ccfd8693cb0260f8bf7dca132a58d84d3499ee` | mined 4,546,446 | broadcast 05:29:35 |
| 4 | desk watch tick | event `attestation-released`, loan → **at-risk**, credit frozen for agent-a | 4,546,446 | 05:31:15 |
| 5 | `credit_repay` (820,931 zat from commerce, memo `0xFF‖"zsc1"‖loanId`) | repayment `a66d965f136b11b5c54014edc32154a2fb87727274fa515f23a0ae9eeb6cbc37` | — | broadcast 05:31:55 |
| 6 | desk repayment matcher (underwriter's own wallet scan, memo decode) | event `loan-repaid`, loan → **repaid**, `repaidTxid` = step 5 | 4,546,455 | 05:33:42 |

**G4 (FR-3.3):** the `released` event carries the spend's own mined height (4,546,446) and was
emitted on the first desk tick that scanned that block — 0 blocks after the spend was mined,
≈100 s after broadcast (well inside the ≤ 2-block AC). **FR-6.5:** `released` moved the open loan
to `at-risk` live.

**G5 (FR-6.1, 6.3, 6.4):** verify → real Bitrefill invoice + redemption → shielded repayment
detected → `repaid`, with no human step beyond starting the gateway with spend permission.

Verification rows at step 2 (underwriter wallet, tip 4,546,442):

| Check | Code | Detail |
|---|---|---|
| C1 protocol | ok | zsol/0.2 on test pool ironwood |
| C2 anchor mined | ok | mined at 4546439, 4 confirmations |
| C3 spend authority | ok | anchor spends 1 note(s) owned by the shared account |
| C4 binding | ok | memo 0xFF‖zs02‖ee134625… matches the recomputed frame digest |
| C5 value | ok | 84970000 zat anchored ≥ 50000000 claimed |
| C6 liveness | ok | 2 anchored note(s) unspent at height 4546442 |
| C7 freshness | ok | issued 229s ago, horizon 24h |
| C8 presentation | ok | Ed25519 signature by sessionKey over this verifier's nonce |

## Found and fixed during this run
1. **Repayment matcher query used non-existent columns** (`is_sent_row`, `mined_height`) of the
   wallet's `v_tx_outputs` view; every tick failed. Fixed to `is_change = 0 AND
   tx_mined_height IS NOT NULL` (`crates/zsol-credit/src/main.rs`). Step 6 is the live proof.
2. **First request ($5) refused by Bitrefill** — test products are sold in $10 steps
   (`range {min 10, max 100, step 10}`). The verification ran (first `verification` event,
   4,546,439) and consumed the challenge before the purchase failed with a typed
   `purchase-failed`. Follow-up: validate the denomination before verifying.
3. CoinGecko returned no price (rate-limited); the Kraken fallback supplied the reference rate.
4. `zsol_release` now passes through the same spend gate (allow + cap) as other broadcasting tools.
