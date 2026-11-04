// Relays tools/call zsol_challenge to the verifier MCP gateway (NEXT_PUBLIC_MCP_URL).
import { mcpZsolChallenge, unwrapTool } from "@/lib/endpoints";

export const dynamic = "force-dynamic";

export async function POST() {
  const { ok, value } = unwrapTool(await mcpZsolChallenge());
  return Response.json(value, { status: ok ? 200 : 502 });
}
