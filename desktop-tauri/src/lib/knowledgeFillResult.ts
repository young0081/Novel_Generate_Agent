export interface KnowledgeFillSummary {
  added: number;
  status?: "completed" | "partial" | "failed" | "cancelled";
  error?: string | null;
  sources?: number;
  outcome: { stopped_reason: string };
}

export function collectionStatus(status?: string): string {
  return ({ completed: "已完成", partial: "部分完成", failed: "未完成", cancelled: "已停止", running: "上次采集未结算", interrupted: "意外中断" } as Record<string, string>)[status ?? ""] ?? "待继续";
}

export function describeKnowledgeFill(result: KnowledgeFillSummary) {
  const cancelled = result.status === "cancelled" || result.outcome.stopped_reason === "cancelled";
  const success = !cancelled && result.added > 0 && !result.error &&
    (result.status ? result.status === "completed" : result.outcome.stopped_reason === "goal_reached");
  const note = cancelled
    ? `已停止采集，已保留 ${result.added} 条设定资料`
    : success
      ? `采集完成，已写入 ${result.added} 条设定资料`
      : result.added > 0
        ? `采集未完整完成，已保留 ${result.added} 条设定资料`
        : "采集失败，未写入新资料";
  return { success, cancelled, note, error: result.error || (!success && !cancelled && result.added === 0
    ? "没有保存有效条目，请检查联网结果或更换来源后重试。" : null) };
}
