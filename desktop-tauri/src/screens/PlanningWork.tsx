// 策划 — Planning / story-bible stage.
// 构思 (concept) → 生成设定 (guided agent actions that memory_save world/
// characters/outline/foreshadow). Wires the real runGoalLive loop.

import {
  useCallback,
  useEffect,
  useState,
  type ReactNode,
} from "react";
import { Spinner } from "../components/Spinner";
import {
  IconBrush,
  IconCheck,
  IconWarn,
  IconUser,
  IconMountain,
  IconThread,
  IconScroll,
  IconProviders,
  IconStar,
  IconSeed,
} from "../components/icons";
import { describeError, isDesktop } from "../lib/core";
import { useToast } from "../components/Toast";
import {
  runGoalLive,
  type AgentStep,
} from "../lib/studio";
import { getProviders } from "../lib/providers";
import {
  derivePhase,
  workflowView,
  isNoProviderError,
  isProviderCompatibilityError,
  upsertStep,
  PLAN_STAGES,
  type RunStep,
} from "../lib/agentRun";
import WorkStatus from "../components/agent/WorkStatus";
import WorkflowSteps from "../components/agent/WorkflowSteps";
import AgentFeed from "../components/agent/AgentFeed";

interface PlanningWorkProps {
  onOpenSettings?: () => void;
}

interface PlanAction {
  key: string;
  label: string;
  blurb: string;
  Icon: (p: { size?: number }) => ReactNode;
  title: string;
  goal: (concept: string) => string;
}

