// Client-safe display helpers (no fetching, no env). Amounts on the wire are zatoshi.

export function fmtTaz(zat: number | null | undefined): string {
  if (typeof zat !== "number" || !Number.isFinite(zat)) return "—";
  return `${(zat / 1e8).toFixed(8).replace(/0+$/, "").replace(/\.$/, "")} TAZ`;
}

export function shortHex(id: string, head = 10, tail = 6): string {
  return id.length > head + tail + 1 ? `${id.slice(0, head)}…${id.slice(-tail)}` : id;
}
