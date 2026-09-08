import assert from "node:assert/strict";
import test from "node:test";
import { collectionStatus, describeKnowledgeFill } from "../src/lib/knowledgeFillResult.ts";

test("unfinished historical collection is never labeled complete", () => {
  assert.equal(collectionStatus("running"), "上次采集未结算");
  assert.equal(collectionStatus("interrupted"), "意外中断");
  assert.equal(collectionStatus("partial"), "部分完成");
});

test("zero saved entries never report collection success, even with an old backend", () => {
  const summary = describeKnowledgeFill({ added: 0, outcome: { stopped_reason: "goal_reached" } });
  assert.equal(summary.success, false);
  assert.ok(summary.error);
});

test("collection distinguishes success, partial failure and cancellation with retained entries", () => {
  for (const status of ["completed", "partial", "failed", "cancelled"]) {
    const summary = describeKnowledgeFill({ status, added: 8, outcome: { stopped_reason: "goal_reached" } });
    assert.equal(summary.success, status === "completed");
    assert.equal(summary.cancelled, status === "cancelled");
    assert.ok(summary.note.includes("8"));
  }
  const partial = describeKnowledgeFill({ status: "partial", added: 3, error: "network failed", outcome: { stopped_reason: "model_stop" } });
  assert.equal(partial.error, "network failed");
  assert.ok(partial.note.includes("已保留 3"));
});
