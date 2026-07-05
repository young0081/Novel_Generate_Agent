// Shared model + helpers for a live agent run (创作 / 策划 generation feeds).
//
// Both the Writing Studio and the Planning screen drive the same real agent
// loop via `runGoalLive` and stream `AgentStep`s. This module centralises the
// once-duplicated derivations — turning that raw stream into a structured view
// the Agent Console components render: the current work phase (工作状态), the
// macro workflow stage (工作流程), per-step reasoning (模型推理) and tool calls
// (工具调用).

import type { ReactNode } from "react";
import {
  IconSave,
  IconFile,
  IconFolder,
  IconSearch,
  IconBranch,
  IconTools,
} from "../components/icons";
import type { AgentStep, StepToolCall } from "./studio";

/** A single model turn rendered as a card in the live feed. */
export interface RunStep {
  key: number;
  step: number;
  text: string;
  toolCalls: StepToolCall[];
}

/**
 * The agent's current work state. Drives the live 工作状态 strip and the
 * highlighted node of the 工作流程 tracker.
 */
export type AgentPhase =
  | "idle" // not started / cleared
  | "warming" // started, model not yet responded (唤起模型)
  | "reasoning" // mid-run, last turn was pure thought
  | "tooling" // mid-run, last turn issued tool calls
  | "done" // finished successfully
  | "stopped" // finished without success (budget / no-progress / max steps)
  | "error"; // the call threw (e.g. network / provider)

/**
 * Fold a streamed `delta` or final `model` step into the running step list,
 * keyed by step number so a step's reasoning grows live as tokens arrive and is
 * then reconciled (authoritative text + tool calls) when the step completes.
 * Non-step events are returned unchanged. `nextKey` mints a fresh React key.
 */
export function upsertStep(
  steps: RunStep[],
  s: AgentStep,
  nextKey: () => number,
): RunStep[] {
  if (s.phase !== "delta" && s.phase !== "model") return steps;
  const last = steps[steps.length - 1];
  if (last && last.step === s.step) {
    const updated: RunStep =
      s.phase === "delta"
        ? { ...last, text: last.text + s.delta }
        : { ...last, text: s.text || last.text, toolCalls: s.tool_calls ?? [] };
    return [...steps.slice(0, -1), updated];
  }
  const created: RunStep =
    s.phase === "delta"
      ? { key: nextKey(), step: s.step, text: s.delta, toolCalls: [] }
      : {
          key: nextKey(),
          step: s.step,
          text: s.text ?? "",
          toolCalls: s.tool_calls ?? [],
        };
  return [...steps, created];
}

/** Derive the work phase from the raw run bits the screens already track. */
export function derivePhase(o: {
  running: boolean;
  steps: RunStep[];
  finished: boolean;
  success: boolean | null;
  errored: boolean;
}): AgentPhase {
  if (o.errored) return "error";
  if (o.running) {
    if (o.steps.length === 0) return "warming";
    const last = o.steps[o.steps.length - 1];
    return last.toolCalls.length > 0 ? "tooling" : "reasoning";
  }
  if (o.finished) return o.success === false ? "stopped" : "done";
  return "idle";
}

/** The overall colour/animation tone for a phase. */
export type PhaseTone = "idle" | "warm" | "reason" | "tool" | "done" | "warn";

export interface PhaseMeta {
  label: string;
  tone: PhaseTone;
  /** Whether the status orb should pulse (the agent is actively working). */
  live: boolean;
}

export const PHASE_META: Record<AgentPhase, PhaseMeta> = {
  idle: { label: "待命", tone: "idle", live: false },
  warming: { label: "唤起模型", tone: "warm", live: true },
  reasoning: { label: "运思推理", tone: "reason", live: true },
  tooling: { label: "调用工具", tone: "tool", live: true },
  done: { label: "已完成", tone: "done", live: false },
  stopped: { label: "已停止", tone: "warn", live: false },
  error: { label: "出错", tone: "warn", live: false },
};

// ---- workflow (工作流程) ----------------------------------------------------

/** One macro stage of the agent's working flow. */
export interface WorkflowStageDef {
  key: string;
  label: string;
}

/** Stages for a writing run: 立意 → 运思 → 用器 → 成章. */
export const WRITE_STAGES: WorkflowStageDef[] = [
  { key: "intent", label: "立意" },
  { key: "reason", label: "运思" },
  { key: "tool", label: "用器" },
  { key: "final", label: "成章" },
];

/** Stages for a planning/story-bible generation run: 构思 → 推演 → 入库 → 立稿. */
export const PLAN_STAGES: WorkflowStageDef[] = [
  { key: "intent", label: "构思" },
  { key: "reason", label: "推演" },
  { key: "tool", label: "入库" },
  { key: "final", label: "立稿" },
];

