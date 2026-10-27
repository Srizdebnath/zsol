// Relays tools/call zsol_verify to the verifier MCP gateway. The gateway runs C1–C8 in its
// own view-only wallet against Zcash testnet; this route only forwards and unwraps.
import { mcpZsolVerify, unwrapTool } from "@/lib/endpoints";

export const dynamic = "force-dynamic";

export async function POST(req: Request) {
  let presentation: unknown;
  try {
    const body = (await req.json()) as { presentation?: unknown };
    presentation = body.presentation;
  } catch {
    return Response.json({ error: "json body expected" }, { status: 400 });
  }
  if (!presentation || typeof presentation !== "object") {
    return Response.json({ error: "presentation (object) required" }, { status: 400 });
  }
  const { ok, value } = unwrapTool(await mcpZsolVerify(presentation));
  return Response.json(value, { status: ok ? 200 : 502 });
}
