import assert from "node:assert/strict";
import test from "node:test";
import {
  latestWritingAnswer,
  resolveWritingResult,
  writingAnswerText,
} from "../src/lib/studioResult.ts";

const previousTurn = [
  { role: "user", content: "Write the prologue" },
  { role: "assistant", content: "The prologue is saved to book/chapter-1.md." },
];

function run(messages, outcome = {}) {
  return {
    session: { id: "writing-session", title: "Chapter", messages, state: {}, created_ms: 1, updated_ms: 2 },
    outcome: { steps: 5, stopped_reason: "model_stop", final_answer: null, ...outcome },
  };
}

test("an empty continuation never shows the previous chapter's completed summary", () => {
  const messages = [
    ...previousTurn,
    { role: "user", content: "Continue with chapter two" },
    { role: "assistant", content: "Reading the outline", tool_call: { id: "read-1", name: "read_file", args: {} } },
    { role: "tool", content: "Outline", tool_result: { call_id: "read-1", name: "read_file", ok: true, untrusted: false } },
    { role: "assistant", content: "" },
  ];
  assert.equal(latestWritingAnswer(messages), null);
  assert.deepEqual(resolveWritingResult(run(messages)), { finalAnswer: null, success: false, cancelled: false });
  assert.equal(messages[1].content, previousTurn[1].content);
});

test("history restoration stops at the most recent user even across several failed continuations", () => {
  const messages = [
    ...previousTurn,
    { role: "user", content: "Continue with chapter two" },
    { role: "assistant", content: "A partial second chapter" },
    { role: "user", content: "Continue the draft" },
    { role: "assistant", content: "  " },
  ];
  assert.equal(latestWritingAnswer(messages), null);
  assert.equal(latestWritingAnswer(previousTurn), previousTurn[1].content);
  assert.equal(latestWritingAnswer([{ role: "assistant", content: "Unscoped history" }]), null);
});

test("an interrupted run preserves its own partial answer", () => {
  const messages = [
    ...previousTurn,
    { role: "user", content: "Continue with chapter two" },
    { role: "assistant", content: "Chapter two begins at the harbor." },
    { role: "assistant", content: "" },
  ];
  assert.deepEqual(resolveWritingResult(run(messages, { stopped_reason: "budget" })), {
    finalAnswer: "Chapter two begins at the harbor.", success: false, cancelled: false,
  });
});

test("runtime diagnostics remain in the transcript without becoming a draft", () => {
  for (const marker of ["loop guard", "protocol error", "completion check", "response recovery"]) {
    const diagnostic = `[${marker}] retry the action`;
    const messages = [
      ...previousTurn,
      { role: "user", content: "Continue with chapter two" },
      { role: "assistant", content: diagnostic },
    ];
    assert.equal(latestWritingAnswer(messages), null);
    assert.equal(resolveWritingResult(run(messages, { final_answer: diagnostic })).finalAnswer, null);
    assert.equal(messages.at(-1).content, diagnostic);
  }
});

test("a draft preceding an appended loop guard is retained without the diagnostic suffix", () => {
  const draft = "Chapter two\n\nThe harbor fell silent.";
  const guarded = `${draft}\n\n[loop guard] Use write_file to save the chapter.`;
  assert.equal(writingAnswerText(guarded), draft);
  assert.equal(writingAnswerText("The sign read [loop guard] by the gate."), "The sign read [loop guard] by the gate.");
});

test("a successful returned outcome supplies the answer and status without a finish event", () => {
  const messages = [...previousTurn, { role: "user", content: "Continue with chapter two" }];
  assert.deepEqual(resolveWritingResult(run(messages, {
    stopped_reason: "goal_reached", final_answer: "Chapter two is saved.",
  })), { finalAnswer: "Chapter two is saved.", success: true, cancelled: false });
});

test("the returned final answer takes precedence over a different transcript answer", () => {
  const messages = [
    ...previousTurn,
    { role: "user", content: "Continue with chapter two" },
    { role: "assistant", content: "An earlier partial draft" },
  ];
  assert.equal(resolveWritingResult(run(messages, {
    stopped_reason: "goal_reached", final_answer: "The completed chapter",
  })).finalAnswer, "The completed chapter");
});

test("cancellation preserves this turn's partial draft and never reports completion", () => {
  const messages = [
    ...previousTurn,
    { role: "user", content: "Continue with chapter two" },
    { role: "assistant", content: "An unfinished chapter" },
  ];
  assert.deepEqual(resolveWritingResult(run(messages, { stopped_reason: "cancelled" })), {
    finalAnswer: "An unfinished chapter", success: false, cancelled: true,
  });
});
