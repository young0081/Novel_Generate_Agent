import type { SessionRecord } from "./sessions";

/** The durable phases exposed by the Harness workbench. */
export const HARNESS_STAGES = [
  { key: "plan", label: "计划" },
  { key: "execute", label: "执行" },
  { key: "verify", label: "验证" },
  { key: "deliver", label: "交付" },
] as const;

export type HarnessStageKey = (typeof HARNESS_STAGES)[number]["key"];

/**
 * Turn the form state into a durable protocol for the native agent. The
 * protocol is intentionally explicit: a Harness run must leave evidence, not
 * only a polished answer.
 */
export function buildHarnessGoal(
  task: string,
  acceptance: string,
  constraints: string,
): string {
  const cleanTask = task.trim();
  const cleanAcceptance = acceptance.trim() || "完成任务并说明可复核的结果";
  const cleanConstraints = constraints.trim() || "保持现有功能兼容，不做无关改动";
  return [
    "你正在执行一个 Harness 长任务。",
    "",
    "## 任务目标",
    cleanTask,
    "",
    "## 验收标准",
    cleanAcceptance,
    "",
    "## 约束",
    cleanConstraints,
    "",
    "## 必须遵守的执行协议",
    "1. 计划：先读取与任务相关的现状，列出可执行步骤、依赖和风险。",
    "2. 执行：逐步完成工作；涉及文件、命令或工具时真实执行，不要虚构结果。",
    "3. 验证：对每条验收标准给出证据。失败时先修复，再重新验证。",
    "4. 交付：最后用固定小节输出「已完成」「验证证据」「未完成/风险」「建议下一步」。",
    "5. 如果任务被中断，保留当前进度，下一轮从已有结果继续，不重复已完成的破坏性操作。",
  ].join("\n");
}

export function harnessContinuationGoal(
  task: string,
  acceptance: string,
  constraints: string,
  followUp: string,
): string {
  return [
    "继续当前 Harness 会话，不要从零开始。",
    buildHarnessGoal(task, acceptance, constraints),
    "",
    "## 本轮继续要求",
    followUp.trim() || "检查上次停留的阶段，完成尚未满足的验收标准并补齐证据。",
    "沿用历史会话、工具结果与工作区现状；先核对再行动。",
  ].join("\n");
}

/** Recover the form fields from a saved Harness record. */
export function restoreHarnessSession(record: SessionRecord): {
  task: string;
  acceptance: string;
  constraints: string;
  finalAnswer: string | null;
} {
  if (record.kind !== "harness") {
    throw new Error("该记录不是 Harness 会话，无法在此继续");
  }
  const userMessages = record.session.messages.filter((message) => message.role === "user");
  const source = record.goal || (userMessages.length > 0
    ? userMessages[userMessages.length - 1].content
    : "");
  const section = (name: string, next: string): string => {
    const match = source.match(new RegExp(`## ${name}\\s*\\n([\\s\\S]*?)(?=\\n## ${next}\\s*\\n|$)`));
    return match?.[1]?.trim() || "";
  };
  const finalAnswer = [...record.session.messages].reverse().find((message) =>
    message.role === "assistant" && !message.tool_call && message.content.trim(),
  )?.content ?? null;
  return {
    task: section("任务目标", "验收标准"),
    acceptance: section("验收标准", "约束"),
    constraints: section("约束", "必须遵守的执行协议"),
    finalAnswer,
  };
}
