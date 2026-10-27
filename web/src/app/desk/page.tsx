"use client";

// FR-7.4 Credit Desk board. Loans and events come straight from zsol-credit
// (NEXT_PUBLIC_CREDIT_URL): an initial GET /v0/credit/loans, the chain tip from /v0/health,
// then the live SSE stream (/v0/events). Shapes mirror crates/zsol-credit/src/db.rs (Loan,
// Event; serde camelCase). If the desk is unreachable the page says so; it never invents rows.
import { useCallback, useEffect, useState } from "react";
import QRCode from "qrcode";
import { Copyable } from "@/components/Copyable";
import { fmtTaz, shortHex } from "@/lib/format";

const CREDIT_URL = (process.env.NEXT_PUBLIC_CREDIT_URL ?? "").replace(/\/$/, "");
const PUBLIC_URL = (process.env.NEXT_PUBLIC_PUBLIC_URL ?? "").replace(/\/$/, "");

type LoanStatus = "open" | "at-risk" | "repaid" | "defaulted";

/** crates/zsol-credit/src/db.rs `Loan`. */
type Loan = {
  id: string;
  agentId: string;
  objectB64u: string;
  anchorTxid: string;
  status: LoanStatus;
  principalZat: number;
  valueUsd: number;
  usdPerZec: number;
  rateSource: string;
  rateAt: number;
  productId: string;
  bitrefillInvoiceId: string;
  bitrefillOrderId: string | null;
  repayTo: string;
  memoHex: string;
  dueHeight: number;
  openedHeight: number;
  repaidTxid: string | null;
  createdAt: number;
  updatedAt: number;
};

/** crates/zsol-credit/src/db.rs `Event`. */
type DeskEvent = {
  id: number;
  loanId: string | null;
  kind: string;
  height: number | null;
  detail: Record<string, unknown> | null;
  at: number;
};

const SSE_KINDS = [
  "tick",
  "verification",
  "loan-opened",
  "attestation-released",
  "attestation-expired",
  "loan-repaid",
  "repayment-short",
  "loan-defaulted",
] as const;

/** Detail keys whose values are Zcash txids (copyable, explorer-linked when configured). */
const TXID_KEYS = new Set(["txid", "anchorTxid", "spentInTxid"]);
const ZAT_KEYS = new Set(["principalZat", "valueZat", "anchoredZat"]);

function LoanBadge({ status }: { status: string }) {
  return <span className={`loan-status loan-${status}`}>{status}</span>;
}

function eventTone(e: DeskEvent): string {
  const d = e.detail ?? {};
  switch (e.kind) {
    case "loan-opened":
    case "loan-repaid":
      return "check-pass";
    case "verification":
      return d.tier === "verified" ? "check-pass" : "check-fail";
    case "attestation-released":
    case "loan-defaulted":
    case "repayment-short":
    case "attestation-expired":
      return "check-fail";
    default:
      return "check-unknown";
  }
}

function EventDetail({ detail }: { detail: DeskEvent["detail"] }) {
  if (!detail || typeof detail !== "object") return null;
  return (
    <span className="event-detail">
      {Object.entries(detail).map(([k, v]) => (
        <span key={k}>
          <small>{k}=</small>
          {TXID_KEYS.has(k) && typeof v === "string" ? (
            <Copyable value={v} tx />
          ) : k === "bitrefillInvoiceId" && typeof v === "string" ? (
            <Copyable value={v} full />
          ) : ZAT_KEYS.has(k) && typeof v === "number" ? (
            <code>{fmtTaz(v)}</code>
          ) : k === "newStatus" && typeof v === "string" ? (
            <LoanBadge status={v} />
          ) : (
            <code>{typeof v === "string" ? v : JSON.stringify(v)}</code>
          )}
        </span>
      ))}
    </span>
  );
}