const ACTIONS: PlanAction[] = [
  {
    key: "worldbuilding",
    label: "世界观",
    blurb: "构筑背景、规则与传说的根基",
    Icon: IconMountain,
    title: "世界观设定",
    goal: (concept) =>
      `你是这部同人小说的策划助手。作者的构思如下：\n\n「${concept}」\n\n` +
      `## ⚠️ 核心约束（违反任一条视为失败）\n` +
      `1. **必须调用工具**：每构思出一条设定，立即使用 memory_save 工具保存，不得仅在回复中描述\n` +
      `2. **参数必须正确**：kind="worldbuilding"（固定值，不得更改），title=设定名称，content=完整内容\n` +
      `3. **最低数量**：至少保存 3 条设定，否则视为未完成任务\n` +
      `4. **禁止偏离**：严格围绕作者构思，不得自行添加无关元素\n\n` +
      `## 你的任务\n` +
      `根据上述构思，系统地构建这部作品的世界观与设定。具体包括：\n` +
      `1. 时代/地域背景（例如：战国末年、赛博朋克都市、修仙界等）\n` +
      `2. 核心规则体系（力量体系、社会结构、重要组织/门派）\n` +
      `3. 独特的设定亮点（能让这个故事与众不同的元素）\n` +
      `4. 可供情节利用的张力点（冲突源、矛盾点）\n\n` +
      `## 执行流程\n` +
      `1. 思考第一条设定 → 立即调用 memory_save 保存\n` +
      `2. 思考第二条设定 → 立即调用 memory_save 保存\n` +
      `3. 思考第三条设定 → 立即调用 memory_save 保存\n` +
      `4. （可选）继续思考并保存更多设定\n` +
      `5. 完成后回复"已完成 X 条世界观设定的保存"\n\n` +
      `现在开始执行。记住：每一条都必须调用 memory_save，仅描述不调用工具视为无效。`,
  },
  {
    key: "character",
    label: "人物",
    blurb: "立起主角与群像的性情与关系",
    Icon: IconUser,
    title: "人物设定",
    goal: (concept) =>
      `你是这部同人小说的策划助手。作者的构思如下：\n\n「${concept}」\n\n` +
      `## ⚠️ 核心约束（违反任一条视为失败）\n` +
      `1. **必须调用工具**：每设计完一个角色，立即使用 memory_save 工具保存，不得仅在回复中描述\n` +
      `2. **参数必须正确**：kind="character"（固定值），title=角色名，content=完整角色设定\n` +
      `3. **最低数量**：至少保存 3 个角色（主角+配角），否则视为未完成任务\n` +
      `4. **禁止偏离**：严格围绕作者构思，角色设定必须与构思契合\n\n` +
      `## 你的任务\n` +
      `根据上述构思，设计这部作品的主要人物。具体包括：\n` +
      `1. 主角的设定（身份、性格、动机、成长弧线）\n` +
      `2. 关键配角的设定（至少 2-3 个）\n` +
      `3. 人物之间的关系网（盟友/对手/情感纽带）\n` +
      `4. 每个角色与故事主题的呼应\n\n` +
      `## 执行流程\n` +
      `1. 设计主角 → 立即调用 memory_save 保存\n` +
      `2. 设计第一个配角 → 立即调用 memory_save 保存\n` +
      `3. 设计第二个配角 → 立即调用 memory_save 保存\n` +
      `4. （可选）继续设计更多角色并保存\n` +
      `5. 完成后回复"已完成 X 个角色的保存"\n\n` +
      `现在开始执行。记住：每一个角色都必须调用 memory_save，仅描述不调用工具视为无效。`,
  },
  {
    key: "outline",
    label: "大纲",
    blurb: "铺排起承转合的故事骨架",
    Icon: IconScroll,
    title: "故事大纲",
    goal: (concept) =>
      `你是这部同人小说的策划助手。作者的构思如下：\n\n「${concept}」\n\n` +
      `## ⚠️ 核心约束（违反任一条视为失败）\n` +
      `1. **必须调用工具**：构思完成后，使用 memory_save 工具保存大纲，不得仅在回复中描述\n` +
      `2. **参数必须正确**：kind="outline"（固定值），title="故事大纲"，content=完整大纲\n` +
      `3. **内容完整**：必须包含核心冲突、主线脉络、分阶段情节节点、关键转折\n` +
      `4. **禁止偏离**：严格围绕作者构思，不得自行改变故事方向\n\n` +
      `## 你的任务\n` +
      `根据上述构思，拟定这部作品的故事大纲。具体包括：\n` +
      `1. 核心冲突（故事的主要矛盾是什么）\n` +
      `2. 主线脉络（故事如何展开）\n` +
      `3. 分阶段情节节点：\n` +
      `   - 开篇（引入/铺垫）\n` +
      `   - 发展（矛盾升级）\n` +
      `   - 高潮（冲突爆发）\n` +
      `   - 结局（收束/余韵）\n` +
      `4. 关键转折点\n\n` +
      `## 执行流程\n` +
      `1. 思考并构建完整大纲（包含上述所有要素）\n` +
      `2. 调用 memory_save 工具保存\n` +
      `3. 回复"已完成故事大纲的保存"\n\n` +
      `现在开始执行。记住：必须调用 memory_save 保存，仅描述不调用工具视为无效。`,
  },
  {
    key: "foreshadow",
    label: "伏笔",
    blurb: "埋下草蛇灰线的线索与回响",
    Icon: IconThread,
    title: "伏笔设计",
    goal: (concept) =>
      `你是这部同人小说的策划助手。作者的构思如下：\n\n「${concept}」\n\n` +
      `## ⚠️ 核心约束（违反任一条视为失败）\n` +
      `1. **必须调用工具**：每设计完一处伏笔，立即使用 memory_save 工具保存，不得仅在回复中描述\n` +
      `2. **参数必须正确**：kind="foreshadow"（固定值），title=伏笔概括，content=埋设+回收设计\n` +
      `3. **最低数量**：至少保存 2 处伏笔，否则视为未完成任务\n` +
      `4. **禁止偏离**：伏笔必须服务于作者构思的故事，不得无关\n\n` +
      `## 你的任务\n` +
      `根据上述构思，设计这部作品中可以埋设的伏笔与线索。具体包括：\n` +
      `1. 每处伏笔的名称/主题\n` +
      `2. 埋设之处（在故事的哪个阶段埋下）\n` +
      `3. 回收方式（日后如何呼应/揭晓）\n` +
      `4. 作用（增强什么效果：悬念/反转/情感冲击等）\n\n` +
      `## 执行流程\n` +
      `1. 设计第一处伏笔 → 立即调用 memory_save 保存\n` +
      `2. 设计第二处伏笔 → 立即调用 memory_save 保存\n` +
      `3. （可选）继续设计更多伏笔并保存\n` +
      `4. 完成后回复"已完成 X 处伏笔的保存"\n\n` +
      `现在开始执行。记住：每一处伏笔都必须调用 memory_save，仅描述不调用工具视为无效。`,
  },
];

interface ActionRun {
  action: PlanAction;
  running: boolean;
  steps: RunStep[];
  finishNote: string | null;
  success: boolean | null;
  error: string | null;
  noProvider: boolean;
  providerCompat: boolean;
  finalAnswer: string | null;
  savedCount: number;
}

interface ActiveModel {
  provider: string;
  model: string;
}

