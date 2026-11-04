// Server-side fetchers for REAL endpoints only (FR-6.3: no mock/fixture data).
// Registry: GET /v0/health, GET /v0/attestations/:agentId  (SPEC §7)
// MCP gateway: POST /mcp JSON-RPC 2.0 (tools/list, tools/call zsol_health,
// zsol_verify). When a service is unreachable the caller gets the explicit
// `down` shape and the UI renders DOWN — never placeholder data.

import { blake2b } from "@noble/hashes/blake2b";

const REGISTRY_URL =
  process.env.NEXT_PUBLIC_REGISTRY_URL?.replace(/\/$/, "") ?? "";
const MCP_URL = process.env.NEXT_PUBLIC_MCP_URL?.replace(/\/$/, "") ?? "";
const CREDIT_URL = process.env.NEXT_PUBLIC_CREDIT_URL?.replace(/\/$/, "") ?? "";

/** NU7 consensus branch id as zsol's boot check formats it (lowercase hex, no 0x). */
export const NU7_BRANCH = "77190ad9";

/** Health probes must not hang the status page; long-running tools (zsol_verify) have none. */
const HEALTH_TIMEOUT_MS = 20_000;

export type Down = { down: true; detail: string };

export type Health =
  | Down
  | { down?: false; ok: boolean; chainLength: number; watermarkHeight: number };

export async function registryHealth(): Promise<Health> {
  if (!REGISTRY_URL) return { down: true, detail: "NEXT_PUBLIC_REGISTRY_URL unset" };
  try {
    const res = await fetch(`${REGISTRY_URL}/v0/health`, {
      cache: "no-store",
      signal: AbortSignal.timeout(HEALTH_TIMEOUT_MS),
    });
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
    return { down: true, detail: e instanceof Error ? `${e.name}: ${e.message}` : "fetch error" };
  }
}

export type Entry = {
  seq: number;
  entryId: string;
  chainDigest: string;
  /** Digest of the globally previous entry; null for the registry's first entry (SPEC §9). */
  prevDigest?: string | null;
  object: unknown; // parsed object (current wire) or b64u canonical json
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
    const payload = (await res.json()) as
      | Entry[]
      | { entries?: Entry[]; agentId?: string };
    // wire shape tolerance: zsol-registry replies {agentId, chainHead, entries};
    // a bare array is accepted for SPEC §7 compat.
    const entries: Entry[] = Array.isArray(payload)
      ? payload
      : Array.isArray(payload?.entries)
        ? payload.entries
        : [];
    return { down: false, agentId: id, entries, ...recheckChain(entries) };
  } catch (e) {
    return { down: true, detail: e instanceof Error ? `${e.name}: ${e.message}` : "fetch error" };
  }
}

const GENESIS_HEX = "0".repeat(64);

