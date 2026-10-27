// Minimal decoder for the attestation object payload (b64u canonical json).
// Display-only: the registry wire types (SPEC §4) — claim-only summary fields.

export type ObjectSummary = {
  protocol?: string;
  network?: string;
  pool?: string;
  agentId?: string;
  claimZat?: number;
  anchorHeight?: number;
  anchorTxid?: string;
  issuedAt?: number;
  maxAgeHours?: number;
};

export function decodeObjectSummary(obj: unknown): ObjectSummary | null {
  if (obj && typeof obj === "object") return obj as ObjectSummary;
  if (typeof obj !== "string") return null;
  const b64u = obj;
  try {
    const json = atob(
      b64u.replace(/-/g, "+").replace(/_/g, "/") +
        "=".repeat((4 - (b64u.replace(/-/g, "+").replace(/_/g, "/").length % 4)) % 4),
    );
    const o = JSON.parse(json) as ObjectSummary;
    return o;
  } catch {
    return null;
  }
}