export default function PlanningWork({ onOpenSettings }: PlanningWorkProps) {
  const toast = useToast();
  const [concept, setConcept] = useState("");
  const [activeModel, setActiveModel] = useState<ActiveModel | null>(null);
  const [runs, setRuns] = useState<Map<string, ActionRun>>(new Map());

  // Check provider on mount
  useEffect(() => {
    if (!isDesktop()) return;
    (async () => {
      try {
        const s = await getProviders();
        const prov = s.providers.find((p) => p.id === s.active_provider);
        if (prov && s.active_model) {
          setActiveModel({ provider: prov.name || "（未命名）", model: s.active_model });
        } else {
          setActiveModel(null);
        }
      } catch {
        /* silent */
      }
    })();
  }, []);

  const startAction = useCallback(
    async (action: PlanAction) => {
      const c = concept.trim();
      if (!c) {
        toast.err("请先填写「构思」再生成设定");
        return;
      }
      const stepSeq = { current: 0 };
      const handleStep = (s: AgentStep) => {
        setRuns((prev) => {
          const cur = prev.get(action.key);
          if (!cur) return prev;
          const next = new Map(prev);
          next.set(action.key, {
            ...cur,
            steps: upsertStep(cur.steps, s, () => (stepSeq.current += 1)),
          });
          return next;
        });
        if (s.phase === "finish") {
          setRuns((prev) => {
            const cur = prev.get(action.key);
            if (!cur) return prev;
            const next = new Map(prev);
            next.set(action.key, {
              ...cur,
              running: false,
              success: s.success,
              finishNote: s.success
                ? `生成完成（共 ${s.steps} 步）`
                : `已停止：${s.reason || "未完成"}（共 ${s.steps} 步）`,
            });
            return next;
          });
        }
      };

      setRuns((prev) => {
        const next = new Map(prev);
        next.set(action.key, {
          action,
          running: true,
          steps: [],
          finishNote: null,
          success: null,
          error: null,
          noProvider: false,
          providerCompat: false,
          finalAnswer: null,
          savedCount: 0,
        });
        return next;
      });

      try {
        const result = await runGoalLive(action.goal(c), action.title, handleStep);
        // Check if any memory_save was actually called
        const toolCalls = result.session.messages.filter(
          (m) => m.tool_call && m.tool_call.name === "memory_save"
        );
        const savedCount = toolCalls.length;

        setRuns((prev) => {
          const cur = prev.get(action.key);
          if (!cur) return prev;
          const next = new Map(prev);
          next.set(action.key, {
            ...cur,
            running: false,
            success: savedCount > 0,
            savedCount,
            finalAnswer: result.outcome.final_answer,
            finishNote: savedCount > 0
              ? `生成完成（已保存 ${savedCount} 条，共 ${result.outcome.steps} 步）`
              : `生成结束，但未保存任何内容（共 ${result.outcome.steps} 步）`,
          });
          return next;
        });

        if (savedCount === 0) {
          toast.info(`${action.label}生成结束，但未保存任何内容（AI 可能没理解任务）`);
        } else {
          toast.ok(`${action.label}生成完成（已保存 ${savedCount} 条）`);
        }
      } catch (e) {
        const msg = describeError(e);
        console.error(`[PlanningWork] ${action.key} failed:`, e);
        setRuns((prev) => {
          const cur = prev.get(action.key);
          if (!cur) return prev;
          const next = new Map(prev);
          next.set(action.key, {
            ...cur,
            running: false,
            error: msg,
            noProvider: isNoProviderError(msg),
            providerCompat: isProviderCompatibilityError(msg),
            savedCount: 0,
            finalAnswer: null,
          });
          return next;
        });
        if (isNoProviderError(msg) || isProviderCompatibilityError(msg)) {
          /* handled in UI */
        } else {
          toast.err(`${action.label}生成失败：${msg}`);
        }
      }
    },
    [concept, toast],
  );

  return (
    <div className="work-content planning2">
      {/* Single column: 构思输入 */}
      <div className="planning2__left">
        <section className="panel planning2__concept-panel">
          <p className="panel__kicker">第一幕 · 策划</p>
          <h2 className="panel__title">构思</h2>
          <p className="panel__subtitle">
            写下你的核心构思，然后到右侧生成正式的世界观、人物、大纲与伏笔设定。需要脑暴？切换到「探讨」标签与 AI 自由对话。
          </p>

          {activeModel ? (
            <div className="planning2__model">
              <IconStar size={12} />
              当前模型：{activeModel.provider} · {activeModel.model}
            </div>
          ) : (
            <div className="planning2__no-model">
              <IconProviders size={18} />
              <span>
                尚未配置模型。
                <button className="link-btn" onClick={onOpenSettings}>
                  前往设置
                </button>
              </span>
            </div>
          )}

          <div className="planning2__concept-field">
            <label className="input-field">
              <span className="input-field__label">
                <IconSeed size={14} />
                构思
              </span>
              <textarea
                className="textarea"
                value={concept}
                onChange={(e) => setConcept(e.target.value)}
                placeholder="例如：鬼灭之刃同人，聚焦杏寿郎与猗窝座的宿命羁绊，重写无限列车与后日谈……"
                rows={5}
                spellCheck={false}
              />
            </label>
          </div>
        </section>
      </div>

      {/* Right column: 生成设定 */}
      <div className="planning2__right">
        <section className="panel">
          <div className="planning2__actions-head">
            <h3 className="planning2__actions-title">生成设定</h3>
            <p className="planning2__actions-subtitle">
              把上面的构思与探讨成果具象为正式的世界观/人物/大纲/伏笔，写入记忆库供后续创作查阅。
            </p>
          </div>
          <div className="planning2__cards">
            {ACTIONS.map((a) => {
              const run = runs.get(a.key);
              const Icon = a.Icon;
              return (
                <div key={a.key} className="planning2__card">
                  <div className="planning2__card-head">
                    <span className="planning2__card-icon">
                      <Icon size={20} />
                    </span>
                    <div>
                      <h4 className="planning2__card-title">{a.label}</h4>
                      <p className="planning2__card-blurb">{a.blurb}</p>
                    </div>
                  </div>
                  <button
                    className="btn btn--primary btn--sm"
                    onClick={() => void startAction(a)}
                    disabled={run?.running || !concept.trim() || !activeModel}
                  >
                    {run?.running ? (
                      <>
                        <Spinner size={14} />
                        生成中…
                      </>
                    ) : (
                      <>
                        <IconBrush size={14} />
                        生成
                      </>
                    )}
                  </button>
                </div>
              );
            })}
          </div>
        </section>

        {Array.from(runs.values())
          .filter((r) => r.running || r.steps.length > 0 || r.error)
          .map((r) => {
            const phase = derivePhase({
              running: r.running,
              steps: r.steps,
              finished: r.finishNote !== null,
              success: r.success,
              errored: r.error !== null,
            });
            const wf = workflowView(phase);
            const lastStepNo = r.steps.length > 0 ? r.steps[r.steps.length - 1].step : 0;
            const toolCount = r.steps.reduce((n, s) => n + s.toolCalls.length, 0);
            return (
              <section key={r.action.key} className="panel planning2__run">
                <div className="planning2__run-head">
                  <span className="planning2__run-icon">
                    <r.action.Icon size={18} />
                  </span>
                  <h3 className="planning2__run-title">{r.action.title}</h3>
                </div>
                {r.noProvider || r.providerCompat ? (
                  <div className="studio2__notice-err">
                    <IconWarn size={14} /> {r.error}
                  </div>
                ) : (
                  <>
                    {(r.running || r.steps.length > 0) && (
                      <div className="agent-console">
                        <WorkflowSteps stages={PLAN_STAGES} current={wf.current} state={wf.state} />
                        <WorkStatus phase={phase} step={lastStepNo} toolCount={toolCount} />
                      </div>
                    )}
                    {r.steps.length > 0 && <AgentFeed steps={r.steps} running={r.running} pendingText="AI 正在构思下一条…" />}
                    {!r.running && r.finalAnswer && (
                      <div className="planning2__final-answer">
                        <p className="planning2__final-label">AI 最终回复：</p>
                        <div className="planning2__final-text">{r.finalAnswer}</div>
                      </div>
                    )}
                    {!r.running && r.error && <div className="studio2__notice-err"><IconWarn size={14} /> {r.error}</div>}
                    {r.finishNote && (
                      <div className={`planning2__finish ${r.success ? "" : "planning2__finish--warn"}`}>
                        <IconCheck size={14} /> {r.finishNote}
                        {r.savedCount === 0 && r.success === false && (
                          <p className="planning2__finish-hint">
                            提示：AI 完成了思考，但没有调用 memory_save 工具保存内容。
                            这可能是因为：① 模型不支持工具调用 ② 回复被截断 ③ 理解偏差。
                            请检查上方的"AI 最终回复"，或尝试切换更强的模型。
                          </p>
                        )}
                      </div>
                    )}
                  </>
                )}
              </section>
            );
          })}
      </div>
    </div>
  );
}
