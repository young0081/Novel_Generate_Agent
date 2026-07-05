// 推演 — World Simulator.
// Users pick existing memory entries to include, add new ad-hoc settings,
// then the AI simulates a cause-effect timeline. New settings are auto-
// classified and saved back to the memory library after simulation.

import { useCallback, useEffect, useRef, useState } from "react";
import { Spinner } from "../components/Spinner";
import {
  IconSimulate, IconCheck, IconWarn, IconProviders,
  IconStar, IconPlus, IconClose, IconRefresh,
} from "../components/icons";
import { describeError, isDesktop, invokeTool } from "../lib/core";
import { useToast } from "../components/Toast";
import { runGoalLive, type AgentStep } from "../lib/studio";
import { getProviders } from "../lib/providers";
import {
  derivePhase, workflowView, isNoProviderError,
  isProviderCompatibilityError, upsertStep, SIMULATE_STAGES, type RunStep,
} from "../lib/agentRun";
import { type MemoryHit, type MemoryRecallData, KIND_LABEL } from "../lib/memory";
import WorkStatus from "../components/agent/WorkStatus";
import WorkflowSteps from "../components/agent/WorkflowSteps";
import AgentFeed from "../components/agent/AgentFeed";

interface SimulateWorkProps { onOpenSettings?: () => void }

type SimType = "plot" | "character" | "world" | "cause";
const SIM_TYPES: Array<{ key: SimType; label: string; blurb: string }> = [
  { key: "plot",      label: "情节推演", blurb: "推演场景后最可能的情节走向" },
  { key: "character", label: "角色反应", blurb: "各角色面对场景时的决策与行动" },
  { key: "world",     label: "世界演变", blurb: "场景对整个世界格局的长期影响" },
  { key: "cause",     label: "因果链",   blurb: "从场景出发推导完整因果影响链" },
];
const TYPE_FOCUS: Record<SimType, string> = {
  plot:      "从下一个节点起，按时间顺序推演 5-8 个情节事件",
  character: "对每个主要角色分析：心理→决策→行动→后果（至少 3 位角色）",
  world:     "宏观视角：局势变化→势力消长→世界规则应激反应",
  cause:     "构建完整因果链：直接原因→结果→间接影响→蝴蝶效应（至少 3 层）",
};

// Kinds shown in the "选择要素" panel
const SELECTABLE_KINDS = [
  "character", "worldbuilding", "setting", "outline", "foreshadow",
] as const;

function buildGoal(
  scenario: string,
  types: SimType[],
  selectedMems: MemoryHit[],
  newSettings: string[],
): string {
  const focusList = types
    .map((t, i) => `${i + 1}. 【${SIM_TYPES.find((s) => s.key === t)!.label}】${TYPE_FOCUS[t]}`)
    .join("\n");

  const memSection = selectedMems.length > 0
    ? `\n\n## 用户指定参与推演的设定要素（直接使用，无需再召回这些条目）\n` +
      selectedMems.map(m =>
        `- **[${KIND_LABEL[m.kind as keyof typeof KIND_LABEL] ?? m.kind}] ${m.title}**：${m.summary}`
      ).join("\n")
    : "";

  const newSection = newSettings.length > 0
    ? `\n\n## 用户新增的设定（纳入推演，推演完成后必须自动录入设定集）\n` +
      newSettings.map((s, i) => `${i + 1}. ${s}`).join("\n")
    : "";

  const recallRule = selectedMems.length > 0
    ? "可补充调用 memory_recall 查询其他未列出的相关设定"
    : '必须先调用 memory_recall 查询"角色 人物"和"世界观 规则 约束"';

  const newSettingsRule = newSettings.length > 0
    ? `6. 推演完成后，对上方每条「用户新增设定」，分析其类型，调用 memory_save 录入：\n` +
      `   - kind 自动判断（character/worldbuilding/setting/foreshadow/lore/other）\n` +
      `   - title 提炼设定名称，content 写完整设定\n`
    : "";

  return (
    `你是这部同人小说世界的全知模拟器。任务是中立客观地模拟事件走向，不是创作散文。` +
    memSection + newSection +
    `\n\n## ⚠️ 核心规则（违反任一条视为失败）\n` +
    `1. ${recallRule}\n` +
    `2. 不写散文，只输出结构化事件节点（编号列表，格式：[时序] 主体 → 行动 → 后果）\n` +
    `3. 严格遵守设定：角色性格不能 OOC，世界规则不能违反\n` +
    `4. 必须调用 memory_save(kind="simulation", title="推演：${scenario.slice(0, 20)}…") 保存推演结果\n` +
    `5. 至少推演 5 个事件节点\n` +
    newSettingsRule +
    `\n## 模拟场景\n「${scenario}」\n\n` +
    `## 本次推演维度\n${focusList}\n\n` +
    `现在开始：获取设定 → 推演事件节点 → 保存推演` +
    (newSettings.length > 0 ? ` → 录入新增设定` : "") +
    ` → 一句话总结。每步都必须调用工具。`
  );
}

