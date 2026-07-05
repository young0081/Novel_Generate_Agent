// Browser-side helper: call the core via our /api/rpc proxy.

import type { RpcResponse, ToolResult, ToolSpec } from "./types";

/**
 * Low-level: send a method + params, return the unwrapped `result` (throws on
 * an RPC error or HTTP failure).
 */
export async function rpc<T = unknown>(method: string, params: unknown = {}): Promise<T> {
  const res = await fetch("/api/rpc", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ method, params }),
  });
  const json = (await res.json()) as RpcResponse;
  if (json.error) {
    throw new Error(`[${json.error.code}] ${json.error.message}`);
  }
  return json.result as T;
}

/** Convenience: list the core's tool catalog. */
export function listTools(): Promise<ToolSpec[]> {
  return rpc<ToolSpec[]>("list_tools");
}

/** Convenience: invoke a tool by name with its args object. */
export function invokeTool(name: string, args: unknown = {}): Promise<ToolResult> {
  return rpc<ToolResult>(name, args);
}
