import assert from "node:assert/strict";
import test from "node:test";
import { Buffer } from "node:buffer";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";

const bundled = await build({
  entryPoints: [fileURLToPath(new URL("../src/lib/agentRun.ts", import.meta.url))],
  bundle: true,
  format: "esm",
  platform: "node",
  target: "node22",
  write: false,
  plugins: [
    {
      name: "agent-run-icon-stubs",
      setup(esbuild) {
        esbuild.onResolve({ filter: /^\.\.\/components\/icons$/ }, () => ({
          path: "agent-run-icons",
          namespace: "test",
        }));
        esbuild.onLoad({ filter: /.*/, namespace: "test" }, () => ({
          contents: [
            "export const IconSave = () => null;",
            "export const IconFile = () => null;",
            "export const IconFolder = () => null;",
            "export const IconSearch = () => null;",
            "export const IconBranch = () => null;",
            "export const IconTools = () => null;",
          ].join("\n"),
          loader: "js",
        }));
      },
    },
  ],
});

const moduleUrl = `data:text/javascript;base64,${Buffer.from(bundled.outputFiles[0].text).toString("base64")}`;
const {
  DISCUSS_STAGES,
  PHASE_META,
  derivePhase,
  settlePendingTools,
  upsertStep,
  workflowView,
} = await import(moduleUrl);

test("discussion workflow uses the native Agent stages", () => {
  assert.deepEqual(
    DISCUSS_STAGES.map((stage) => stage.label),
    ["理解", "运思", "取用", "回应"],
  );
});

function currentPhase(steps, overrides = {}) {
  return derivePhase({
    running: true,
    steps,
    finished: false,
    success: null,
    errored: false,
    ...overrides,
  });
}

test("agent events form a complete preparing-to-done sequence", () => {
  let key = 0;
  let steps = [];
  const nextKey = () => ++key;

  assert.equal(currentPhase(steps), "warming");

  const sequence = [
    {
      event: { phase: "step", step: 1, messages: 2 },
      phase: "reasoning",
      stepCount: 1,
      streaming: false,
    },
    {
      event: { phase: "delta", step: 1, delta: "先审视人物动机" },
      phase: "streaming",
      stepCount: 1,
      streaming: true,
      text: "先审视人物动机",
    },
    {
      event: {
        phase: "model",
        step: 1,
        text: "先审视人物动机",
        tool_calls: [{ id: "call-1", name: "read_file", args: { path: "chapter.md" } }],
      },
      phase: "tooling",
      stepCount: 1,
      streaming: false,
      toolStatus: "queued",
    },
    {
      event: { phase: "tool_start", step: 1, id: "call-1", name: "read_file" },
      phase: "tooling",
      stepCount: 1,
      streaming: false,
      toolStatus: "running",
    },
    {
      event: {
        phase: "tool_finish",
        step: 1,
        id: "call-1",
        name: "read_file",
        ok: true,
        duration_ms: 24,
        summary: "已读取章节",
        error: null,
      },
      phase: "reasoning",
      stepCount: 1,
      streaming: false,
      toolStatus: "success",
    },
    {
      event: { phase: "step", step: 2, messages: 4 },
      phase: "reasoning",
      stepCount: 2,
      streaming: false,
    },
    {
      event: { phase: "delta", step: 2, delta: "据此收束成稿" },
      phase: "streaming",
      stepCount: 2,
      streaming: true,
      text: "据此收束成稿",
    },
    {
      event: { phase: "model", step: 2, text: "据此收束成稿", tool_calls: [] },
      phase: "reasoning",
      stepCount: 2,
      streaming: false,
    },
    {
      event: {
        phase: "finish",
        reason: "goal_reached",
        success: true,
        steps: 2,
        final: "成稿",
      },
      phase: "done",
      stepCount: 2,
      streaming: false,
      finished: true,
    },
  ];

  for (const row of sequence) {
    steps = upsertStep(steps, row.event, nextKey);
    const last = steps.at(-1);
    assert.equal(steps.length, row.stepCount, `${row.event.phase}: step count`);
    assert.equal(last.streaming, row.streaming, `${row.event.phase}: streaming flag`);
    if (row.text !== undefined) assert.equal(last.text, row.text, `${row.event.phase}: text`);
    if (row.toolStatus !== undefined) {
      assert.equal(steps[0].toolCalls[0].status, row.toolStatus, `${row.event.phase}: tool status`);
    }
    assert.equal(
      currentPhase(steps, row.finished
        ? { running: false, finished: true, success: true }
        : {}),
      row.phase,
      `${row.event.phase}: phase`,
    );
  }
});

