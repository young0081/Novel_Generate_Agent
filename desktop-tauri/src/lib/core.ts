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

/** Run a scripted goal (offline for now). */
export async function runGoal(args: RunGoalArgs): Promise<GoalRun> {
  return invoke<GoalRun>("run_goal", args as unknown as Record<string, unknown>);
}

/** Request cancellation of the current run. */
export async function cancel(): Promise<void> {
  return invoke<void>("cancel");
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