interface ActiveModel { provider: string; model: string }

export default function SimulateWork({ onOpenSettings }: SimulateWorkProps) {
  const toast = useToast();

  // ── input state ──────────────────────────────────────────────────────────
  const [scenario, setScenario] = useState("");
  const [simTypes, setSimTypes] = useState<SimType[]>(["plot"]);

  // existing memory selection
  const [memoryItems, setMemoryItems] = useState<MemoryHit[]>([]);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [loadingMem, setLoadingMem] = useState(false);

  // user-added new settings
  const [newSettings, setNewSettings] = useState<string[]>([]);
  const [newDraft, setNewDraft] = useState("");

  // ── run state ─────────────────────────────────────────────────────────────
  const [activeModel, setActiveModel] = useState<ActiveModel | null>(null);
  const [running, setRunning] = useState(false);
  const [steps, setSteps] = useState<RunStep[]>([]);
  const [finished, setFinished] = useState(false);
  const [success, setSuccess] = useState<boolean | null>(null);
  const [finishNote, setFinishNote] = useState<string | null>(null);
  const [finalAnswer, setFinalAnswer] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [noProvider, setNoProvider] = useState(false);
  const [providerCompat, setProviderCompat] = useState(false);
  const stepSeq = useRef(0);

  // ── load provider ─────────────────────────────────────────────────────────
  useEffect(() => {
    if (!isDesktop()) return;
    (async () => {
      try {
        const s = await getProviders();
        const prov = s.providers.find((p) => p.id === s.active_provider);
        if (prov && s.active_model) {
          setActiveModel({ provider: prov.name || "（未命名）", model: s.active_model });
        }
      } catch { /* silent */ }
    })();
  }, []);

  // ── load memory items ─────────────────────────────────────────────────────
  const loadMemories = useCallback(async () => {
    if (!isDesktop()) return;
    setLoadingMem(true);
    try {
      const result = await invokeTool<MemoryRecallData>("memory_recall", { query: "" });
      setMemoryItems(result.data.hits ?? []);
    } catch { /* silent */ }
    finally { setLoadingMem(false); }
  }, []);

  useEffect(() => { void loadMemories(); }, [loadMemories]);

  const toggleId = (id: string) =>
    setSelectedIds((prev) => {
      const next = new Set(prev);
      next.has(id) ? next.delete(id) : next.add(id);
      return next;
    });

  const addNewSetting = () => {
    const t = newDraft.trim();
    if (!t) return;
    setNewSettings((prev) => [...prev, t]);
    setNewDraft("");
  };

  // ── simulation ────────────────────────────────────────────────────────────
  const startSimulation = useCallback(async () => {
    const s = scenario.trim();
    if (!s) { toast.err("请先描述要模拟的场景"); return; }
    if (simTypes.length === 0) { toast.err("请至少选择一种推演维度"); return; }

    const selectedMems = memoryItems.filter((m) => selectedIds.has(m.id));
    const goal = buildGoal(s, simTypes, selectedMems, newSettings);

    stepSeq.current = 0;
    setSteps([]); setFinished(false); setSuccess(null);
    setFinishNote(null); setFinalAnswer(null);
    setError(null); setNoProvider(false); setProviderCompat(false);
    setRunning(true);

    const handleStep = (ev: AgentStep) => {
      setSteps((prev) => upsertStep(prev, ev, () => (stepSeq.current += 1)));
      if (ev.phase === "finish") {
        setRunning(false); setFinished(true); setSuccess(ev.success);
        setFinishNote(ev.success
          ? `推演完成（共 ${ev.steps} 步）`
          : `已停止：${ev.reason || "未完成"}（${ev.steps} 步）`);
      }
    };

    try {
      const result = await runGoalLive(
        goal,
        `推演·${simTypes.map((k) => SIM_TYPES.find((t) => t.key === k)!.label).join("+")}`,
        handleStep
      );
      setFinalAnswer(result.outcome.final_answer);
      const saved = result.session.messages.filter(
        (m) => m.tool_call?.name === "memory_save"
      ).length;
      toast.ok(saved > 0 ? `推演完成，已保存 ${saved} 条记录` : "推演结束");
      // Reload memories in case new ones were saved
      if (newSettings.length > 0 && saved > 0) void loadMemories();
    } catch (e) {
      const msg = describeError(e);
      setRunning(false); setError(msg);
      setNoProvider(isNoProviderError(msg));
      setProviderCompat(isProviderCompatibilityError(msg));
      if (!isNoProviderError(msg) && !isProviderCompatibilityError(msg))
        toast.err(`推演失败：${msg}`);
    }
  }, [scenario, simTypes, selectedIds, memoryItems, newSettings, toast, loadMemories]);

  // ── derived ───────────────────────────────────────────────────────────────
  const hasResult = running || steps.length > 0 || finished || error !== null;
  const phase = derivePhase({ running, steps, finished, success, errored: error !== null });
  const wf = workflowView(phase);
  const lastStep = steps.length > 0 ? steps[steps.length - 1].step : 0;
  const toolCount = steps.reduce((n, s) => n + s.toolCalls.length, 0);

  // Group selectable memory items by kind
  const memGroups = SELECTABLE_KINDS
    .map((kind) => ({
      kind,
      label: KIND_LABEL[kind],
      items: memoryItems.filter((m) => m.kind === kind),
    }))
    .filter((g) => g.items.length > 0);

  return (
    <div className="work-content simulate2">

      {/* ── Left ─────────────────────────────────────────────────────────── */}
      <div className="simulate2__left">
        <section className="panel simulate2__input-panel">
          <p className="panel__kicker">世界模拟器</p>
          <h2 className="panel__title">推演</h2>
          <p className="panel__subtitle">
            描述场景，选择参与推演的设定要素，AI 将基于现有设定与世界规则模拟后续事件走向。
          </p>

          {activeModel ? (
            <div className="planning2__model">
              <IconStar size={12} />
              当前模型：{activeModel.provider} · {activeModel.model}
            </div>
          ) : (
            <div className="planning2__no-model">
              <IconProviders size={18} />
              <span>尚未配置模型。<button className="link-btn" onClick={onOpenSettings}>前往设置</button></span>
            </div>
          )}

          {/* Scenario */}
          <div className="simulate2__field">
            <label className="input-field">
              <span className="input-field__label">模拟场景</span>
              <textarea
                className="textarea"
                value={scenario}
                onChange={(e) => setScenario(e.target.value)}
                placeholder={"例如：主角在第三章拒绝了导师的邀请……\n例如：反派提前得知了主角的计划……"}
                rows={4}
                spellCheck={false}
              />
            </label>
          </div>

          {/* Simulation types */}
          <div className="simulate2__section-label">推演维度</div>
          <div className="simulate2__types">
            {SIM_TYPES.map((t) => {
              const active = simTypes.includes(t.key);
              return (
                <button
                  key={t.key}
                  className={`simulate2__type-btn${active ? " is-active" : ""}`}
                  onClick={() => {
                    if (running) return;
                    setSimTypes((prev) =>
                      prev.includes(t.key)
                        ? prev.length > 1 ? prev.filter((k) => k !== t.key) : prev
                        : [...prev, t.key]
                    );
                  }}
                  disabled={running}
                >
                  <div className="simulate2__type-row">
                    <div className={`simulate2__checkbox${active ? " is-checked" : ""}`}>
                      {active && <svg width="10" height="8" viewBox="0 0 10 8" fill="none">
                        <path d="M1 4L3.5 6.5L9 1" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"/>
                      </svg>}
                    </div>
                    <span className="simulate2__type-label">{t.label}</span>
                  </div>
                  <span className="simulate2__type-blurb">{t.blurb}</span>
                </button>
              );
            })}
          </div>

          {/* Existing memory selection */}
          <div className="simulate2__section-label" style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
            <span>参与要素 {selectedIds.size > 0 && <span className="simulate2__badge">{selectedIds.size}</span>}</span>
            <button
              className="btn btn--ghost btn--xs"
              onClick={() => void loadMemories()}
              disabled={loadingMem}
              title="刷新设定列表"
            >
              {loadingMem ? <Spinner size={12} /> : <IconRefresh size={12} />}
            </button>
          </div>

          {memGroups.length === 0 && !loadingMem && (
            <p className="simulate2__hint">暂无设定，先在「策划」中生成，或在下方手动添加。</p>
          )}

          {memGroups.map((g) => (
            <div key={g.kind} className="simulate2__mem-group">
              <div className="simulate2__mem-group-label">
                <span>{g.label}</span>
                <button
                  className="simulate2__group-toggle"
                  onClick={() => {
                    const allSelected = g.items.every((m) => selectedIds.has(m.id));
                    setSelectedIds((prev) => {
                      const next = new Set(prev);
                      g.items.forEach((m) => allSelected ? next.delete(m.id) : next.add(m.id));
                      return next;
                    });
                  }}
                >
                  {g.items.every((m) => selectedIds.has(m.id)) ? "取消全选" : "全选"}
                </button>
              </div>
              <div className="simulate2__mem-chips">
                {g.items.map((m) => (
                  <button
                    key={m.id}
                    className={`simulate2__chip${selectedIds.has(m.id) ? " is-selected" : ""}`}
                    onClick={() => toggleId(m.id)}
                    title={m.summary}
                  >
                    {m.title}
                  </button>
                ))}
              </div>
            </div>
          ))}

          {/* New settings */}
          <div className="simulate2__section-label">新增设定</div>
          <p className="simulate2__hint">添加目前记忆库中没有的设定，推演后 AI 会自动分类录入。</p>

          <div className="simulate2__new-setting-input">
            <input
              className="input"
              placeholder="输入新设定内容，例如：林惊羽获得了风系灵根…"
              value={newDraft}
              onChange={(e) => setNewDraft(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); addNewSetting(); } }}
            />
            <button
              className="btn btn--primary btn--icon"
              onClick={addNewSetting}
              disabled={!newDraft.trim()}
            >
              <IconPlus size={15} />
            </button>
          </div>

          {newSettings.length > 0 && (
            <div className="simulate2__new-list">
              {newSettings.map((s, i) => (
                <div key={i} className="simulate2__new-item">
                  <span className="simulate2__new-text">{s}</span>
                  <button
                    className="simulate2__new-del"
                    onClick={() => setNewSettings((prev) => prev.filter((_, j) => j !== i))}
                  >
                    <IconClose size={12} />
                  </button>
                </div>
              ))}
            </div>
          )}

          <button
            className="btn btn--primary"
            style={{ width: "100%", marginTop: "var(--sp-5)" }}
            onClick={() => void startSimulation()}
            disabled={running || !scenario.trim() || !activeModel}
          >
            {running ? <><Spinner size={15} /> 推演中…</> : <><IconSimulate size={15} /> 开始推演</>}
          </button>
        </section>
      </div>

      {/* ── Right ────────────────────────────────────────────────────────── */}
      <div className="simulate2__right">
        {!hasResult && (
          <div className="simulate2__empty">
            <IconSimulate size={48} />
            <p>填写场景，选择要素，点击「开始推演」</p>
            <p className="simulate2__empty-hint">
              勾选已有设定直接参与推演，添加新设定推演后自动录入
            </p>
          </div>
        )}

        {hasResult && (
          <section className="panel simulate2__result">
            {(noProvider || providerCompat) ? (
              <div className="studio2__notice-err"><IconWarn size={14} /> {error}</div>
            ) : (
              <>
                {(running || steps.length > 0) && (
                  <div className="agent-console">
                    <WorkflowSteps stages={SIMULATE_STAGES} current={wf.current} state={wf.state} />
                    <WorkStatus phase={phase} step={lastStep} toolCount={toolCount} />
                  </div>
                )}
                {steps.length > 0 && (
                  <AgentFeed steps={steps} running={running} pendingText="AI 正在推演下一个事件节点…" />
                )}
                {!running && finalAnswer && (
                  <div className="simulate2__answer">
                    <p className="simulate2__answer-label">推演摘要</p>
                    <div className="simulate2__answer-text">{finalAnswer}</div>
                  </div>
                )}
                {!running && error && !noProvider && !providerCompat && (
                  <div className="studio2__notice-err"><IconWarn size={14} /> {error}</div>
                )}
                {finishNote && (
                  <div className={`planning2__finish${success ? "" : " planning2__finish--warn"}`}>
                    <IconCheck size={14} /> {finishNote}
                  </div>
                )}
              </>
            )}
          </section>
        )}
      </div>
    </div>
  );
}