function VerifyQr() {
  const [url, setUrl] = useState("");
  const [svg, setSvg] = useState("");
  const [err, setErr] = useState("");
  const [local, setLocal] = useState(false);

  useEffect(() => {
    const base = PUBLIC_URL || window.location.origin;
    const u = `${base}/verify`;
    setUrl(u);
    try {
      const h = new URL(u).hostname;
      setLocal(h === "localhost" || h === "127.0.0.1" || h === "::1" || h === "[::1]");
    } catch {
      setLocal(false);
    }
    QRCode.toString(u, { type: "svg", margin: 1, errorCorrectionLevel: "M" })
      .then(setSvg)
      .catch((e: unknown) => setErr(e instanceof Error ? e.message : String(e)));
  }, []);

  return (
    <div className="card">
      <h2>judges: verify it on your phone</h2>
      <div className="qr">
        <div className="qr-box" aria-label={`QR code for ${url}`}>
          {svg ? <div dangerouslySetInnerHTML={{ __html: svg }} style={{ width: "100%", height: "100%" }} /> : null}
        </div>
        <div style={{ flex: 1, minWidth: 220 }}>
          <p style={{ marginTop: 0 }}>
            Scan to open the public verifier console. Ask the gateway for a challenge, have the
            attestor agent answer it, and run C1–C8 yourself against testnet.
          </p>
          <Copyable value={url || null} full />
          {err && (
            <p>
              <span className="status-down">QR failed</span> <small>{err}</small>
            </p>
          )}
          {local && (
            <p>
              <span className="status-down">localhost</span>{" "}
              <small>
                a phone cannot reach this address; set NEXT_PUBLIC_PUBLIC_URL to the public or LAN URL.
              </small>
            </p>
          )}
        </div>
      </div>
    </div>
  );
}

