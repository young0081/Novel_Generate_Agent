// Typed core client wrapping the Tauri command bridge.
//
// Every call degrades gracefully when running outside the desktop shell
// (e.g. a plain browser `vite preview`): instead of throwing an opaque
// "invoke is not defined" error, we surface a friendly Chinese message so
// the build and a browser preview never hard-fail.

import { invoke as tauriInvoke } from "@tauri-apps/api/core";

/** A single tool result envelope returned by the Rust core. */
export interface ToolMetadata {
  bytes: number;
  truncated: boolean;
  was_binary: boolean;
  redactions: number;
  untrusted: boolean;
  duration_ms: number;
}

export interface ToolResult<T = unknown> {
  ok: boolean;
  content: string;
  data: T;
  summary?: string | null;
  metadata: ToolMetadata;
}

/**
 * Convert a structured tool failure into a regular exception.
 *
 * The bridge deliberately resolves tool-level failures as `{ ok: false }`.
 * Mutation callers must opt in to this guard before updating optimistic UI.
 */
export function requireToolSuccess<T>(
  result: ToolResult<T>,
  fallback = "工具执行失败",
): ToolResult<T> {
  if (!result.ok) {
    throw new Error(result.content.trim() || result.summary?.trim() || fallback);
  }
  return result;
}

/** Unique id used to isolate concurrent Tauri event streams. */
export function newRequestId(prefix = "request"): string {
  const cryptoApi = globalThis.crypto;
  if (cryptoApi && typeof cryptoApi.randomUUID === "function") {
    return `${prefix}-${cryptoApi.randomUUID()}`;
  }
  return `${prefix}-${Date.now().toString(36)}-${Math.random()
    .toString(36)
    .slice(2, 10)}`;
}

/** Description of a tool exposed by the Rust core. */
export interface ToolSpec {
  name: string;
  description: string;
  input_schema: unknown;
  capabilities: string[];
  mutating: boolean;
}

/** Result of a scripted goal run. */
export interface GoalRun {
  outcome: unknown;
  session: unknown;
}

export interface RunGoalArgs {
  goal: string;
  title: string;
  protocol?: string;
  responses: string[];
}

/** Thrown when a Tauri command is unavailable (running in a browser). */
export class NotInDesktopError extends Error {
  constructor() {
    super("需在桌面应用中运行（当前环境无法连接本地核心）");
    this.name = "NotInDesktopError";
  }
}

/** True when the Tauri runtime bridge is present. */
export function isDesktop(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof (window as unknown as { __TAURI_INTERNALS__?: unknown })
      .__TAURI_INTERNALS__ !== "undefined"
  );
}

async function invoke<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (!isDesktop()) {
    throw new NotInDesktopError();
  }
  return (await tauriInvoke(cmd, args)) as T;
}

/** Liveness probe. Returns the literal "pong" when the core is reachable. */
export async function ping(): Promise<string> {
  return invoke<string>("ping");
}

/** List every tool the core exposes. */
export async function listTools(): Promise<ToolSpec[]> {
  return invoke<ToolSpec[]>("list_tools");
}

/**
 * Invoke a tool by name. This is the workhorse: the core never throws for
 * tool-level failures — those come back as `{ ok: false, ... }`. We still
 * guard the transport (browser fallback) above.
 */
export async function invokeTool<T = unknown>(
  name: string,
  args: Record<string, unknown>,
): Promise<ToolResult<T>> {
  return invoke<ToolResult<T>>("invoke_tool", { name, args });
}

/** Conditionally create/overwrite a workspace text file in one backend lease. */
export async function createWorkspaceFile(
  path: string,
  content: string,
  overwrite = false,
): Promise<void> {
  return invoke<void>("workspace_create_file", { path, content, overwrite });
}

/** Rename a workspace file without exposing a read/write/delete race window. */
export async function renameWorkspaceFile(
  oldPath: string,
  newPath: string,
  overwrite = false,
): Promise<void> {
  return invoke<void>("workspace_rename_file", { oldPath, newPath, overwrite });
}

export function isWorkspaceTargetExistsError(error: unknown): boolean {
  return describeError(error).includes("WORKSPACE_TARGET_EXISTS");
}

/** Run a scripted goal (offline for now). */
export async function runGoal(args: RunGoalArgs): Promise<GoalRun> {
  return invoke<GoalRun>("run_goal", args as unknown as Record<string, unknown>);
}

/** Cancel one streamed request; omit the id only for legacy cancel-all callers. */
export async function cancel(requestId?: string): Promise<void> {
  return invoke<void>("cancel", { requestId });
}

/**
 * Normalise any thrown value into a readable Chinese string for the UI.
 */
export function describeError(err: unknown): string {
  if (err instanceof NotInDesktopError) return err.message;
  if (err instanceof Error) return err.message;
  if (typeof err === "string") return err;
  try {
    return JSON.stringify(err);
  } catch {
    return "未知错误";
  }
}