test("model, tool lifecycle, and finish events clear streaming", () => {
  const clearingEvents = [
    {
      name: "model",
      event: { phase: "model", step: 1, text: "完整文字", tool_calls: [] },
    },
    {
      name: "tool_start",
      event: { phase: "tool_start", step: 1, id: "call-1", name: "read_file" },
    },
    {
      name: "tool_finish",
      event: {
        phase: "tool_finish",
        step: 1,
        id: "call-1",
        name: "read_file",
        ok: false,
        duration_ms: 8,
        summary: null,
        error: "failed",
      },
    },
    {
      name: "finish",
      event: { phase: "finish", reason: "model_stop", success: false, steps: 1, final: null },
    },
  ];

  for (const row of clearingEvents) {
    let key = 0;
    let steps = upsertStep([], { phase: "delta", step: 1, delta: "流式文字" }, () => ++key);
    assert.equal(steps[0].streaming, true, `${row.name}: precondition`);
    steps = upsertStep(steps, row.event, () => ++key);
    assert.equal(steps[0].streaming, false, `${row.name}: clears streaming`);
  }
});

test("tool completion maps recoverable error and cancellation states", () => {
  const cases = [
    { error: "tool_failed", expected: "error" },
    { error: "cancelled", expected: "cancelled" },
  ];

  for (const row of cases) {
    let key = 0;
    let steps = upsertStep([], {
      phase: "model",
      step: 1,
      text: "先查阅已有设定",
      tool_calls: [{ id: "call-1", name: "memory_recall", args: { query: "旧伞" } }],
    }, () => ++key);
    steps = upsertStep(steps, {
      phase: "tool_start",
      step: 1,
      id: "call-1",
      name: "memory_recall",
    }, () => ++key);
    steps = upsertStep(steps, {
      phase: "tool_finish",
      step: 1,
      id: "call-1",
      name: "memory_recall",
      ok: false,
      duration_ms: 18,
      summary: null,
      error: row.error,
    }, () => ++key);

    assert.equal(steps[0].toolCalls[0].status, row.expected);
    assert.equal(currentPhase(steps), "reasoning", "a tool failure can return to model reasoning");
  }
});

test("derivePhase covers every run phase including streaming", () => {
  const blankStep = { key: 1, step: 1, text: "", toolCalls: [], streaming: false };
  const cases = [
    { expected: "idle", overrides: { running: false } },
    { expected: "warming", overrides: {} },
    { expected: "reasoning", overrides: { steps: [blankStep] } },
    { expected: "streaming", overrides: { steps: [{ ...blankStep, streaming: true }] } },
    {
      expected: "tooling",
      overrides: {
        steps: [{
          ...blankStep,
          streaming: true,
          toolCalls: [{ id: "call-1", name: "read_file", args: null, status: "running" }],
        }],
      },
    },
    { expected: "cancelling", overrides: { cancelling: true } },
    { expected: "cancelled", overrides: { running: false, cancelled: true } },
    {
      expected: "done",
      overrides: { running: false, finished: true, success: true },
    },
    {
      expected: "stopped",
      overrides: { running: false, finished: true, success: false },
    },
    { expected: "error", overrides: { running: false, errored: true } },
  ];

  for (const row of cases) {
    assert.equal(currentPhase([], row.overrides), row.expected, row.expected);
  }

  assert.deepEqual(workflowView("streaming"), { current: 1, state: "running" });
  assert.equal(PHASE_META.streaming.label, "正在成文");
  assert.equal(PHASE_META.streaming.live, true);
});

test("settlePendingTools closes pending calls without rewriting terminal calls", () => {
  const terminalCall = {
    id: "done",
    name: "memory_save",
    args: null,
    status: "success",
    summary: "保留此摘要",
  };
  const base = [{
    key: 1,
    step: 1,
    text: "仍在流式输出",
    streaming: true,
    toolCalls: [
      { id: "queued", name: "read_file", args: null, status: "queued" },
      { id: "running", name: "write_file", args: null, status: "running" },
      terminalCall,
    ],
  }];
  const cases = [
    { status: "success", message: undefined },
    { status: "error", message: "供应商中断" },
    { status: "cancelled", message: "用户停止" },
  ];

  for (const row of cases) {
    const settled = settlePendingTools(base, row.status, row.message);
    assert.equal(settled[0].streaming, false, `${row.status}: clears streaming`);
    assert.deepEqual(
      settled[0].toolCalls.slice(0, 2).map((call) => call.status),
      [row.status, row.status],
      `${row.status}: closes pending calls`,
    );
    assert.strictEqual(settled[0].toolCalls[2], terminalCall, `${row.status}: preserves terminal call`);
    if (row.status === "error") {
      assert.equal(settled[0].toolCalls[0].error, row.message);
      assert.equal(settled[0].toolCalls[0].summary, row.message);
    }
    if (row.status === "cancelled") {
      assert.equal(settled[0].toolCalls[0].error, undefined);
      assert.equal(settled[0].toolCalls[0].summary, row.message);
    }
  }

  const alreadySettled = [{ ...base[0], streaming: false, toolCalls: [terminalCall] }];
  assert.strictEqual(settlePendingTools(alreadySettled, "success"), alreadySettled);
});
