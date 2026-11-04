# T-32/T-33 — official-client (Anthropic `claude`) MCP evidence — 2026-10-26

- `claude mcp add zsol-local --env ... -- /abs/target/debug/zsol-mcp` →
  `zsol-local: .../zsol-mcp - ✔ Connected` (stdio, initialize handshake passes; official client).
- HTTP: first attempt FAILED — official client rejected `text/plain` content-type
  (CLIENT_HTTP_UNEXPECTED_CONTENT). Fixed http.rs to emit application/json (axum Json),
  rebuilt → `zsol-http-local: http://127.0.0.1:18099/mcp (HTTP) - ✔ Connected`.
- Tool flows (tools/list 9 tools, tools/call zsol_prepare_anchor typed
  balance-below-claim) verified via raw JSON-RPC smokes (see commit 5deae42);
  deep in-runtime tools/call via `claude -p` untested here (CLI credit limit),
  server去除 registered after evidence capture.
