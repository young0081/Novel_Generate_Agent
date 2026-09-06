import type {
  AgentStep,
  StepToolCall,
  ToolFinishStep,
  ToolStartStep,
} from "./studio";

export const MODEL_TOOL_CALL_CONTRACT = {
  id: "call_contract_1",
  name: "read_file",
  args: { path: "chapter.md" },
} satisfies StepToolCall;

export const TOOL_START_CONTRACT = {
  phase: "tool_start",
  step: 2,
  id: "call_contract_1",
  name: "read_file",
  request_id: "request_contract_1",
} satisfies AgentStep & ToolStartStep;

export const TOOL_FINISH_CONTRACT = {
  phase: "tool_finish",
  step: 2,
  id: "call_contract_1",
  name: "read_file",
  ok: true,
  duration_ms: 17,
  summary: "completed",
  error: null,
  request_id: "request_contract_1",
} satisfies AgentStep & ToolFinishStep;

export const TOOL_FINISH_REJECTS_RAW_OUTPUT: ToolFinishStep = {
  phase: "tool_finish",
  step: 2,
  id: "call_contract_1",
  name: "read_file",
  ok: true,
  duration_ms: 17,
  summary: "completed",
  error: null,
  // @ts-expect-error Lifecycle events intentionally cannot carry full tool output.
  output: "sensitive result",
};
