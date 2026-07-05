// Shared types mirroring the Rust core's JSON shapes (na-tools / na-host).

export interface ToolSpec {
  name: string;
  description: string;
  input_schema: unknown;
  capabilities: string[];
  mutating: boolean;
}

export interface ResultMeta {
  bytes: number;
  truncated: boolean;
  was_binary: boolean;
  redactions: number;
  untrusted: boolean;
  duration_ms: number;
}

export interface ToolResult {
  ok: boolean;
  content: string;
  data: unknown;
  summary?: string | null;
  metadata: ResultMeta;
}

export interface RpcError {
  code: number;
  message: string;
  data?: unknown;
}

export interface RpcResponse {
  jsonrpc: string;
  id: unknown;
  result?: unknown;
  error?: RpcError;
}
