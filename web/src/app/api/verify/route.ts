// T-48 relay: the browser never talks to the LAN gateway directly; this server
// route forwards tools/call zsol_verify to NEXT_PUBLIC_MCP_URL (regtest/LAN).
// It is a pure relay — no data is synthesized here (FR-6.4).
import { mcpZsolVerify } from "@/lib/endpoints";

export const dynamic = "force-dynamic";

export async function POST(req: Request) {
  let object = "";
  try {
    const body = (await req.json()) as { object?: unknown };
    if (typeof body.object !== "string" || body.object.length === 0) {
      return Response.json({ error: "object (b64u) required" }, { status: 400 });
    }
    object = body.object;
  } catch {
    return Response.json({ error: "json body expected" }, { status: 400 });
  }

  const r = await mcpZsolVerify(object);
  if ("down" in r && r.down) {
    return Response.json({ error: "gateway down", detail: r.detail }, { status: 502 });
  }
  return Response.json(r.result);
}