export default function DeskPage() {
  const [loans, setLoans] = useState<Loan[] | null>(null);
  const [events, setEvents] = useState<DeskEvent[]>([]);
  const [tip, setTip] = useState<{ height: number; source: string; at: number } | null>(null);
  const [down, setDown] = useState("");
  const [live, setLive] = useState(false);

  const refresh = useCallback(async () => {
    if (!CREDIT_URL) {
      setDown("NEXT_PUBLIC_CREDIT_URL unset");
      return;
    }
    try {
      const r = await fetch(`${CREDIT_URL}/v0/credit/loans`, { cache: "no-store" });
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      const j = (await r.json()) as { loans: Loan[]; events: DeskEvent[] };
      setLoans(j.loans);
      setEvents(j.events);
      setDown("");
    } catch (e) {
      setDown(e instanceof Error ? e.message : String(e));
    }
  }, []);

  // One-off chain tip from the desk's boot-checked /v0/health; SSE ticks keep it current.
  const loadTip = useCallback(async () => {
    if (!CREDIT_URL) return;
    try {
      const r = await fetch(`${CREDIT_URL}/v0/health`, { cache: "no-store" });
      if (!r.ok) return;
      const j = (await r.json()) as { chain?: { Ok?: { tipHeight?: number } } };
      const h = j.chain?.Ok?.tipHeight;
      if (typeof h === "number") {
        setTip((prev) => (prev ? prev : { height: h, source: "/v0/health", at: Date.now() }));
      }
    } catch {
      // the loans fetch reports DOWN; the tip simply stays unknown
    }
  }, []);

  useEffect(() => {
    refresh();
    loadTip();
    if (!CREDIT_URL) return;
    const es = new EventSource(`${CREDIT_URL}/v0/events`);
    es.onopen = () => {
      setLive(true);
      // catch anything emitted while (re)connecting
      refresh();
    };
    es.onerror = () => setLive(false);
    const onEvent = (msg: MessageEvent) => {
      let ev: DeskEvent;
      try {
        ev = JSON.parse(msg.data as string) as DeskEvent;
      } catch {
        return;
      }
      if (ev.kind === "tick") {
        if (typeof ev.height === "number") setTip({ height: ev.height, source: "underwriter tick", at: Date.now() });
        return;
      }
      setEvents((prev) => [ev, ...prev.filter((p) => p.id !== ev.id)].slice(0, 50));
      refresh();
    };
    for (const k of SSE_KINDS) es.addEventListener(k, onEvent as EventListener);
    return () => es.close();
  }, [refresh, loadTip]);

  const counts = (loans ?? []).reduce<Record<string, number>>((acc, l) => {
    acc[l.status] = (acc[l.status] ?? 0) + 1;
    return acc;
  }, {});

  return (
    <>
      <h1>
        credit <span>desk</span>
      </h1>
      <div className="card">
        <dl className="kv">
          <dt>desk</dt>
          <dd>
            {down ? (
              <span className="status-down">DOWN · {down}</span>
            ) : loans === null ? (
              <span>connecting…</span>
            ) : (
              <span className="status-ok">{CREDIT_URL}</span>
            )}
          </dd>
          <dt>live stream (SSE)</dt>
          <dd>{live ? <span className="status-ok">connected</span> : <span className="status-down">not connected</span>}</dd>
          <dt>chain tip</dt>
          <dd>
            {tip ? (
              <>
                <strong>{tip.height}</strong> <small>({tip.source}, {new Date(tip.at).toLocaleTimeString()})</small>
              </>
            ) : (
              "waiting for the desk"
            )}
          </dd>
          <dt>book</dt>
          <dd>
            {loans === null
              ? "—"
              : (["open", "at-risk", "repaid", "defaulted"] as const).map((s) => (
                  <span key={s} style={{ marginRight: 8 }}>
                    <LoanBadge status={s} /> {counts[s] ?? 0}
                  </span>
                ))}
          </dd>
        </dl>
        <p className="footnote" style={{ marginTop: 12 }}>
          A borrower agent proves live solvency (zsol attestation, challenge-response), the desk
          buys the product through Bitrefill (test products in test mode) and waits for a shielded
          repayment carrying the loan memo. If the borrower spends its anchored notes, the loan turns
          at-risk within one block.
        </p>
      </div>

      <div className="card">
        <h2>loans</h2>
        {loans === null ? (
          <p>{down ? "desk unreachable" : "loading…"}</p>
        ) : loans.length === 0 ? (
          <p>
            No loans yet. Ask a borrower agent to call <code>credit_request</code>.
          </p>
        ) : (
          <div className="table-wrap">
            <table className="entries">
              <thead>
                <tr>
                  <th>status</th>
                  <th>loan · agent</th>
                  <th>Bitrefill</th>
                  <th>principal</th>
                  <th>anchor tx</th>
                  <th>repayment tx</th>
                  <th>heights</th>
                </tr>
              </thead>
              <tbody>
                {loans.map((l) => {
                  const left = tip && (l.status === "open" || l.status === "at-risk") ? l.dueHeight - tip.height : null;
                  return (
                    <tr key={l.id} className={`row-${l.status}`}>
                      <td>
                        <LoanBadge status={l.status} />
                      </td>
                      <td>
                        <code title={l.id}>{shortHex(l.id, 8, 4)}</code>
                        <br />
                        <strong>{l.agentId}</strong>
                      </td>
                      <td>
                        {l.productId} · ${l.valueUsd}
                        <br />
                        <small>invoice</small> <Copyable value={l.bitrefillInvoiceId} full />
                        {l.bitrefillOrderId && (
                          <>
                            <br />
                            <small>order</small> <Copyable value={l.bitrefillOrderId} full />
                          </>
                        )}
                      </td>
                      <td>
                        {fmtTaz(l.principalZat)}
                        <br />
                        <small>
                          @ ${l.usdPerZec.toFixed(2)}/ZEC ({l.rateSource})
                        </small>
                      </td>
                      <td>
                        <Copyable value={l.anchorTxid} tx />
                      </td>
                      <td>
                        <Copyable value={l.repaidTxid} tx />
                      </td>
                      <td>
                        opened {l.openedHeight}
                        <br />
                        due {l.dueHeight}
                        {left !== null && (
                          <>
                            <br />
                            <small>{left >= 0 ? `${left} blocks left` : `${-left} blocks overdue`}</small>
                          </>
                        )}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </div>

      <div className="card">
        <h2>events</h2>
        {events.length === 0 ? (
          <p>none yet</p>
        ) : (
          events.map((e) => (
            <div key={`${e.id}-${e.at}`} className={`check-row ${eventTone(e)}`}>
              <span className="check-name">{e.kind}</span>
              <span style={{ flex: 1 }}>
                <small>
                  {e.height ? `h=${e.height} · ` : ""}
                  {new Date(e.at * 1000).toLocaleTimeString()}
                  {e.loanId ? ` · loan ${shortHex(e.loanId, 8, 4)}` : ""}
                </small>
                <br />
                <EventDetail detail={e.detail} />
              </span>
            </div>
          ))
        )}
      </div>

      <VerifyQr />
    </>
  );
}
