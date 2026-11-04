# G1 + G2 — real anchor and independent verification on testnet (2026-10-27)

## Funding (from the owner, shielded)
- txid `2a4fe529d32bbf049729594d98225b026fce6b31a75ec6ac43e54f5e69523ada`, mined 4,544,876,
  1.00000000 TAZ into borrower-a `zsol-attn`, **Ironwood pool**.
- First wallet-built spend: `07514933e2de25639b0d5afb5320fddfbe79bd523f4c19a80db5df3461f7638d`
  (0.1 TAZ zsol-attn → commerce, fee 10,000 zat ZIP-317), mined.

## G1 — anchor (`zsol attest anchor --agent-id agent-a --claim-zat 5000000`)
- anchorTxid `30f12fbd02f03b18efff3114b7a6e4e9a4fd9ad32f714c3e774eba78f0706569`
- anchorHeight 4544949 · pool ironwood · 126 s from command to mined
- frame digest D = `ba3b7162fc7a6811e6e6a01fd3a88750d43a84a75b0238b89ffc3976773eac0c`
- memoBinding `_3pzMDK6O3Fi_HpoEebmoB_TqIdQ1DqEp1sCOLif_Dl2dz6sDA` = `0xFF ‖ "zs02" ‖ D`
- full output: `G1-anchor.json`

## G2 — verification in the underwriter's separate wallet (`zsol verify run`)
The underwriter wallet (`~/.zsol/underwriter`) holds no borrower key material. It imported
borrower-a's `zsol-attn` UFVK view-only at the object's birthday, scanned testnet and ran C1–C8.
Result **verified** at tip 4544963 (scanned 4544963). Full report: `G2-report.json`.

| Check | Result | Code | Detail |
|---|---|---|---|
| C1 protocol | PASS | `ok` | zsol/0.2 on test pool ironwood |
| C2 anchor mined | PASS | `ok` | mined at 4544949, 15 confirmations |
| C3 spend authority | PASS | `ok` | anchor spends 1 note(s) owned by the shared account |
| C4 binding | PASS | `ok` | memo 0xFF‖zs02‖ba3b7162… matches the recomputed frame digest |
| C5 value | PASS | `ok` | 89980000 zat anchored ≥ 5000000 claimed |
| C6 liveness | PASS | `ok` | 2 anchored note(s) unspent at height 4544963 |
| C7 freshness | PASS | `ok` | issued 608s ago, horizon 24h |
| C8 presentation | PASS | `ok` | Ed25519 signature by sessionKey over this verifier's nonce |

Anchored notes seen by the verifier: ironwood#0 84980000 zat, ironwood#1 5000000 zat (binding memo).

### Bug found and fixed during G2
The first run returned `scan-incomplete`: a block arrived while the verifier scanned (tip
4,544,951, scanned 4,544,950). The verifier now re-syncs up to three times and applies the
documented ≤ 1-block tolerance (ARCHITECTURE §5). Every other check passed on both runs.
