# zsol web (Next 16, FR-6.x)

Strict-TS App Router control plane for zsol. Neobrutalism tokens live in
`src/app/globals.css` (3px hard borders, hard shadows, mono type).

## Run

```bash
pnpm install
NEXT_PUBLIC_REGISTRY_URL=http://127.0.0.1:8088 \
NEXT_PUBLIC_MCP_URL=http://127.0.0.1:8099 \
pnpm dev    # or: pnpm build && pnpm start
```

| Page | Real sources (no other data paths exist) |
|---|---|
| `/` | `GET {REGISTRY}/v0/health` + `POST {MCP}/mcp` `tools/list` & `tools/call zsol_health` |
| `/explorer` | `GET {REGISTRY}/v0/attestations/:agentId` + client-side chain recompute: recomputes every `chainDigest` from the returned objects (`BLAKE2b-256(prev ‖ canonical(object))`, SPEC §7) — tampering renders `chainOk=false` with a reason |
| `/verify` | `POST /api/verify` → server relay → MCP `tools/call zsol_verify` (per-check rows exactly as the gateway replied) |

Down services render explicit DOWN states. `grep -rniE  "mock|fixture|sample"` is
guarded in CI (FR-7.2).
