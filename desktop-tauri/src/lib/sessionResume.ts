import type { SessionRecord } from "./sessions";
import type { Session } from "./studio";

export type SessionResumeMode = "discuss" | "studio" | "planning" | "harness";

export function sessionResumeTarget(kind: string): { mode: SessionResumeMode; label: string } | null {
  switch (kind) {
    case "discuss": return { mode: "discuss", label: "继续探讨" };
    case "writing": return { mode: "studio", label: "继续创作" };
    case "planning": return { mode: "planning", label: "继续策划" };
    case "harness": return { mode: "harness", label: "继续 Harness" };
    default: return null;
  }
}

export const PLANNING_ACTION_TITLES = {
  worldbuilding: "世界观设定",
  character: "人物设定",
  outline: "故事大纲",
  foreshadow: "伏笔设计",
} as const;

export type PlanningActionKey = keyof typeof PLANNING_ACTION_TITLES;

export function restorePlanningSession(record: SessionRecord) {
  if (record.kind !== "planning") throw new Error("该记录不是策划会话，无法在此继续");
  const actionKey = (Object.keys(PLANNING_ACTION_TITLES) as PlanningActionKey[])
    .find((key) => PLANNING_ACTION_TITLES[key] === record.session.title);
  if (!actionKey) throw new Error("无法识别该策划会话的设定类型");

  // The saved goal is the latest turn. Older sessions keep the original
  // concept inside a guided user message; support both prompt generations.
  const goals = [record.goal, ...record.session.messages
    .filter((message) => message.role === "user")
    .reverse().map((message) => message.content)];
  let concept = "";
  for (const goal of goals) {
    const match = goal?.match(/作者的构思如下：\s*「([\s\S]*?)」\r?\n\r?\n/)
      ?? goal?.match(/下面是作者的构思：\s*「([\s\S]*?)」\r?\n\r?\n/);
    if (match) {
      concept = match[1];
      break;
    }
  }
  const finalAnswer = [...record.session.messages].reverse().find((message) =>
    message.role === "assistant" && !message.tool_call && message.content.trim(),
  )?.content ?? null;
  return { actionKey, concept, finalAnswer };
}

export function planningContinuationGoal(title: string, concept: string, followUp: string): string {
  return `继续当前「${title}」策划会话。作者的构思如下：\n\n「${concept.trim()}」\n\n` +
    `## 本轮要求\n${followUp.trim() || "接着上次的进度完善设定，完成尚未完成的部分。"}\n\n` +
    "沿用历史会话中的设定与结论。先核对已保存的内容，避免重复入库；需要新增或修改设定时调用相应记忆工具，并说明本轮实际完成了什么。";
}

/** Count this turn's successful saves, even if older messages were compacted. */
export function countNewPlanningSaves(session: Session, previous: Session | null): number {
  const previousCalls = new Set(previous?.messages.flatMap((message) =>
    message.tool_result ? [message.tool_result.call_id] : [],
  ));
  return session.messages.filter((message) =>
    message.tool_result?.name === "memory_save" && message.tool_result.ok &&
    !previousCalls.has(message.tool_result.call_id),
  ).length;
}

export function planningStopNote(reason: string, savedCount: number, steps: number): string {
  return `${reason}（共 ${steps} 步，已保留 ${savedCount} 条新增设定），可继续当前会话。`;
}
