// Browser-side helper: call the core via our /api/rpc proxy.

import type { RpcResponse, ToolResult, ToolSpec } from "./types";

const MAX_ERROR_BODY_LENGTH = 500;

function responseLabel(res: Response): string {
  return `${res.status}${res.statusText ? ` ${res.statusText}` : ""}`;
}

function bodyExcerpt(body: string): string {
  const normalized = body.trim().replace(/\s+/g, " ");
  if (!normalized) return "empty response body";
  return normalized.length > MAX_ERROR_BODY_LENGTH
    ? `${normalized.slice(0, MAX_ERROR_BODY_LENGTH)}...`
    : normalized;
}

/** Send one RPC request and return the full JSON-RPC envelope. */
export async function rpcResponse(method: string, params: unknown = {}): Promise<RpcResponse> {
  const res = await fetch("/api/rpc", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ method, params }),
  });

  const body = await res.text();
  let json: unknown;
  try {
    json = JSON.parse(body);
  } catch {
    const prefix = res.ok ? "RPC returned invalid JSON" : "RPC HTTP error";
    throw new Error(`${prefix} (${responseLabel(res)}): ${bodyExcerpt(body)}`);
  }

  if (typeof json !== "object" || json === null || Array.isArray(json)) {
    throw new Error(`RPC returned an invalid response (${responseLabel(res)})`);
  }

  const response = json as RpcResponse;
  if (!("result" in response) && !("error" in response)) {
    throw new Error(`RPC response has neither result nor error (${responseLabel(res)})`);
  }
  if (!res.ok && !response.error) {
    throw new Error(`RPC HTTP error (${responseLabel(res)}): ${bodyExcerpt(body)}`);
  }
  return response;
}

/**
 * Low-level: send a method + params, return the unwrapped `result` (throws on
 * an RPC error or HTTP failure).
 */
export async function rpc<T = unknown>(method: string, params: unknown = {}): Promise<T> {
  const json = await rpcResponse(method, params);
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
export function invokeTool<TData = unknown>(
  name: string,
  args: unknown = {},
): Promise<ToolResult<TData>> {
  return rpc<ToolResult<TData>>("invoke_tool", { name, args });
}
