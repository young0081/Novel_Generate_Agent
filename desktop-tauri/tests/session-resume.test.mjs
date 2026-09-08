import assert from "node:assert/strict";
import test from "node:test";
import {
  countNewPlanningSaves,
  planningContinuationGoal,
  planningStopNote,
  PLANNING_ACTION_TITLES,
  restorePlanningSession,
  sessionResumeTarget,
} from "../src/lib/sessionResume.ts";

test("planning budget and cancellation notes preserve actual saved counts", () => {
  for (const reason of ["已达到预算限制", "已由用户停止", "已达到步数上限"]) {
    const note = planningStopNote(reason, 2, 9);
    assert.ok(note.includes(reason));
    assert.ok(note.includes("已保留 2 条新增设定"));
    assert.ok(note.includes("共 9 步"));
    assert.ok(note.includes("继续当前会话"));
  }
});
import {
  buildHarnessGoal,
  harnessContinuationGoal,
  restoreHarnessSession,
} from "../src/lib/harness.ts";

function record(overrides = {}) {
  return {
    kind: "planning",
    goal: "你是策划助手。作者的构思如下：\n\n「海上城邦\n主角寻找「失落航线」，保持克制。」\n\n## 你的任务\n构建设定",
    session: {
      id: "saved-planning", title: "世界观设定", state: {},
      created_ms: 1, updated_ms: 2, messages: [
        { role: "system", content: "项目规则" },
        { role: "assistant", content: "已保存海上城邦的航行规则" },
        { role: "assistant", content: "保存动作", tool_call: { id: "save-old", name: "memory_save", args: {} } },
        { role: "tool", content: "saved", tool_result: { call_id: "save-old", name: "memory_save", ok: true } },
      ],
    },
    ...overrides,
  };
}

test("planning history routes to planning without changing writing or discussion routes", () => {
  assert.deepEqual(sessionResumeTarget("planning"), { mode: "planning", label: "继续策划" });
  assert.deepEqual(sessionResumeTarget("writing"), { mode: "studio", label: "继续创作" });
  assert.deepEqual(sessionResumeTarget("discuss"), { mode: "discuss", label: "继续探讨" });
  assert.deepEqual(sessionResumeTarget("harness"), { mode: "harness", label: "继续 Harness" });
  for (const kind of ["simulation", "ide", "unknown", "toString"]) {
    assert.equal(sessionResumeTarget(kind), null);
  }
});

test("all four planning actions restore the concept and last answer without treating tool text as an answer", () => {
  for (const [actionKey, title] of Object.entries(PLANNING_ACTION_TITLES)) {
    const saved = record();
    saved.session.title = title;
    assert.deepEqual(restorePlanningSession(saved), {
      actionKey,
      concept: "海上城邦\n主角寻找「失落航线」，保持克制。",
      finalAnswer: "已保存海上城邦的航行规则",
    });
  }
});

test("legacy planning prompts restore from history when the latest goal is a follow-up", () => {
  const saved = record({ goal: "补充港口势力关系" });
  saved.session.messages.unshift({ role: "user", content: "下面是作者的构思：\r\n「潮汐决定魔法的力量」\r\n\r\n请据此设计人物" });
  assert.equal(restorePlanningSession(saved).concept, "潮汐决定魔法的力量");
});

test("continuation keeps a recoverable updated concept and the author's new requirement", () => {
  const goal = planningContinuationGoal("人物设定", " 新的构思 ", " 补充主角动机 ");
  assert.ok(goal.includes("补充主角动机"));
  assert.ok(goal.includes("避免重复入库"));
  assert.equal(restorePlanningSession(record({ goal })).concept, "新的构思");
  assert.ok(planningContinuationGoal("人物设定", "构思", " ").includes("完成尚未完成的部分"));
});

test("unrecognized history stays recoverable without placing the whole generated prompt in the concept", () => {
  assert.equal(restorePlanningSession(record({ goal: null })).concept, "");
  assert.throws(() => restorePlanningSession(record({ kind: "writing" })), /不是策划会话/);
  const saved = record();
  saved.session.title = "无法识别的旧任务";
  assert.throws(() => restorePlanningSession(saved), /无法识别/);
});

test("save counts exclude historical, failed, and unrelated tool results after resuming", () => {
  const previous = record().session;
  const result = {
    ...previous,
    messages: [
      ...previous.messages,
      { role: "tool", tool_result: { call_id: "failed", name: "memory_save", ok: false } },
      { role: "tool", tool_result: { call_id: "recall", name: "memory_recall", ok: true } },
      { role: "tool", tool_result: { call_id: "new-save", name: "memory_save", ok: true } },
    ],
  };
  assert.equal(countNewPlanningSaves(previous, previous), 0);
  assert.equal(countNewPlanningSaves(previous, null), 1);
  assert.equal(countNewPlanningSaves(result, previous), 1);
  assert.equal(countNewPlanningSaves({ ...result, messages: result.messages.slice(-3) }, previous), 1);
});

test("Harness goals preserve task, acceptance, constraints, and continuation instructions", () => {
  const goal = buildHarnessGoal("修复历史恢复", "回归测试通过\n页面可以继续", "保留旧版本");
  assert.ok(goal.includes("## 任务目标\n修复历史恢复"));
  assert.ok(goal.includes("## 验收标准\n回归测试通过\n页面可以继续"));
  assert.ok(goal.includes("## 约束\n保留旧版本"));
  assert.ok(goal.includes("验证：对每条验收标准给出证据"));
  const continued = harnessContinuationGoal("修复历史恢复", "测试通过", "不改无关功能", "补齐失败用例");
  assert.ok(continued.includes("## 本轮继续要求\n补齐失败用例"));
});

test("Harness history restores its form and final answer", () => {
  const saved = {
    kind: "harness",
    goal: buildHarnessGoal("检查执行链", "必须有验证证据", "不破坏现有功能"),
    session: {
      id: "harness-1", title: "Harness · 检查执行链", state: {},
      created_ms: 1, updated_ms: 2,
      messages: [{ role: "assistant", content: "已完成并验证" }],
    },
  };
  assert.deepEqual(restoreHarnessSession(saved), {
    task: "检查执行链",
    acceptance: "必须有验证证据",
    constraints: "不破坏现有功能",
    finalAnswer: "已完成并验证",
  });
  assert.throws(() => restoreHarnessSession({ ...saved, kind: "writing" }), /不是 Harness/);
});
