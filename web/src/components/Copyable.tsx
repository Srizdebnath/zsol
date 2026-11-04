"use client";

// Copyable monospace identifier (txids, invoice ids, nonces). No public block explorer serves
// post-NU7 testnet, so a txid is a link ONLY when NEXT_PUBLIC_EXPLORER_TX is configured.
import { useState } from "react";
import { shortHex } from "@/lib/format";

const EXPLORER_TX = process.env.NEXT_PUBLIC_EXPLORER_TX ?? "";

export function Copyable({
  value,
  tx = false,
  full = false,
}: {
  value: string | null | undefined;
  /** Treat as a Zcash txid: link to NEXT_PUBLIC_EXPLORER_TX when set. */
  tx?: boolean;
  /** Render the whole value instead of a shortened head…tail. */
  full?: boolean;
}) {
  const [copied, setCopied] = useState<"" | "ok" | "fail">("");
  if (!value) return <span>—</span>;
  const shown = full ? value : shortHex(value);

  async function copy() {
    try {
      await navigator.clipboard.writeText(value!);
      setCopied("ok");
    } catch {
      // navigator.clipboard needs a secure context; plain-http LAN demos fall back here.
      const ta = document.createElement("textarea");
      ta.value = value!;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      const ok = document.execCommand("copy");
      document.body.removeChild(ta);
      setCopied(ok ? "ok" : "fail");
    }
    setTimeout(() => setCopied(""), 1500);
  }

  return (
    <span className="copyable">
      {tx && EXPLORER_TX ? (
        <a href={`${EXPLORER_TX}${value}`} target="_blank" rel="noreferrer">
          <code title={value}>{shown}</code>
        </a>
      ) : (
        <code title={value}>{shown}</code>
      )}
      <button type="button" className="copy-btn" onClick={copy} aria-label={`copy ${value}`}>
        {copied === "ok" ? "copied" : copied === "fail" ? "select" : "copy"}
      </button>
    </span>
  );
}
