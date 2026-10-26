"use client";

// T-48 (FR-6.4): /verify console → zsol-mcp HTTP zsol_verify. Tier + per-check
// list rendered from the REAL tool reply; failures keep their §9 reason codes.
import { useState } from "react";

type CheckRow = { check?: string; name?: string; ok?: boolean; pass?: boolean; detail?: string; reason?: string };
type VerifyPayload = {
  tier?: string;
  ok?: boolean;
  checks?: CheckRow[];
  [k: string]: unknown;
};

export default function VerifyPage() {
  const [obj, setObj] = useState("");
  const [pending, setPending] = useState(false);
  const [reply, setReply] = useState<string>("");

  async function run() {
    setPending(true);
    setReply("");
    try {
      const res = await fetch("/api/verify", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ object: obj.trim() }),
      });
      const j = (await res.json()) as VerifyPayload | { error: string; detail?: string };
      setReply(JSON.stringify(j, null, 2));
    } catch (e) {
      setReply(`fetch error: ${e instanceof Error ? e.message : "unknown"}`);
    } finally {
      setPending(false);
    }
  }

  // Rendered strictly from what came back; unknown shapes render as raw JSON.
  let parsed: VerifyPayload | null = null;
  try {
    parsed = JSON.parse(reply) as VerifyPayload;
  } catch {
    parsed = null;
  }
  const checks = Array.isArray(parsed?.checks) ? parsed!.checks! : [];

  return (
    <>
      <h1>
        verifying <span>console</span>
      </h1>
      <div className="card">
        <p style={{ marginTop: 0 }}>
          Paste an attestation object (b64u of canonical JSON, as published via the registry or
          handed over off-band). The server relays it to the MCP gateway&apos;s{" "}
          <code>zsol_verify</code> — every §6 check re-runs against the verifier&apos;s own
          stack. This page never fabricates rows.
        </p>
        <textarea
          value={obj}
          onChange={(e) => setObj(e.target.value)}
          placeholder="b64u attestation object…"
        />
        <div style={{ marginTop: 12 }}>
          <button className="btn" onClick={run} disabled={pending || obj.trim().length === 0}>
            {pending ? "verifying…" : "verify"}
          </button>
        </div>
      </div>

      {parsed && !("error" in (parsed as object)) && (
        <div className="card">
          <h2>
            result:{" "}
            {parsed.tier === "verified" ? (
              <span className="tier-verified">{parsed.tier}</span>
            ) : (
              <span className="tier-claim-only">{parsed.tier ?? "see raw"}</span>
            )}
          </h2>
          {checks.length > 0 && (
            <>
              {checks.map((c, i) => (
                <div
                  key={i}
                  className={`check-row ${
                    c.pass === true || c.ok === true ? "check-pass" : c.pass === false || (c.ok === false) ? "check-fail" : "check-unknown"
                  }`}
                >
                  <span className="check-name">{c.check ?? c.name ?? "check"}</span>
                  <span>{c.pass ? "PASS" : `FAIL${c.reason ? ` · ${c.reason}` : ""}`}</span>
                  {c.detail && <small>{c.detail}</small>}
                </div>
              ))}
            </>
          )}
          <h3 style={{ marginBottom: 4 }}>raw tool reply</h3>
          <pre style={{ fontSize: "0.7rem", whiteSpace: "pre-wrap" }}>{reply}</pre>
        </div>
      )}

      {parsed && "error" in (parsed as object) && (
        <div className="card">
          <span className="status-down">gateway error</span>
          <pre style={{ fontSize: "0.75rem", whiteSpace: "pre-wrap" }}>{reply}</pre>
        </div>
      )}
    </>
  );
}
