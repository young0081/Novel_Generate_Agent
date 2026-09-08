// Typed wrapper for the live AI writing loop (创作 / Writing Studio).
//
// Mirrors the browser-safe guarding in `core.ts`: when running outside the
// desktop shell (a plain `vite preview`), every entry point throws a friendly
// Chinese NotInDesktopError instead of an opaque "invoke is not defined".
//
// The real agent loop runs in the Rust core via `run_goal_live`, streaming
// progress over the "agent-step" Tauri event. The contract here:
//   - set up the listener BEFORE invoking
//   - always unlisten in a finally (even when invoke throws)

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { isDesktop, newRequestId, NotInDesktopError } from "./core";
import type { SamplingParams } from "./providers";

/** A single chat message in a session transcript. */
export interface Message {
  role: "system" | "user" | "assistant" | "tool";
  content: string;
  tool_call?: {
    id: string;
    name: string;
    args: unknown;
  } | null;
  tool_result?: {
    call_id: string;
    name: string;
    ok: boolean;
    untrusted: boolean;
  } | null;
}

/** The session returned (and streamed) by the agent loop. */
export interface Session {
  id: string;
  title: string;
  messages: Message[];
  state: unknown;
  created_ms: number;
  updated_ms: number;
}

/** The envelope returned by `run_goal_live`. */
export interface LiveRun {
  outcome: {
    steps: number;
    final_answer: string | null;
    stopped_reason: string;
    auto_saved_path?: string;
    auto_save_error?: string;
    warning?: string;
  };
  session: Session;
}

/** A single tool call surfaced in a streamed "model" step. */
export interface StepToolCall {
  id: string;
  name: string;
  args: unknown;
}

/** Provider token accounting, including Gemini explicit-cache hits. */
export interface UsageMetadata {
  prompt_token_count?: number;
  response_token_count?: number;
  total_token_count?: number;
  cached_content_token_count?: number;
}

/** A tool call has been dispatched using its stable correlation id. */
export interface ToolStartStep {
  phase: "tool_start";
  step: number;
  id: string;
  name: string;
}

/** A tool call reached a terminal result; no full output is exposed. */
export interface ToolFinishStep {
  phase: "tool_finish";
  step: number;
  id: string;
  name: string;
  ok: boolean;
  duration_ms: number;
  summary: string | null;
  /** A bounded categorical error code, never the provider/tool error body. */
  error: string | null;
}

/**
 * Live progress payloads emitted on the "agent-step" event. A discriminated
 * union on `phase` so the UI can render each kind precisely.
 */
export type AgentStep = (
  | { phase: "step"; step: number; messages: number }
  | { phase: "delta"; step: number; delta: string }
  | {
      phase: "model";
      step: number;
      text: string;
      tool_calls: StepToolCall[];
      usage?: UsageMetadata | null;
    }
  | ToolStartStep
  | ToolFinishStep
  | {
      phase: "finish";
      reason: string;
      success: boolean;
      steps: number;
      final: string | null;
    }
) & { request_id?: string };

export type LiveSessionKind =
  | "writing"
  | "planning"
  | "simulation"
  | "ide"
  | "discuss"
  | "harness";

export type ThinkingLevel = "light" | "balanced" | "deep";

export interface LiveRunOptions {
  sampling?: SamplingParams;
  thinkingLevel?: ThinkingLevel;
}

/**
 * Run the real agent loop, streaming each step to `onStep`.
 *
 * Sets up the "agent-step" listener first, invokes `run_goal_live`, and always
 * tears the listener down in a `finally` — even when the invoke rejects (e.g.
 * no active model provider, in which case the core throws a string we surface
 * verbatim to the caller).
 */
export async function runGoalLive(
  goal: string,
  title: string,
  onStep: (step: AgentStep) => void,
    sessionId?: string,
    sessionKind: LiveSessionKind = "writing",
    requestId?: string,
    options?: LiveRunOptions,
): Promise<LiveRun> {
  if (!isDesktop()) {
    throw new NotInDesktopError();
  }
  const activeRequestId = requestId ?? newRequestId("agent");
  const un = await listen<AgentStep>("agent-step", (e) => {
    if (e.payload.request_id === activeRequestId) onStep(e.payload);
  });
  try {
    return (await tauriInvoke("run_goal_live", {
      goal,
      title,
      sessionId,
      sessionKind,
      requestId: activeRequestId,
      sampling: options?.sampling,
      thinkingLevel: options?.thinkingLevel,
    })) as LiveRun;
  } finally {
    un();
  }
}

/** A single turn in a plain (tool-less) chat with the active model. */
export interface ChatMessage {
  role: "system" | "user" | "assistant";
  content: string;
}

/**
 * Plain multi-turn chat with the active model — no tools, no agent loop. Used
 * by the 策划 (planning) screen's 与 AI 探讨 discussion. Pass the full running
 * history each call; resolves with the assistant's reply text.
 *
 * Throws a string when no model provider is active (surfaced verbatim to the
 * caller, mirroring `run_goal_live`).
 */
export async function chat(messages: ChatMessage[]): Promise<string> {
  if (!isDesktop()) {
    throw new NotInDesktopError();
  }
  return (await tauriInvoke("chat", { messages })) as string;
}

/**
 * Streaming multi-turn chat: the reply types out live. `onDelta` is called with
 * each text fragment as it arrives (over the `chat-delta` event); the promise
 * resolves with the full reply text. Sets up the listener before invoking and
 * always tears it down in a `finally`, mirroring `runGoalLive`.
 */
export async function chatStream(
  messages: ChatMessage[],
  onDelta: (delta: string) => void,
  sessionId?: string,
  requestId?: string,
): Promise<{ text: string; sessionId: string; warning?: string; cancelled?: boolean }> {
  if (!isDesktop()) {
    throw new NotInDesktopError();
  }
  const activeRequestId = requestId ?? newRequestId("chat");
  const un = await listen<{ delta: string; request_id?: string }>("chat-delta", (e) => {
    if (e.payload.request_id === activeRequestId) onDelta(e.payload.delta);
  });
  try {
    const res = (await tauriInvoke("chat_stream", {
      messages,
      sessionId,
      requestId: activeRequestId,
    })) as {
      text: string;
      session_id: string;
      warning?: string;
      cancelled?: boolean;
    };
    return {
      text: res.text,
      sessionId: res.session_id,
      warning: res.warning,
      cancelled: res.cancelled,
    };
  } finally {
    un();
  }
}
