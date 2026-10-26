// Server-side fetchers for REAL endpoints only (FR-6.3: no mock/fixture data).
// Registry: GET /v0/health, GET /v0/attestations/:agentId  (SPEC §7)
// MCP gateway: POST /mcp JSON-RPC 2.0 (tools/list, tools/call zsol_health,
// zsol_verify). When a service is unreachable the caller gets the explicit
// `down` shape and the UI renders DOWN — never placeholder data.

import { blake2b } from "@noble/hashes/blake2b";

const REGISTRY_URL =
  process.env.NEXT_PUBLIC_REGISTRY_URL?.replace(/\/$/, "") ?? "";
const MCP_URL = process.env.NEXT_PUBLIC_MCP_URL?.replace(/\/$/, "") ?? "";

export type Down = { down: true; detail: string };

export type Health =
  | Down
  | { down?: false; ok: boolean; chainLength: number; watermarkHeight: number };

export async function registryHealth(): Promise<Health> {
  if (!REGISTRY_URL) return { down: true, detail: "NEXT_PUBLIC_REGISTRY_URL unset" };
  try {
    const res = await fetch(`${REGISTRY_URL}/v0/health`, { cache: "no-store" });
    if (!res.ok) return { down: true, detail: `HTTP ${res.status}` };
    const j = (await res.json()) as {
      ok?: boolean;
      chainLength?: number;
      watermarkHeight?: number;
    };
    return {
      ok: j.ok === true,
      chainLength: j.chainLength ?? -1,
      watermarkHeight: j.watermarkHeight ?? 0,
    };
  } catch (e) {
    return { down: true, detail: e instanceof Error ? e.name : "fetch error" };
  }
}

export type Entry = {
  seq: number;
  entryId: string;
  chainDigest: string;
  object: string; // b64u of canonical json (SPEC §4/§7)
};

export type FetchResult =
  | Down
  | {
      down?: false;
      agentId: string;
      entries: Entry[];
      chainOk: boolean;
      chainError?: string;
    };

export async function registryAgent(id: string): Promise<FetchResult> {
  if (!REGISTRY_URL) return { down: true, detail: "NEXT_PUBLIC_REGISTRY_URL unset" };
  try {
    const res = await fetch(`${REGISTRY_URL}/v0/attestations/${encodeURIComponent(id)}`, {
      cache: "no-store",
    });
    if (res.status === 404) return { down: false, agentId: id, entries: [], chainOk: true };
    if (!res.ok) return { down: true, detail: `HTTP ${res.status}` };
    const entries = (await res.json()) as Entry[];
    return { down: false, agentId: id, entries, ...recheckChain(entries) };
  } catch (e) {
    return { down: true, detail: e instanceof Error ? e.name : "fetch error" };
  }
}

const GENESIS_HEX = "0".repeat(64);

// chainDigestₙ = BLAKE2b-256(chainDigestₙ₋₁ ‖ canonical(objectₙ)) — SPEC §7.
// The wire carries b64u(canonical json); the chain recompute re-canonicalizes
// the parsed object (sorted keys, compact) so byte-exact digests match Rust.
export function recheckChain(entries: Entry[]): { chainOk: boolean; chainError?: string } {
  let prev = GENESIS_HEX;
  try {
    for (const e of entries) {
      const obj = JSON.parse(fromB64u(e.object)) as Record<string, unknown>;
      const canon = canonicalJson(obj);
      const d = blake2b256(hexToBytes(prev)!, new TextEncoder().encode(canon));
      const hex = bytesToHex(d);
      if (hex !== e.chainDigest.toLowerCase()) {
        return {
          chainOk: false,
          chainError: `seq ${e.seq}: recomputed digest ≠ server chainDigest`,
        };
      }
      if (hex !== e.entryId.toLowerCase()) {
        return { chainOk: false, chainError: `seq ${e.seq}: entryId ≠ hex(chainDigest)` };
      }
      prev = hex;
    }
    return { chainOk: true };
  } catch (e) {
    return { chainOk: false, chainError: `decode/parse: ${e instanceof Error ? e.message : "?"}` };
  }
}

function canonicalJson(v: unknown): string {
  if (v === null || typeof v !== "object") return JSON.stringify(v);
  if (Array.isArray(v)) {
    return `[${v.map((x) => canonicalJson(x)).join(",")}]`;
  }
  const keys = Object.keys(v as Record<string, unknown>).sort();
  const rec = v as Record<string, unknown>;
  return `{${keys.map((k) => `${JSON.stringify(k)}:${canonicalJson(rec[k])}`).join(",")}}`;
}

function blake2b256(a: Uint8Array, b: Uint8Array): Uint8Array {
  const joined = new Uint8Array(a.length + b.length);
  joined.set(a, 0);
  joined.set(b, a.length);
  return blake2b(joined, { dkLen: 32 });
}

function hexToBytes(hex: string): Uint8Array {
  return new Uint8Array(hex.match(/.{1,2}/g)!.map((h) => parseInt(h, 16)));
}

function bytesToHex(b: Uint8Array): string {
  return Array.from(b)
    .map((x) => x.toString(16).padStart(2, "0"))
    .join("");
}

function fromB64u(s: string): string {
  const b64 = s.replace(/-/g, "+").replace(/_/g, "/");
  const padded = b64 + "=".repeat((4 - (b64.length % 4)) % 4);
  const bin = atob(padded);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return new TextDecoder().decode(bytes);
}

// ---- MCP over JSON-RPC 2.0 (streamable HTTP, SPEC §8) -------------------

export type McpTools =
  | Down
  | { down?: false; tools: string[] };

export type McpToolCall =
  | Down
  | { down?: false; result: unknown };

export async function mcpToolsList(): Promise<McpTools> {
  if (!MCP_URL) return { down: true, detail: "NEXT_PUBLIC_MCP_URL unset" };
  const r = await mcpRpc("tools/list", {});
  if ("down" in r) return r as Down;
  const tools = ((r.result as { tools?: { name: string }[] })?.tools ?? []).map(
    (t) => t.name,
  );
  return { down: false, tools };
}

export async function mcpZsolHealth(): Promise<McpToolCall> {
  return callTool("zsol_health", {});
}

export async function mcpZsolVerify(objB64u: string): Promise<McpToolCall> {
  return callTool("zsol_verify", { object: objB64u });
}

async function callTool(name: string, args: Record<string, unknown>): Promise<McpToolCall> {
  if (!MCP_URL) return { down: true, detail: "NEXT_PUBLIC_MCP_URL unset" };
  const r = await mcpRpc("tools/call", { name, arguments: args });
  if ("down" in r) return r as Down;
  return { down: false, result: r.result };
}

async function mcpRpc(
  method: string,
  params: unknown,
): Promise<{ result: unknown } | Down> {
  try {
    const res = await fetch(`${MCP_URL}/mcp`, {
      method: "POST",
      headers: { "content-type": "application/json", accept: "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: "zsol-web", method, params }),
      cache: "no-store",
    });
    if (!res.ok) return { down: true, detail: `HTTP ${res.status}` };
    const j = (await res.json()) as {
      error?: { code: number; message: string };
      result?: unknown;
    };
    if (j.error) return { down: true, detail: `${j.error.code}: ${j.error.message}` };
    return { result: j.result ?? null };
  } catch (e) {
    return { down: true, detail: e instanceof Error ? e.name : "fetch error" };
  }
}
