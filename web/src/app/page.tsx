// FR-7.1 stack status. Every value is fetched server-side, per request, from the live services:
//   registry        NEXT_PUBLIC_REGISTRY_URL  GET /v0/health
//   verifier MCP    NEXT_PUBLIC_MCP_URL       POST /mcp tools/list + tools/call zsol_health
//   Credit Desk     NEXT_PUBLIC_CREDIT_URL    GET /v0/health
// A service that does not answer renders DOWN with the reason. Nothing is filled in.
import {
  NU7_BRANCH,
  creditHealth,
  mcpToolsList,
  mcpZsolHealthParsed,
  registryHealth,
  rustResult,
  type ChainInfo,
  type Probe,
  type RustResult,
} from "@/lib/endpoints";
import { fmtTaz } from "@/lib/format";
import { AutoRefresh } from "@/components/AutoRefresh";

export const dynamic = "force-dynamic";

function Down({ detail }: { detail: string }) {
  return (
    <p style={{ margin: 0 }}>
      <span className="status-down">DOWN</span> <small>{detail}</small>
    </p>
  );
}

function ChainRows({ chain }: { chain: RustResult<ChainInfo> }) {
  const r = rustResult(chain);
  if (!r.ok) {
    return (
      <>
        <dt>chain (boot check)</dt>
        <dd>
          <span className="status-down">DOWN</span> <small>{r.error}</small>
        </dd>
      </>
    );
  }
  const c = r.value;
  const nu7 = c.consensusBranchId.toLowerCase().replace(/^0x/, "") === NU7_BRANCH;
  return (
    <>
      <dt>lightwalletd</dt>
      <dd>
        {c.endpoint} <small>({c.vendor} {c.version})</small>
      </dd>
      <dt>network</dt>
      <dd>{c.chainName}</dd>
      <dt>consensus branch</dt>
      <dd>
        <code>0x{c.consensusBranchId}</code>{" "}
        {nu7 ? (
          <span className="status-ok">NU7</span>
        ) : (
          <span className="status-down">not NU7 (expected 0x{NU7_BRANCH})</span>
        )}
      </dd>
      <dt>tip height</dt>
      <dd>
        <strong>{c.tipHeight}</strong>
      </dd>
    </>
  );
}

function ProbeCell({ p }: { p: Probe }) {
  if (typeof p === "string") return <small>{p}</small>;
  if (p.error) {
    return (
      <>
        <span className="status-down">DOWN</span> <small>{p.url} · {p.error}</small>
      </>
    );
  }
  const good = typeof p.status === "number" && p.status < 400;
  return (
    <>
      <span className={good ? "status-ok" : "status-down"}>HTTP {p.status}</span> <small>{p.url}</small>
    </>
  );
}

function BitrefillBalance({ r }: { r: RustResult<unknown> }) {
  const b = rustResult(r);
  if (!b.ok) return <small>{b.error}</small>;
  const v = b.value as { balance?: unknown; currency?: unknown } | null;
  if (v && typeof v === "object" && "balance" in v) {
    return (
      <span>
        {String(v.balance)} {String(v.currency ?? "")}
      </span>
    );
  }
  return <code>{JSON.stringify(b.value)}</code>;
}

