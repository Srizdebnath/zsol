# Web control plane evidence (T-46, T-47, T-48 · FR-6.1–6.4) — commits 1dcaf36, 8afa44f

## T-46 scaffold (FR-6.1)

- `web/` — Next 16 App Router, src/ layout, strict TS (`strict`, `noUncheckedIndexedAccess`),
  plain-CSS neobrutalism tokens (`3px` solid borders, `6px 6px 0` hard shadows, mono).
- Build green in CI shape (no env) AND live shape (LAN env baked).

## T-47 dashboard + explorer (FR-6.2/6.3) — LIVE against the running regtest stack

Fetched `GET /` (Next server-rendered) while registry @18088 + MCP @18099 were live:

- registry card: `OK, chainLength 1, watermarkHeight 132` (real `/v0/health`).
- MCP card: `9 tools via JSON-RPC 2.0` (real `tools/list`: zsol_health,
  zsol_prepare_anchor, zsol_anchor, zsol_make_attestation, zsol_publish,
  zsol_share_scope, zsol_import_scope, zsol_verify, zsol_registry_fetch).
- agent-visible stack card: real `tools/call zsol_health` JSON:
  `{"attestAccount":0,"dryRun":false,"registry":{"chainLength":1,"ok":true},
  "zallet":{"ok":true},"zebra":{"height":131,"ok":true}}`.

Fetched `GET /explorer?agentId=regtest-agent-2`:

- entries=1, `chainOk` (web recomputed BLAKE2b-256 over the returned canonical
  object and matched the registry's Rust digest byte-for-byte):
  recomputed `ce9f521d891db5e4…` == server `chainDigest` == `entryId`.
- Cross-language digest equivalence proof (Rust blake2 ⇄ TS @noble/hashes).

Down-state proof: with `NEXT_PUBLIC_*` unset the same build renders explicit
DOWN cards for each card that depends on the missing service. A down service
NEVER renders placeholder data.

## T-48 verify console (FR-6.4)

- Browser → `POST /api/verify` → server relay → MCP `tools/call zsol_verify`.
- Live reply (unwrapped at the relay so the browser gets the verdict):
  `tier: claim-only`, reason `uivk-unavailable`,
  checks `[structural ✓, memo-parse ✓, scope ✗ (honest skip), freshness ✓]`,
  anchorTxid `64f675f120fb07…` — the REAL anchored tx from T-39.
- The console template even refuses to hide errors: unknown shapes render raw
  JSON below the parsed tier; failures keep their §9 reason codes.

## no-mock lint

`grep -rniE "mock|fixture|sample"` over `web/src` = 0 runtime hits (only the
negated "no mock/fixture data" comment lines). Enforced by `scripts/no-mocks.sh`
in CI (T-49).
