# G5b — Credit Desk refuses a released attestation, live on testnet (2026-10-27)

Demo step 4 (BUILD_PLAN §5): an agent whose proof is no longer backed asks for credit.
Borrower A presents its newest attestation (anchor `20ec2cc8…00ef`, the one it released during
G4) to the running Credit Desk via MCP `credit_request` (test-gift-card-code, $10). No transaction
is needed; the desk challenges, imports the UFVK view-only and runs C1–C8 at tip 4,546,536.

Result: HTTP 422 `claim-only`, reasons `["anchored-note-spent"]`; no Bitrefill invoice is created.

| Check | Result | Code | Detail |
|---|---|---|---|
| C1 protocol | PASS | ok | zsol/0.2 on test pool ironwood |
| C2 anchor mined | PASS | ok | mined at 4546439, 98 confirmations |
| C3 spend authority | PASS | ok | anchor spends 1 note(s) owned by the shared account |
| C4 binding | PASS | ok | memo 0xFF‖zs02‖ee134625… matches the recomputed frame digest |
| C5 value | PASS | ok | 84970000 zat anchored ≥ 50000000 claimed |
| C6 liveness | **FAIL** | `anchored-note-spent` | spent in `8657a142cf9feabd6efa4ab229ccfd8693cb0260f8bf7dca132a58d84d3499ee` at height 4546446 |
| C7 freshness | PASS | ok | issued 2676s ago, horizon 24h |
| C8 presentation | PASS | ok | Ed25519 signature by sessionKey over this verifier's nonce |

Every other check still passes: the attestation was genuine, the funds are simply gone. This is
the case the 0.1 (UIVK) design could not detect (FINDINGS). The detail string was printed with
Rust's `Some(…)` debug form at the time of the run; fixed to a plain height afterwards.