/** Stages for a world-simulation run: 召境 → 推演 → 记录 → 成卦. */
export const SIMULATE_STAGES: WorkflowStageDef[] = [
  { key: "intent", label: "召境" },
  { key: "reason", label: "推演" },
  { key: "tool", label: "记录" },
  { key: "final", label: "成卦" },
];

export type WorkflowState = "idle" | "running" | "done" | "stopped" | "error";

/** Map a phase to the active stage index + the tracker's overall state. */
export function workflowView(phase: AgentPhase): {
  current: number;
  state: WorkflowState;
} {
  switch (phase) {
    case "idle":
      return { current: 0, state: "idle" };
    case "warming":
      return { current: 0, state: "running" };
    case "reasoning":
      return { current: 1, state: "running" };
    case "tooling":
      return { current: 2, state: "running" };
    case "done":
      return { current: 3, state: "done" };
    case "stopped":
      return { current: 3, state: "stopped" };
    case "error":
      return { current: 3, state: "error" };
  }
}

// ---- tool-call presentation -------------------------------------------------

export type IconCmp = (p: { size?: number }) => ReactNode;

/** Trim a value to a compact, single-line preview. */
export function clip(v: string, max = 64): string {
  const oneLine = v.replace(/\s+/g, " ").trim();
  return oneLine.length > max ? `${oneLine.slice(0, max)}…` : oneLine;
}

/** A compact, human-friendly preview of a tool call's arguments. */
export function previewArgs(args: unknown): string | null {
  if (args == null) return null;
  if (typeof args !== "object") return clip(String(args));
  const obj = args as Record<string, unknown>;
  // Surface the most meaningful field first.
  const PRIORITY = [
    "kind",
    "title",
    "name",
    "path",
    "file",
    "message",
    "query",
    "id",
    "branch",
  ];
  for (const k of PRIORITY) {
    const v = obj[k];
    if (typeof v === "string" && v.trim()) return `${k}: ${clip(v)}`;
  }
  for (const [k, v] of Object.entries(obj)) {
    if (typeof v === "string" && v.trim()) return `${k}: ${clip(v)}`;
    if (typeof v === "number" || typeof v === "boolean") return `${k}: ${String(v)}`;
  }
  return null;
}

/** Pretty-print tool args for the expandable detail view. */
export function formatArgs(args: unknown): string {
  if (args == null) return "（无参数）";
  if (typeof args === "string") return args;
  try {
    return JSON.stringify(args, null, 2);
  } catch {
    return String(args);
  }
}

/**
 * Map a tool name to a fitting ink icon + a short human verb, so a tool call
 * reads at a glance ("落笔 write_file", "查阅 memory_recall") instead of a
 * generic wrench for everything.
 */
export function toolGlyph(name: string): { Icon: IconCmp; verb: string } {
  const n = name.toLowerCase();
  if (n.includes("write") || n.includes("save") || n.includes("append")) {
    return { Icon: IconSave, verb: "落笔" };
  }
  if (n.includes("read") || n.includes("open") || n.includes("cat")) {
    return { Icon: IconFile, verb: "展卷" };
  }
  if (n.includes("list") || n.includes("dir") || n.includes("ls")) {
    return { Icon: IconFolder, verb: "查目" };
  }
  if (
    n.includes("search") ||
    n.includes("find") ||
    n.includes("grep") ||
    n.includes("query") ||
    n.includes("recall") ||
    n.includes("memory")
  ) {
    return { Icon: IconSearch, verb: "查阅" };
  }
  if (
    n.includes("vcs") ||
    n.includes("commit") ||
    n.includes("branch") ||
    n.includes("git")
  ) {
    return { Icon: IconBranch, verb: "存档" };
  }
  return { Icon: IconTools, verb: "用器" };
}

/** Detect the "no active model provider" error so callers can offer a fix. */
export function isNoProviderError(message: string): boolean {
  const lower = message.toLowerCase();
  return (
    message.includes("尚未选择当前模型供应商") ||
    message.includes("尚未配置") ||
    message.includes("未选择") ||
    message.includes("未配置") ||
    message.includes("active provider") ||
    lower.includes("no provider") ||
    lower.includes("provider is not configured")
  );
}

/** Detect provider failures that commonly mean native tool calling is rejected. */
export function isProviderCompatibilityError(message: string): boolean {
  const lower = message.toLowerCase();
  return (
    lower.includes("bad_response_status_code") ||
    lower.includes("tool_choice") ||
    lower.includes("tool_calls") ||
    lower.includes("\"tools\"") ||
    lower.includes("function calling") ||
    lower.includes("function_call") ||
    lower.includes("tools") ||
    message.includes("返回 500") ||
    message.includes("返回 400") ||
    message.includes("返回 422")
  );
}
