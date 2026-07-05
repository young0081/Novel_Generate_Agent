// JSON-RPC proxy: browser -> this route -> Rust core (na-host).

import { NextResponse } from "next/server";

import { coreRpc } from "@/lib/coreClient";

// The bridge uses Node's child_process, so this must run on the Node runtime.
export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(req: Request) {
  let body: { method?: unknown; params?: unknown };
  try {
    body = await req.json();
  } catch {
    return NextResponse.json(
      { error: { code: -32700, message: "invalid JSON body" } },
      { status: 400 },
    );
  }

  const method = body?.method;
  if (typeof method !== "string" || method.length === 0) {
    return NextResponse.json(
      { error: { code: -32600, message: 'request must include a string "method"' } },
      { status: 400 },
    );
  }
  const params = body?.params ?? {};

  try {
    const response = await coreRpc(method, params);
    return NextResponse.json(response);
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    return NextResponse.json(
      { jsonrpc: "2.0", id: null, error: { code: -32000, message } },
      { status: 502 },
    );
  }
}
