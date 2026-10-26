// T-47 explorer (FR-6.2/6.3): agentId → GET /v0/attestations/:agentId, PLUS a
// client-side chain recompute (BLAKE2b-256 over canonical objects) so tampering
// or reordering is visible: chainOk=false renders red with the reason.
import { registryAgent } from "@/lib/endpoints";
import { decodeObjectSummary } from "@/lib/object";

export const dynamic = "force-dynamic";

export default async function ExplorerPage({
  searchParams,
}: {
  searchParams: Promise<{ agentId?: string }>;
}) {
  const { agentId } = await searchParams;

  return (
    <>
      <h1>
        registry <span>explorer</span>
      </h1>
      <form method="get" action="/explorer">
        <input type="text" name="agentId" defaultValue={agentId ?? ""} placeholder="agentId" />
        <button className="btn" type="submit">
          fetch chain
        </button>
      </form>

      {agentId ? (
        <AgentChain agentId={agentId} />
      ) : (
        <p style={{ marginTop: 20 }}>
          Enter an agentId to pull its live chain from the registry.
        </p>
      )}
    </>
  );
}

async function AgentChain({ agentId }: { agentId: string }) {
  const r = await registryAgent(agentId);

  if ("down" in r && r.down) {
    return (
      <div className="card">
        <span className="status-down">REGISTRY DOWN</span> <small>{r.detail}</small>
      </div>
    );
  }

  const chainOk = r.chainOk;
  return (
    <>
      <div className="card">
        <dl className="kv">
          <dt>agentId</dt>
          <dd>{r.agentId}</dd>
          <dt>entries</dt>
          <dd>{r.entries.length}</dd>
          <dt>chain recompute</dt>
          <dd>
            {chainOk ? (
              <span className="status-ok">chainOk</span>
            ) : (
              <span className="status-down">BROKEN — {r.chainError}</span>
            )}
          </dd>
        </dl>
      </div>

      {r.entries.length > 0 && (
        <div className="card">
          <table className="entries">
            <thead>
              <tr>
                <th>seq</th>
                <th>entryId (hex chainDigest)</th>
                <th>claimZat</th>
                <th>anchorHeight</th>
                <th>anchorTxid</th>
                <th>issuedAt</th>
              </tr>
            </thead>
            <tbody>
              {r.entries.map((e) => {
                const s = decodeObjectSummary(e.object);
                return (
                  <tr key={`${e.seq}`}>
                    <td>{e.seq}</td>
                    <td>
                      <code>{e.entryId.slice(0, 16)}…</code>
                    </td>
                    <td>{s?.claimZat ?? "—"}</td>
                    <td>{s?.anchorHeight ?? "—"}</td>
                    <td>
                      <code>{s?.anchorTxid ? `${s.anchorTxid.slice(0, 12)}…` : "—"}</code>
                    </td>
                    <td>{s?.issuedAt ? new Date(s.issuedAt * 1000).toISOString() : "—"}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
          <p className="footnote" style={{ marginTop: 12 }}>
            claimZat is in zatoshi (10⁸ zat = 1 ZEC). Verify a full published object on the
            /verify console.
          </p>
        </div>
      )}
    </>
  );
}
