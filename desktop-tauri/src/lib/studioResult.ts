import type { LiveRun, Message } from "./studio";

export function writingAnswerText(content: string | null | undefined): string | null {
  const text = content?.trim();
  if (!text) return null;
  const diagnostic = text.search(/^[ \t]*\[(?:loop guard|protocol error|completion check|response recovery)\]/m);
  return (diagnostic < 0 ? text : text.slice(0, diagnostic).trim()) || null;
}

export function latestWritingAnswer(messages: readonly Message[]): string | null {
  let turnStart = messages.length - 1;
  while (turnStart >= 0 && messages[turnStart].role !== "user") turnStart -= 1;
  if (turnStart < 0) return null;

  // A resumed run may have no answer. Never borrow a previous chapter's result.
  for (let i = messages.length - 1; i > turnStart; i -= 1) {
    const message = messages[i];
    if (message.role !== "assistant" || message.tool_call) continue;
    const answer = writingAnswerText(message.content);
    if (answer) return answer;
  }
  return null;
}

export function resolveWritingResult(run: LiveRun) {
  return {
    finalAnswer: writingAnswerText(run.outcome.final_answer)
      ?? latestWritingAnswer(run.session.messages),
    success: run.outcome.stopped_reason === "goal_reached",
    cancelled: run.outcome.stopped_reason === "cancelled",
  };
}