export default async function StatusPage() {
  const [reg, tools, gw, desk] = await Promise.all([
    registryHealth(),
    mcpToolsList(),
    mcpZsolHealthParsed(),
    creditHealth(),
  ]);

  return (
    <>
      <h1>
        zsol <span>status</span>
      </h1>
      <AutoRefresh seconds={25} />

      <div className="card">
        <h2>claims registry</h2>
        {"down" in reg && reg.down ? (
          <Down detail={reg.detail} />
        ) : (
          <dl className="kv">
            <dt>health</dt>
            <dd>{reg.ok ? <span className="status-ok">OK</span> : <span className="status-down">FAIL</span>}</dd>
            <dt>chain length</dt>
            <dd>{reg.chainLength}</dd>
            <dt>watermark height (FR-3.4)</dt>
            <dd>{reg.watermarkHeight}</dd>
          </dl>
        )}
      </div>

      <div className="card">
        <h2>verifier MCP gateway</h2>
        {"down" in gw && gw.down ? (
          <Down detail={gw.detail} />
        ) : (
          <dl className="kv">
            <dt>tools (JSON-RPC 2.0)</dt>
            <dd>
              {"down" in tools && tools.down ? (
                <small>tools/list failed: {tools.detail}</small>
              ) : (
                <>
                  <span className="status-ok">{tools.tools.length} tools</span>{" "}
                  <small>{tools.tools.join(" · ")}</small>
                </>
              )}
            </dd>
            <ChainRows chain={gw.health.chain} />
            <dt>verifier wallet</dt>
            <dd>
              {gw.health.wallet.exists ? (
                <span className="status-ok">present</span>
              ) : (
                <span className="status-down">missing</span>
              )}
            </dd>
            <dt>registry (as seen by gateway)</dt>
            <dd>
              <ProbeCell p={gw.health.registry} />
            </dd>
            <dt>credit desk (as seen by gateway)</dt>
            <dd>
              <ProbeCell p={gw.health.creditDesk} />
            </dd>
            <dt>spending tools</dt>
            <dd>
              {gw.health.spend.allowed ? `enabled · cap ${fmtTaz(gw.health.spend.capZat)}` : "disabled (view-only)"}
            </dd>
          </dl>
        )}
      </div>

      <div className="card">
        <h2>Credit Desk (underwriter)</h2>
        {"down" in desk && desk.down ? (
          <Down detail={desk.detail} />
        ) : (
          <>
            <dl className="kv">
              <dt>health</dt>
              <dd>
                {desk.health.ok ? (
                  <span className="status-ok">OK</span>
                ) : (
                  <span className="status-down">DEGRADED</span>
                )}
              </dd>
              <ChainRows chain={desk.health.chain} />
              <dt>treasury</dt>
              <dd>
                {desk.health.treasury ? (
                  <>
                    <strong>{fmtTaz(desk.health.treasury.spendableZat)}</strong> spendable
                    <br />
                    <small>
                      total {fmtTaz(desk.health.treasury.totalZat)} · pending{" "}
                      {fmtTaz(desk.health.treasury.pendingZat)}
                    </small>
                  </>
                ) : (
                  <span className="status-down">no treasury account</span>
                )}
              </dd>
              <dt>Bitrefill API</dt>
              <dd>
                {desk.health.bitrefill ? (
                  <span className="status-ok">reachable</span>
                ) : (
                  <span className="status-down">DOWN</span>
                )}{" "}
                <BitrefillBalance r={desk.health.bitrefillBalance} />
              </dd>
            </dl>
            <h2 style={{ marginTop: 20 }}>policy (as served)</h2>
            <dl className="kv">
              <dt>underwriter</dt>
              <dd>
                {desk.health.policy.underwriterId} <small>v{desk.health.policy.version}</small>
              </dd>
              <dt>mode</dt>
              <dd>
                <span className="badge" style={{ background: "var(--yellow)" }}>
                  {desk.health.policy.mode}
                </span>
              </dd>
              <dt>loan-to-value</dt>
              <dd>{(desk.health.policy.ltvBps / 100).toFixed(2)}% of min(claim, anchored)</dd>
              <dt>max loan</dt>
              <dd>${desk.health.policy.maxLoanUsd}</dd>
              <dt>max tenor</dt>
              <dd>{desk.health.policy.maxTenorBlocks} blocks</dd>
              <dt>min confirmations</dt>
              <dd>{desk.health.policy.minConf}</dd>
              <dt>rate max age</dt>
              <dd>{desk.health.policy.rateMaxAgeSecs}s</dd>
              <dt>allowed products</dt>
              <dd>{desk.health.policy.allowedProducts.join(" · ")}</dd>
            </dl>
          </>
        )}
      </div>

      <p className="footnote">
        Sources: NEXT_PUBLIC_REGISTRY_URL (/v0/health), NEXT_PUBLIC_MCP_URL (/mcp tools/list and
        tools/call zsol_health) and NEXT_PUBLIC_CREDIT_URL (/v0/health), fetched on the server for
        every page load. Chain rows come from each service&apos;s own boot check, which fails unless
        the endpoint&apos;s branch matches the branch local consensus derives for the tip.
      </p>
    </>
  );
}
