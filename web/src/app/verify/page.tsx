"use client";

// FR-7.3 judge self-serve verification. Step 1 asks the verifier gateway for a fresh challenge;
// the attestor answers it (MCP tool zsol_present) with {object, ufvk, challenge, signature};
// step 2 runs C1–C8 in the gateway's own wallet. Rows are rendered exactly as returned.
// Shapes mirror zsol_verifier::Report and zsol_types::CheckRow (serde camelCase; tier and
// reason codes kebab-case). Tool failures arrive as {error: true, reason, detail}.
import { useState } from "react";
import { Copyable } from "@/components/Copyable";
import { fmtTaz } from "@/lib/format";

type Row = { check: string; ok: boolean; code: string; detail: string };
type Note = {
  pool: string;
  outputIndex: number;
  valueZat: number;
  binding: boolean;
  spentInTxid: string | null;
  spentHeight: number | null;
};
type Report = {
  tier: "verified" | "claim-only";
  reasons: string[];
  checks: Row[];
  agentId: string;
  claimZat: number;
  anchorTxid: string;
  anchoredZat: number;
  notes?: Note[];
  tipHeight: number;
  scannedHeight: number | null;
  verifiedAt: number;
};
type ToolFailure = { error: unknown; reason?: unknown; detail?: unknown };

function isReport(v: unknown): v is Report {
  return !!v && typeof v === "object" && "tier" in v && Array.isArray((v as Report).checks);
}

function describeFailure(v: unknown): string {
  if (v && typeof v === "object" && "error" in v) {
    const f = v as ToolFailure;
    const reason = f.reason !== undefined ? String(f.reason) : typeof f.error === "string" ? f.error : "error";
    const detail = typeof f.detail === "string" ? f.detail : f.detail !== undefined ? JSON.stringify(f.detail) : "";
    return detail ? `${reason}: ${detail}` : reason;
  }
  return JSON.stringify(v, null, 2);
}

export default function VerifyPage() {
  const [challenge, setChallenge] = useState("");
  const [presentation, setPresentation] = useState("");
  const [busy, setBusy] = useState<"" | "challenge" | "verify">("");
  const [report, setReport] = useState<Report | null>(null);
  const [error, setError] = useState("");

  async function issue() {
    setBusy("challenge");
    setError("");
    try {
      const r = await fetch("/api/challenge", { method: "POST" });
      const j: unknown = await r.json();
      if (!r.ok) throw new Error(describeFailure(j));
      setChallenge(JSON.stringify(j, null, 2));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy("");
    }
  }

  async function verify() {
    setBusy("verify");
    setError("");
    setReport(null);
    try {
      let parsed: { presentation?: unknown };
      try {
        parsed = JSON.parse(presentation) as { presentation?: unknown };
      } catch {
        throw new Error("presentation is not valid JSON");
      }
      // accept the whole zsol_present reply ({presentation, disclosure}) or the bare presentation
      const p = parsed.presentation ?? parsed;
      const r = await fetch("/api/verify", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ presentation: p }),
      });
      const j: unknown = await r.json();
      if (!r.ok || !isReport(j)) setError(describeFailure(j));
      else setReport(j);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy("");
    }
  }

  return (
    <>
      <h1>
        verify <span>live</span>
      </h1>
      <div className="card">
        <h2>1 · issue a challenge</h2>
        <p style={{ marginTop: 0 }}>
          A fresh 32-byte nonce bound to this verifier, valid for 10 minutes. Give it to the
          attestor agent, which answers with <code>zsol_present</code>.
        </p>
        <button className="btn" onClick={issue} disabled={busy !== ""}>
          {busy === "challenge" ? "issuing…" : "issue challenge"}
        </button>
        {challenge && (
          <>
            <pre style={{ fontSize: "0.75rem", whiteSpace: "pre-wrap" }}>{challenge}</pre>
            <Copyable value={JSON.stringify(JSON.parse(challenge))} />
          </>
        )}
      </div>

      <div className="card">
        <h2>2 · verify the presentation</h2>
        <p style={{ marginTop: 0 }}>
          Paste the attestor&apos;s presentation JSON. The gateway imports the UFVK view-only, scans
          testnet from the account birthday and runs checks C1–C8. This takes a few seconds.
        </p>
        <textarea value={presentation} onChange={(e) => setPresentation(e.target.value)} placeholder='{"object": …, "ufvk": "uviewtest1…", "challenge": …, "signature": …}' />
        <div style={{ marginTop: 12 }}>
          <button className="btn" onClick={verify} disabled={busy !== "" || presentation.trim() === ""}>
            {busy === "verify" ? "scanning testnet…" : "verify"}
          </button>
        </div>
      </div>

      {error && (
        <div className="card">
          <span className="status-down">error</span>
          <pre style={{ fontSize: "0.75rem", whiteSpace: "pre-wrap" }}>{error}</pre>
        </div>
      )}

      {report && (
        <div className="card">
          <h2>
            tier:{" "}
            <span className={report.tier === "verified" ? "tier-verified" : "tier-claim-only"}>{report.tier}</span>
          </h2>
          {report.reasons.length > 0 && (
            <p>
              reasons:{" "}
              {report.reasons.map((r) => (
                <span key={r} className="badge" style={{ background: "var(--pink)", marginRight: 6 }}>
                  {r}
                </span>
              ))}
            </p>
          )}
          <dl className="kv">
            <dt>agent</dt>
            <dd>{report.agentId}</dd>
            <dt>claimed</dt>
            <dd>
              {fmtTaz(report.claimZat)} <small>({report.claimZat} zat)</small>
            </dd>
            <dt>anchored (unspent)</dt>
            <dd>
              {fmtTaz(report.anchoredZat)} <small>({report.anchoredZat} zat)</small>
            </dd>
            <dt>anchor tx</dt>
            <dd>
              <Copyable value={report.anchorTxid} tx full />
            </dd>
            <dt>checked at height</dt>
            <dd>
              {report.tipHeight}
              {report.scannedHeight !== null && report.scannedHeight !== report.tipHeight && (
                <small> (scanned to {report.scannedHeight})</small>
              )}
            </dd>
            <dt>verified at</dt>
            <dd>{new Date(report.verifiedAt * 1000).toLocaleString()}</dd>
          </dl>
          {report.checks.map((c, i) => (
            <div key={i} className={`check-row ${c.ok ? "check-pass" : "check-fail"}`}>
              <span className="check-name">{c.check}</span>
              <span>{c.ok ? "PASS" : `FAIL · ${c.code}`}</span>
              <small>{c.detail}</small>
            </div>
          ))}
          {report.notes && report.notes.length > 0 && (
            <div className="table-wrap" style={{ marginTop: 16 }}>
              <table className="entries">
                <thead>
                  <tr>
                    <th>pool</th>
                    <th>output</th>
                    <th>value</th>
                    <th>bound to object</th>
                    <th>spent in</th>
                  </tr>
                </thead>
                <tbody>
                  {report.notes.map((n) => (
                    <tr key={`${n.pool}-${n.outputIndex}`}>
                      <td>{n.pool}</td>
                      <td>{n.outputIndex}</td>
                      <td>{fmtTaz(n.valueZat)}</td>
                      <td>{n.binding ? "yes" : "no"}</td>
                      <td>{n.spentInTxid ? (
                          <>
                            <Copyable value={n.spentInTxid} tx />
                            {n.spentHeight !== null && <small> h={n.spentHeight}</small>}
                          </>
                        ) : (
                          "unspent"
                        )}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      )}
    </>
  );
}
