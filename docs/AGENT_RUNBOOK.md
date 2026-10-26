# zsol agent runbook (T-40, FR-5.2)

Zero-mock contract: everything here points at real binaries and real endpoints.
Every spend-gated step notes when it leans on `ZSOL_MCP_DRY_RUN=0` (FR-4.5).

## 0. Build

```bash
cd zsol && cargo build --release -p zsol-mcp
# binary: target/release/zsol-mcp  (stdio MCP gateway + optional HTTP)
```

## 1. Register the gateway in Claude Code

**stdio (personal, single agent):**

```bash
claude mcp add zsol \
  --env ZEBRA_RPC_URL=http://127.0.0.1:18232 \
  --env ZALLET_RPC_URL=http://127.0.0.1:14232 \
  --env ZSOL_REGISTRY_URL=http://127.0.0.1:8088 \
  --env ZSOL_MCP_DRY_RUN=1 \
  -- /ABS/PATH/TO/zsol/target/release/zsol-mcp
```

**streamable HTTP (shared gateway, FR-4.2)** — run the HTTP transport first:

```bash
ZSOL_MCP_HTTP_BIND=127.0.0.1:8099 ZSOL_MCP_NO_STDIO=1 \
ZEBRA_RPC_URL=http://127.0.0.1:18232 ZALLET_RPC_URL=http://127.0.0.1:14232 \
ZSOL_REGISTRY_URL=http://127.0.0.1:8088 \
  target/release/zsol-mcp &

claude mcp add --transport http zsol-http http://127.0.0.1:8099/mcp
```

Verify visibility (FR-4.1 AC): `claude mcp list` must show the 9 zsol tools —
`zsol_health`, `zsol_prepare_anchor`, `zsol_anchor`, `zsol_make_attestation`,
`zsol_publish`, `zsol_share_scope`, `zsol_import_scope`, `zsol_verify`,
`zsol_registry_fetch`.

## 2. Boot-time honesty checks

- `zsol_health` — verifies zebra reachable, zallet verbs pinned, registry chain.
- `zsol_prepare_anchor {claimZat: 1000000}` — dry-run by construction; plan +
  balance check only, nothing is ever sent from this tool.

## 3. Attestor flow (dry-run gains a PLAN; ZSOL_MCP_DRY_RUN=0 performs sends)

Sequential prompt batch for the demo agent (FR-5.2, quotes are the demo's
claim payloads for T-43/G4):

1. `zsol_prepare_anchor {"claimZat": 1000000}` → "Check my attest account can
   anchor 0.01 TAZ; show the plan."
2. `zsol_anchor {"agentId": "agent-0001", "claimZat": 1000000}` →
   "Anchor 0.01 TAZ to my attest account for agent-0001." **(spend)**
3. `zsol_make_attestation {}` → "Turn my last anchor into a signed claim object."
4. `zsol_publish {"object": "<b64u from step 3>"}` → "Publish to the registry."
5. `zsol_share_scope {"peerLabel": "merchant-v1"}` → "Give the merchant a scoped
   viewing key (never account 0, never master)."

## 4. Verifier flow (second Zallet, second agent machine — T-41 topology)

1. `zsol_import_scope {"uivk": "<scope from attestor>", "label": "merchant-v1"}`
2. `zsol_registry_fetch {"agentId": "agent-0001"}` → recomputed chainOk per entry.
3. `zsol_verify {"object": "<b64u>"}` → tier `verified`/`claim-only` + per-check
   rows with §9 reasons (§6 checks all run against the verifier's OWN stack).

## 5. Honest limits (annex pointer)

- Dry-run mode never sends (FR-4.5). Anything requiring a send says so.
- A shared uivk cannot be revoked; scopes are burned on conflict (annex, T-45).
- The registry is evidence ABOUT claims; solvency truth stays with the chain.