// chainDigestₙ = BLAKE2b-256(chainDigestₙ₋₁ ‖ canonical(objectₙ)) — SPEC §9.
// The chain is GLOBAL across agents, so each entry's predecessor is its prevDigest
// (null = genesis), not the previous entry of this agent. Objects are re-canonicalized
// (sorted keys, compact) so the digests match Rust byte for byte.
export function recheckChain(entries: Entry[]): { chainOk: boolean; chainError?: string } {
  try {
    for (const e of entries) {
      const prev = e.prevDigest ? e.prevDigest.toLowerCase() : GENESIS_HEX;
      // The registry may hand the object as b64u(canonical json) or already
      // decoded — re-canonicalize whichever shape it is (sorted keys, compact).
      const obj =
        typeof e.object === "string"
          ? (JSON.parse(fromB64u(e.object)) as Record<string, unknown>)
          : (e.object as Record<string, unknown>);
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
  const r = await mcpRpc("tools/list", {}, HEALTH_TIMEOUT_MS);
  if ("down" in r) return r as Down;
  const tools = ((r.result as { tools?: { name: string }[] })?.tools ?? []).map(
    (t) => t.name,
  );
  return { down: false, tools };
}

export async function mcpZsolHealth(): Promise<McpToolCall> {
  return callTool("zsol_health", {}, HEALTH_TIMEOUT_MS);
}

export async function mcpZsolVerify(presentation: unknown): Promise<McpToolCall> {
  return callTool("zsol_verify", { presentation });
}

export async function mcpZsolChallenge(): Promise<McpToolCall> {
  return callTool("zsol_challenge", {});
}

/** Unwraps an MCP tools/call envelope {content:[{text}], isError} into its JSON payload. */
export function unwrapTool(r: McpToolCall): { ok: boolean; value: unknown } {
  if ("down" in r && r.down) return { ok: false, value: { error: "gateway down", detail: r.detail } };
  const env = (r as { result?: { content?: { text?: string }[]; isError?: boolean } }).result;
  const text = env?.content?.[0]?.text;
  let value: unknown = env;
  if (typeof text === "string") {
    try {
      value = JSON.parse(text);
    } catch {
      value = { error: "unparseable tool reply", raw: text };
    }
  }
  return { ok: env?.isError !== true, value };
}

async function callTool(
  name: string,
  args: Record<string, unknown>,
  timeoutMs?: number,
): Promise<McpToolCall> {
  if (!MCP_URL) return { down: true, detail: "NEXT_PUBLIC_MCP_URL unset" };
  const r = await mcpRpc("tools/call", { name, arguments: args }, timeoutMs);
  if ("down" in r) return r as Down;
  return { down: false, result: r.result };
}

async function mcpRpc(
  method: string,
  params: unknown,
  timeoutMs?: number,
): Promise<{ result: unknown } | Down> {
  try {
    const res = await fetch(`${MCP_URL}/mcp`, {
      method: "POST",
      headers: { "content-type": "application/json", accept: "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: "zsol-web", method, params }),
      cache: "no-store",
      signal: timeoutMs ? AbortSignal.timeout(timeoutMs) : undefined,
    });
    if (!res.ok) return { down: true, detail: `HTTP ${res.status}` };
    const j = (await res.json()) as {
      error?: { code: number; message: string };
      result?: unknown;
    };
    if (j.error) return { down: true, detail: `${j.error.code}: ${j.error.message}` };
    return { result: j.result ?? null };
  } catch (e) {
    return { down: true, detail: e instanceof Error ? `${e.name}: ${e.message}` : "fetch error" };
  }
}

// ---- shared wire shapes ---------------------------------------------------

/** serde_json encodes a Rust `Result<T, String>` externally tagged: {"Ok": T} | {"Err": "…"}. */
export type RustResult<T> = { Ok: T } | { Err: string };

export function rustResult<T>(r: RustResult<T> | null | undefined): { ok: true; value: T } | { ok: false; error: string } {
  if (r && typeof r === "object" && "Ok" in r) return { ok: true, value: r.Ok };
  if (r && typeof r === "object" && "Err" in r) return { ok: false, error: String(r.Err) };
  return { ok: false, error: "missing from reply" };
}

/** zsol-lightwallet chain::ChainInfo (boot-checked: branch matches local consensus at tip). */
export type ChainInfo = {
  endpoint: string;
  chainName: string;
  consensusBranchId: string;
  tipHeight: number;
  vendor: string;
  version: string;
};

/** zsol-mcp ops::health payload (tools/call zsol_health). */
export type Probe = string | { url: string; status?: number; error?: string };
export type ZsolHealth = {
  chain: RustResult<ChainInfo>;
  wallet: { dir: string; exists: boolean };
  registry: Probe;
  creditDesk: Probe;
  spend: { allowed: boolean; capZat: number };
};

export async function mcpZsolHealthParsed(): Promise<Down | { down?: false; health: ZsolHealth }> {
  const raw = await mcpZsolHealth();
  if ("down" in raw && raw.down) return raw;
  const { ok, value } = unwrapTool(raw);
  if (!ok) {
    const v = value as { reason?: unknown; detail?: unknown };
    return { down: true, detail: `zsol_health failed: ${String(v.reason ?? "")} ${String(v.detail ?? "")}`.trim() };
  }
  return { down: false, health: value as ZsolHealth };
}

// ---- Credit Desk (zsol-credit, SPEC §11) ----------------------------------

export type CreditPolicy = {
  version: string;
  underwriterId: string;
  mode: string;
  ltvBps: number;
  maxLoanUsd: number;
  maxTenorBlocks: number;
  minConf: number;
  rateMaxAgeSecs: number;
  allowedProducts: string[];
};

/** zsol-lightwallet wallet::Balance. */
export type Balance = { account: string; totalZat: number; spendableZat: number; pendingZat: number };

/** GET /v0/health on zsol-credit. */
export type CreditHealth = {
  ok: boolean;
  chain: RustResult<ChainInfo>;
  treasury: Balance | null;
  bitrefill: boolean;
  bitrefillBalance: RustResult<unknown>;
  policy: CreditPolicy;
};

export async function creditHealth(): Promise<Down | { down?: false; health: CreditHealth }> {
  if (!CREDIT_URL) return { down: true, detail: "NEXT_PUBLIC_CREDIT_URL unset" };
  try {
    const res = await fetch(`${CREDIT_URL}/v0/health`, {
      cache: "no-store",
      signal: AbortSignal.timeout(HEALTH_TIMEOUT_MS),
    });
    if (!res.ok) return { down: true, detail: `HTTP ${res.status}` };
    return { down: false, health: (await res.json()) as CreditHealth };
  } catch (e) {
    return { down: true, detail: e instanceof Error ? `${e.name}: ${e.message}` : "fetch error" };
  }
}
