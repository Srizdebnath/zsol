# zsol demo topology (T-41, FR-5.2)

Canonical mapping of roles to machines/services for the FR-5.x demo and the
G4 gate. Two independent Zallet instances — never a shared wallet.

```
┌──────────────────────────── host / operator ────────────────────────────┐
│  z3 stack (T-31 attach):                                                │
│   zebra  ───── testnet Node (RPC cookie auth, read-only cookie volume)   │
│   zaino  ───── indexer (wallet view provider)                           │
│   zallet-attn ─── ATTESTOR Zallet (accounts: 9 = attest, 91/92 = shares) │
│   zallet-verif ── VERIFIER Zallet (fresh wallet, funded 0.05 TAZ T-06)   │
│  zsol services:                                                         │
│   zsol-registry (LAN; FR-3.4 hardening on when public)                  │
│   zsol-mcp (HTTP :8099) + web control plane (FR-6.x)                    │
└─────────────────────────────────────────────────────────────────────────┘
```

| Role | Machine | Holds | Uses | Never holds |
|---|---|---|---|---|
| **Operator** | host | z3 control, registry, web | RPC-router, doctor pins (T-04) | any spend keys |
| **Attestor agent** | attestor box, its own `claude` | Zallet wallet (seed) | attestor flow: prepare→anchor→make→publish→share_scope | verifier-side keys |
| **Verifier agent/merchant** | verifier box, its own `claude` | fresh Zallet wallet | import_scope→registry_fetch→verify | any spend key of attestor |

## Why two Zallets (SRS open-question 2)

- The attestor's scope material (uivk) must be PRODUCED by a wallet the
  attestor controls and IMPORTED into a wallet the verifier controls — one
  process verifying its own attestations proves nothing.
- `zsol_share_scope` runs on the attestor side, refuses account #0 and the
  master attest account (invariant 5), and maps each peer label onto a
  dedicated sub-account (`ZSOL_SHARE_ACCOUNTS`, default 91/92 → per-verifier
  scoping for T-44).
- The verifier wallet stays independent: its balances are its own; importing
  a uivk adds VIEW access only (FR-2.7 / annex T-45).

## Data flow at a glance

1. attestor agent anchors self-spend (memo = zs01‖ver‖window‖D-frame).
2. attestor agent derives object (+ min conf waits) and publishes (registry
   gives entryId = hex(chainDigest), append-only chain, T-26..T-30).
3. attestor agent hands the object + scoped uivk to the merchant over a
   DIRECT channel (registry never stores uivk material — invariant 6).
4. merchant agent imports uivk, scans to tip, runs zsol_verify = all §6
   checks against ITS OWN zebra/zallet view of the chain.
5. merchant prices per `config/pricing-policy.json` (T-42): verified tier
   discount applies only to §6-passing objects within maxAgeHours.
