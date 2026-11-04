# G3 — adversarial cases, live on testnet (2026-10-27)

Command: `ZSOL_LIVE_ATTESTOR=~/.zsol/borrower-a ZSOL_LIVE_ADVERSARY=~/.zsol/borrower-b cargo test --release -p zsol-verifier --test live_testnet -- --ignored --nocapture --test-threads 1`

Each case verifies in a fresh, empty view-only wallet. Each must fail exactly one check (asserted).

| Case | Expected failure | Transaction |
|---|---|---|
| G3a attestor edits agentId after anchoring, re-signs with its genuine session key | C4 `memo-digest-mismatch` | none (uses anchor `30f12fbd…6569`) |
| G3b-1 verifier V1 forwards a presentation made for V1 to V2 | C8 `presentation-invalid` | none |
| G3b-2 V1 swaps in V2's fresh challenge without the session key | C8 `presentation-invalid` | none |
| G3c B anchors 0.0299 TAZ but claims 50 TAZ | C5 `claim-exceeds-outputs` | `60fc81f2e06bc3a351c9828b93080ecf930aab6aac15325dda486a6e7f868b67` @4,544,995 |
| G3d B, holding only A's UFVK, sends its own 20,000 zat into A's zsol-attn with a memo binding B's session key (the 0.1 flaw F-3) | C3 `not-spent-by-account` | `45f286019ddebd0aeeeffce9242ae09e0a58215a78f42d3adaf09e295e2ebe2a` @4,544,997 |

Result: `test result: ok. 4 passed; 0 failed` in 269 s.

## Raw output

```

running 4 tests
test g3a_edited_agent_id_fails_binding ... G3a edited agentId (re-signed by the genuine session key):
    PASS C1 protocol          ok                       zsol/0.2 on test pool ironwood
    PASS C2 anchor mined      ok                       mined at 4544949, 45 confirmations
    PASS C3 spend authority   ok                       anchor spends 1 note(s) owned by the shared account
    FAIL C4 binding           memo-digest-mismatch     no anchor output carries this object's digest
    PASS C5 value             ok                       89980000 zat anchored ≥ 5000000 claimed
    PASS C6 liveness          ok                       2 anchored note(s) unspent at height 4544993
    PASS C7 freshness         ok                       issued 1364s ago, horizon 24h
    PASS C8 presentation      ok                       Ed25519 signature by sessionKey over this verifier's nonce
ok
test g3b_relayed_presentation_fails ... G3b-1 V1 forwards A's presentation (made for V1) to V2:
    PASS C1 protocol          ok                       zsol/0.2 on test pool ironwood
    PASS C2 anchor mined      ok                       mined at 4544949, 46 confirmations
    PASS C3 spend authority   ok                       anchor spends 1 note(s) owned by the shared account
    PASS C4 binding           ok                       memo 0xFF‖zs02‖ba3b7162… matches the recomputed frame digest
    PASS C5 value             ok                       89980000 zat anchored ≥ 5000000 claimed
    PASS C6 liveness          ok                       2 anchored note(s) unspent at height 4544994
    PASS C7 freshness         ok                       issued 1387s ago, horizon 24h
    FAIL C8 presentation      presentation-invalid     challenge was issued to a different verifier
G3b-2 V1 swaps in V2's fresh challenge but cannot re-sign:
    PASS C1 protocol          ok                       zsol/0.2 on test pool ironwood
    PASS C2 anchor mined      ok                       mined at 4544949, 46 confirmations
    PASS C3 spend authority   ok                       anchor spends 1 note(s) owned by the shared account
    PASS C4 binding           ok                       memo 0xFF‖zs02‖ba3b7162… matches the recomputed frame digest
    PASS C5 value             ok                       89980000 zat anchored ≥ 5000000 claimed
    PASS C6 liveness          ok                       2 anchored note(s) unspent at height 4544994
    PASS C7 freshness         ok                       issued 1409s ago, horizon 24h
    FAIL C8 presentation      presentation-invalid     signature does not verify for this challenge
ok
test g3c_overclaim_fails_value ... G3c over-claim anchor 60fc81f2e06bc3a351c9828b93080ecf930aab6aac15325dda486a6e7f868b67 at 4544995:
    PASS C1 protocol          ok                       zsol/0.2 on test pool ironwood
    PASS C2 anchor mined      ok                       mined at 4544995, 1 confirmations
    PASS C3 spend authority   ok                       anchor spends 1 note(s) owned by the shared account
    PASS C4 binding           ok                       memo 0xFF‖zs02‖7d66b961… matches the recomputed frame digest
    FAIL C5 value             claim-exceeds-outputs    2990000 zat anchored < 5000000000 claimed
    PASS C6 liveness          ok                       2 anchored note(s) unspent at height 4544995
    PASS C7 freshness         ok                       issued 75s ago, horizon 24h
    PASS C8 presentation      ok                       Ed25519 signature by sessionKey over this verifier's nonce
ok
test g3d_donation_forgery_fails_spend_authority ... G3d donation 45f286019ddebd0aeeeffce9242ae09e0a58215a78f42d3adaf09e295e2ebe2a at 4544997 (attacker funds → victim zsol-attn, attacker's memo):
    PASS C1 protocol          ok                       zsol/0.2 on test pool ironwood
    PASS C2 anchor mined      ok                       mined at 4544997, 1 confirmations
    FAIL C3 spend authority   not-spent-by-account     anchor spends no note of the shared account
    PASS C4 binding           ok                       memo 0xFF‖zs02‖6f9b461e… matches the recomputed frame digest
    PASS C5 value             ok                       20000 zat anchored ≥ 20000 claimed
    PASS C6 liveness          ok                       1 anchored note(s) unspent at height 4544997
    PASS C7 freshness         ok                       issued 90s ago, horizon 24h
    PASS C8 presentation      ok                       Ed25519 signature by sessionKey over this verifier's nonce
ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 269.08s

```
