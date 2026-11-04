// T-47 dashboard (FR-6.2): fetches ONLY real endpoints. Down services show DOWN.
import { mcpToolsList, mcpZsolHealth, registryHealth } from "@/lib/endpoints";

export const dynamic = "force-dynamic";

export default async function StatusPage() {
  const [reg, tools, stack] = await Promise.all([
    registryHealth(),
    mcpToolsList(),
    mcpZsolHealth(),
  ]);

  return (
    <>
      <h1>
        zsol <span>status</span>
      </h1>
      <div className="card">
        <h2>registry</h2>
        {"down" in reg && reg.down ? (
          <>
            <span className="status-down">DOWN</span>{" "}
            <small>{reg.detail}</small>
          </>
        ) : (
          <dl className="kv">
            <dt>health</dt>
            <dd>{reg.ok ? <span className="status-ok">OK</span> : <span className="status-down">FAIL</span>}</dd>
            <dt>chainLength</dt>
            <dd>{reg.chainLength}</dd>
            <dt>watermarkHeight (FR-3.4)</dt>
            <dd>{reg.watermarkHeight}</dd>
          </dl>
        )}
      </div>

      <div className="card">
        <h2>MCP gateway (tools/list)</h2>
        {"down" in tools && tools.down ? (
          <>
            <span className="status-down">DOWN</span> <small>{tools.detail}</small>
          </>
        ) : (
          <p>
            <span className="status-ok">
              {tools.tools?.length ?? 0} tools via JSON-RPC 2.0
            </span>
            <br />
            <small>{tools.tools?.join(" · ")}</small>
          </p>
        )}
      </div>

      <div className="card">
        <h2>agent-visible stack (tools/call zsol_health)</h2>
        {"down" in stack && stack.down ? (
          <>
            <span className="status-down">DOWN</span> <small>{stack.detail}</small>
          </>
        ) : (
          <pre style={{ fontSize: "0.75rem", whiteSpace: "pre-wrap" }}>
            {(() => {
              const env = stack as { result?: unknown };
              const content = (env.result as { content?: { text?: string }[] })?.content;
              const text = content?.[0]?.text ?? JSON.stringify(env.result);
              try {
                return JSON.stringify(JSON.parse(String(text)), null, 2);
              } catch {
                return String(text);
              }
            })()}
          </pre>
        )}
      </div>

      <p className="footnote">
        Sources: NEXT_PUBLIC_REGISTRY_URL (/v0/health) and NEXT_PUBLIC_MCP_URL (/mcp,
        streamable HTTP JSON-RPC). Nothing else is rendered here.
      </p>
    </>
  );
}
