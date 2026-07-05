// 策划 — the planning / story-bible stage that comes BEFORE writing chapters.
// Two movements:
//   1) 构思 (Concept): describe the work — 原作 / 同人设定 / 方向 / 基调.
//   2) 立稿 / 生成设定 (Generate the story bible): guided action cards
//      (世界观 / 人物 / 大纲 / 伏笔). Each runs the real agent loop via
//      runGoalLive with a tailored goal that embeds the 构思 text and tells the
//      AI to memory_save with the right kind — so the result shows up in the
//      人物 / 伏笔 / 设定 screens. While running we surface the Agent Console
//      (工作流程 + 工作状态 + 模型推理/工具调用 feed) with live token streaming.
//
// Free-form brainstorming with the AI now lives on its own 「探讨」screen.
//
// A panel toolbar shows the active model at a glance; when none is configured a
// calm callout offers a jump to 供应商.

import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";
import Panel from "../components/Panel";
import { Spinner } from "../components/Spinner";
import {
  IconSeed,
  IconBrush,
  IconCheck,
  IconWarn,
  IconInfo,
  IconUser,
  IconMountain,
  IconThread,
  IconScroll,
  IconProviders,
  IconStar,
  IconTools,
  BrushStroke,
} from "../components/icons";
import { describeError, isDesktop } from "../lib/core";
import { useToast } from "../components/Toast";
import { runGoalLive, type AgentStep } from "../lib/studio";
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
import type { ScreenId } from "../lib/screens";

interface PlanningScreenProps {
  onNavigate?: (id: ScreenId) => void;
}

// ---- the guided story-bible actions ----------------------------------------

interface PlanAction {
  key: string;
  label: string;
  blurb: string;
  Icon: (p: { size?: number }) => ReactNode;
  kind: string;
  target: ScreenId;
  targetLabel: string;
  title: string;
  goal: (concept: string) => string;
}

const ACTIONS: PlanAction[] = [
  {
    key: "worldbuilding",
    label: "世界观",
    blurb: "构筑背景、规则与传说的根基",
    Icon: IconMountain,
    kind: "worldbuilding",
    target: "settings",
    targetLabel: "设定",
    title: "世界观设定",
    goal: (concept) =>
      `你是这部同人小说的策划。下面是作者的构思：\n「${concept}」\n\n` +
      `请据此系统地构建这部作品的世界观与设定：时代/地域背景、核心规则（力量体系/` +
      `社会结构/重要组织等）、独特的设定亮点与可供后续情节利用的张力。\n\n` +
      `要求：把每一条成形的世界观/设定，使用 memory_save 工具逐条保存，kind 设为 ` +
      `"worldbuilding"（或 "setting"），并配以清晰的标题与正文。先思考再调用工具，` +
      `条目之间彼此呼应、避免重复。`,
  },
  {
    key: "character",
    label: "人物",
    blurb: "立起主角与群像的性情与关系",
    Icon: IconUser,
    kind: "character",
    target: "characters",
    targetLabel: "人物",
    title: "人物设定",
    goal: (concept) =>
      `你是这部同人小说的策划。下面是作者的构思：\n「${concept}」\n\n` +
      `请据此设计这部作品的主要人物：为每个角色给出身份、性格、动机、成长弧线，以及` +
      `他/她与其他角色之间的关系与张力。至少覆盖主角与若干关键配角。\n\n` +
      `要求：把每一个成形的人物，使用 memory_save 工具单独保存，kind 设为 ` +
      `"character"，标题用角色名，正文写清其设定。先思考再调用工具，人物之间关系自洽。`,
  },
  {
    key: "outline",
    label: "大纲",
    blurb: "铺排起承转合的故事骨架",
    Icon: IconScroll,
    kind: "outline",
    target: "checkpoints",
    targetLabel: "记忆库",
    title: "故事大纲",
    goal: (concept) =>
      `你是这部同人小说的策划。下面是作者的构思：\n「${concept}」\n\n` +
      `请据此拟定这部作品的故事大纲：核心冲突、主线脉络，以及分阶段（如开篇/发展/` +
      `高潮/结局）的关键情节节点与转折，使其起承转合、节奏分明。\n\n` +
      `要求：把成形的大纲，使用 memory_save 工具保存（可整体一条、或按章节/阶段分条），` +
      `kind 设为 "outline"，标题点明阶段或章回，正文写清情节走向。先思考再调用工具。`,
  },
  {
    key: "foreshadow",
    label: "伏笔",
    blurb: "埋下草蛇灰线的线索与回响",
    Icon: IconThread,
    kind: "foreshadow",
    target: "foreshadow",
    targetLabel: "伏笔",
    title: "伏笔设计",
    goal: (concept) =>
      `你是这部同人小说的策划。下面是作者的构思：\n「${concept}」\n\n` +
      `请据此设计这部作品中可以埋设的伏笔与线索：每一处伏笔写明「埋设之处」与「日后` +
      `如何回收/呼应」，让草蛇灰线、前后照应，增强阅读的回味。\n\n` +
      `要求：把每一处成形的伏笔，使用 memory_save 工具单独保存，kind 设为 ` +
      `"foreshadow"，标题概括线索，正文写清埋设与回收。先思考再调用工具。`,
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
}

/** A compact summary of the active model, loaded from the provider store. */
interface ActiveModel {
  provider: string;
  model: string;
}

export default function PlanningScreen({ onNavigate }: PlanningScreenProps) {
  const toast = useToast();

  // active-model awareness
  const [active, setActive] = useState<ActiveModel | null>(null);
  const [providerChecked, setProviderChecked] = useState(false);

  // 1) 构思
  const [concept, setConcept] = useState("");
  const conceptReady = concept.trim().length > 0;

  // 2) 立稿 / 生成设定
  const [run, setRun] = useState<ActionRun | null>(null);
  const stepSeq = useRef(0);
  const liveTailRef = useRef<HTMLDivElement>(null);
  const pendingDeltaRef = useRef<AgentStep | null>(null);
  const rafRef = useRef<number | null>(null);

  const loadActive = useCallback(async () => {
    if (!isDesktop()) {
      setProviderChecked(true);
      return;
    }
    try {
      const s = await getProviders();
      const prov = s.providers.find((p) => p.id === s.active_provider);
      if (prov && s.active_model) {
        setActive({ provider: prov.name || "（未命名）", model: s.active_model });
      } else {
        setActive(null);
      }
    } catch {
      setActive(null);
    } finally {
      setProviderChecked(true);
    }
  }, []);

  useEffect(() => {
    void loadActive();
  }, [loadActive]);

  // auto-scroll the live action feed to the newest step
  useEffect(() => {
    if (run?.running) {
      liveTailRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
    }
  }, [run?.steps, run?.running]);

  // ---- guided action run ---------------------------------------------------
  const handleStepNow = useCallback((s: AgentStep) => {
    if (s.phase === "delta" || s.phase === "model") {
      setRun((prev) =>
        prev
          ? {
              ...prev,
              steps: upsertStep(prev.steps, s, () => (stepSeq.current += 1)),
            }
          : prev,
      );
    } else if (s.phase === "finish") {
      setRun((prev) =>
        prev
          ? {
              ...prev,
              success: s.success,
              finishNote: s.success
                ? `已生成（共 ${s.steps} 步）`
                : `已停止：${s.reason || "未完成"}（共 ${s.steps} 步）`,
            }
          : prev,
      );
    }
  }, []);

  const flushPendingDelta = useCallback(() => {
    rafRef.current = null;
    const pending = pendingDeltaRef.current;
    pendingDeltaRef.current = null;
    if (pending) handleStepNow(pending);
  }, [handleStepNow]);

  const handleStep = useCallback(
    (s: AgentStep) => {
      if (s.phase !== "delta") {
        if (pendingDeltaRef.current) flushPendingDelta();
        handleStepNow(s);
        return;
      }
      const cur = pendingDeltaRef.current;
      pendingDeltaRef.current =
        cur && cur.phase === "delta" && cur.step === s.step
          ? { ...cur, delta: cur.delta + s.delta }
          : s;
      if (rafRef.current == null) {
        rafRef.current = window.requestAnimationFrame(flushPendingDelta);
      }
    },
    [flushPendingDelta, handleStepNow],
  );

  useEffect(() => {
    return () => {
      if (rafRef.current != null) {
        window.cancelAnimationFrame(rafRef.current);
      }
    };
  }, []);

  const runAction = useCallback(
    async (action: PlanAction) => {
      if (run?.running) return;
      const c = concept.trim();
      if (!c) {
        toast.err("请先在「构思」里写下你的想法");
        return;
      }
      stepSeq.current = 0;
      setRun({
        action,
        running: true,
        steps: [],
        finishNote: null,
        success: null,
        error: null,
        noProvider: false,
        providerCompat: false,
      });
      try {
        await runGoalLive(action.goal(c), action.title, handleStep);
        toast.ok(`${action.label}已生成`);
        if (!active) void loadActive();
      } catch (e) {
        const msg = describeError(e);
        const noProvider = isNoProviderError(msg);
        const providerCompat = !noProvider && isProviderCompatibilityError(msg);
        setRun((prev) =>
          prev ? { ...prev, error: msg, noProvider, providerCompat } : prev,
        );
        if (!noProvider && !providerCompat) toast.err(`生成失败：${msg}`);
      } finally {
        setRun((prev) => (prev ? { ...prev, running: false } : prev));
      }
    },
    [run?.running, concept, handleStep, toast, active, loadActive],
  );

  const closeRun = useCallback(() => {
    setRun((prev) => (prev && prev.running ? prev : null));
  }, []);

  const running = run?.running ?? false;
  const actionsDisabled = running || !conceptReady;
  const showProviderHint = providerChecked && !active && isDesktop();

  const toolbar = (
    <div className="toolbar plan-toolbar">
      <span className="toolbar__label">当前模型</span>
      {active ? (
        <span className="chip chip--accent">
          <IconStar size={11} />
          {active.provider} · {active.model}
        </span>
      ) : (
        <span className="chip">{providerChecked ? "尚未选用" : "检查中…"}</span>
      )}
      <button
        className="plan-toolbar__link"
        onClick={() => onNavigate?.("providers")}
        title="前往供应商配置"
      >
        <IconProviders size={13} />
        {active ? "切换 / 管理" : "去配置"}
      </button>
    </div>
  );

  return (
    <Panel
      title="策划"
      en="Planning"
      subtitle="谋篇于落笔之前 · 立构思、生成世界观与人物的故事蓝本"
      toolbar={toolbar}
    >
      <div className="scroll-area planning">
        {showProviderHint && (
          <div className="callout callout--accent plan-provider-hint">
            <span className="callout__icon">
              <IconProviders size={20} />
            </span>
            <div className="callout__main">
              <h4 className="callout__title">还没有启用模型</h4>
              <p className="callout__text">
                配置一个供应商（填入 API Key 与模型）后，即可据你的构思生成世界观、
                人物等故事蓝本。
              </p>
            </div>
            <button
              className="btn btn--primary callout__action"
              onClick={() => onNavigate?.("providers")}
            >
              <IconProviders size={15} />
              去配置供应商
            </button>
          </div>
        )}

        {/* ---- 1) 构思 ---- */}
        <section className="plan-block">
          <PlanBlockHead
            step={1}
            Icon={IconSeed}
            title="构思"
            sub="在动笔之前，先在这里勾勒这部作品：原作、同人设定、想写的方向与基调。它会作为下面「生成设定」的依据。想边聊边理，可去「探讨」。"
          />
          <textarea
            className="textarea plan-concept"
            value={concept}
            onChange={(e) => setConcept(e.target.value)}
            placeholder={
              "例如：基于《某部原作》的同人。想写主角 A 与 B 在架空王朝下的相遇与抉择，" +
              "基调偏古典、克制而细腻，侧重权谋与情义的拉扯……"
            }
            spellCheck={false}
          />
          <div className="plan-concept__foot">
            <p className="field__hint">
              <IconInfo size={12} className="field__hint-icon" />
              这一步塑形，后续 AI 会围绕你的构思展开。写得越具体，蓝本越贴合你的设想。
            </p>
            <span
              className={`plan-concept__count${conceptReady ? " is-ready" : ""}`}
            >
              {conceptReady ? (
                <>
                  <IconCheck size={12} />
                  {concept.trim().length} 字
                </>
              ) : (
                "待落墨"
              )}
            </span>
          </div>
        </section>

        {/* ---- 2) 立稿 / 生成设定 ---- */}
        <section className="plan-block">
          <PlanBlockHead
            step={2}
            Icon={IconBrush}
            title="立稿 · 生成设定"
            sub="选一项让 AI 据你的构思展开，并写入记忆库——随后可在对应的 设定 / 人物 / 伏笔 中查看。"
          />

          {!conceptReady && (
            <div className="banner banner--info plan-gate">
              <IconInfo size={16} />
              先在上面的「构思」里写下你的想法，再来生成蓝本。
            </div>
          )}

          <div className="plan-actions">
            {ACTIONS.map((a) => {
              const isCurrent = run?.action.key === a.key;
              const isThisRunning = isCurrent && running;
              return (
                <button
                  key={a.key}
                  type="button"
                  className={`plan-action-card${isCurrent ? " is-current" : ""}`}
                  onClick={() => void runAction(a)}
                  disabled={actionsDisabled}
                  title={
                    !conceptReady
                      ? "请先填写构思"
                      : running
                        ? "正在生成，请稍候"
                        : `生成${a.label}`
                  }
                >
                  <span className="plan-action-card__mark">
                    {isThisRunning ? <Spinner size={18} /> : <a.Icon size={20} />}
                  </span>
                  <span className="plan-action-card__label">{a.label}</span>
                  <span className="plan-action-card__blurb">{a.blurb}</span>
                  <span className="plan-action-card__to">
                    <a.Icon size={11} />
                    存入 {a.targetLabel}
                  </span>
                </button>
              );
            })}
          </div>

          {run && (
            <PlanRunFeed
              run={run}
              liveTailRef={liveTailRef}
              onClose={closeRun}
              onNavigate={onNavigate}
            />
          )}
        </section>
      </div>
    </Panel>
  );
}

// ---- a numbered, brush-marked movement header (shared by the blocks) --------

function PlanBlockHead({
  step,
  Icon,
  title,
  sub,
  action,
}: {
  step: number;
  Icon: (p: { size?: number }) => ReactNode;
  title: string;
  sub: string;
  action?: ReactNode;
}) {
  return (
    <div className="plan-block__head">
      <span className="plan-block__mark">
        <Icon size={16} />
        <span className="plan-block__step">{step}</span>
      </span>
      <div className="plan-block__titles">
        <h3 className="plan-block__title">{title}</h3>
        <p className="plan-block__sub">{sub}</p>
      </div>
      {action}
    </div>
  );
}

// ---- the live agent-step feed for a guided action (Agent Console) ------------

function PlanRunFeed({
  run,
  liveTailRef,
  onClose,
  onNavigate,
}: {
  run: ActionRun;
  liveTailRef: RefObject<HTMLDivElement | null>;
  onClose: () => void;
  onNavigate?: (id: ScreenId) => void;
}) {
  const {
    action,
    running,
    steps,
    finishNote,
    success,
    error,
    noProvider,
    providerCompat,
  } = run;
  const done = !running && (finishNote !== null || error !== null);

  if (noProvider) {
    return (
      <div className="studio__notice plan-run__notice">
        <BrushStroke className="studio__notice-flourish" aria-hidden="true" />
        <span className="studio__notice-seal">
          <IconProviders size={26} />
        </span>
        <h3 className="studio__notice-title">尚未选用模型</h3>
        <p className="studio__notice-text">
          生成设定需要一个已启用的模型供应商。请先到「供应商」添加并启用一个模型，
          再回到这里生成{action.label}。
        </p>
        {error && <div className="studio__notice-err">{error}</div>}
        <button
          className="btn btn--primary"
          onClick={() => onNavigate?.("providers")}
        >
          <IconProviders size={16} />
          前往供应商配置
        </button>
      </div>
    );
  }

  if (providerCompat) {
    return (
      <div className="studio__notice plan-run__notice">
        <BrushStroke className="studio__notice-flourish" aria-hidden="true" />
        <span className="studio__notice-seal">
          <IconTools size={26} />
        </span>
        <h3 className="studio__notice-title">模型工具模式不兼容</h3>
        <p className="studio__notice-text">
          当前模型可能拒绝原生工具调用。请到「供应商」把工具调用模式改为「自动兼容」
          或「文本工具」，再回来生成{action.label}。
        </p>
        {error && <div className="studio__notice-err">{error}</div>}
        <button
          className="btn btn--primary"
          onClick={() => onNavigate?.("providers")}
        >
          <IconProviders size={16} />
          调整供应商设置
        </button>
      </div>
    );
  }

  const phase = derivePhase({
    running,
    steps,
    finished: finishNote !== null,
    success,
    errored: error !== null,
  });
  const wf = workflowView(phase);
  const toolCount = steps.reduce((n, s) => n + s.toolCalls.length, 0);
  const lastStepNo = steps.length > 0 ? steps[steps.length - 1].step : 0;
  const currentToolNote =
    phase === "tooling" && steps.length > 0 && steps[steps.length - 1].toolCalls[0]
      ? steps[steps.length - 1].toolCalls[0].name
      : undefined;

  return (
    <div className="plan-run">
      <div className="studio__section-label plan-run__label">
        {running ? (
          <>
            <span className="ink-pulse" aria-hidden="true" />
            正在生成「{action.label}」…
            {lastStepNo > 0 && ` 第 ${lastStepNo} 步`}
          </>
        ) : (
          <>
            <action.Icon size={13} />「{action.label}」· {steps.length} 步
          </>
        )}
        {done && (
          <button
            className="btn btn--ghost btn--sm plan-run__close"
            onClick={onClose}
            title="收起"
          >
            收起
          </button>
        )}
      </div>

      {/* ---- the agent console: 工作流程 + 工作状态 ---- */}
      <div className="agent-console">
        <WorkflowSteps stages={PLAN_STAGES} current={wf.current} state={wf.state} />
        <WorkStatus
          phase={phase}
          step={lastStepNo}
          toolCount={toolCount}
          note={currentToolNote}
        />
      </div>

      {done && error && (
        <div className="banner banner--warn plan-run__banner">
          <IconWarn size={16} />
          {error}
        </div>
      )}
      {done && !error && (
        <div className="plan-run__done">
          <span className="plan-run__done-mark">
            <IconCheck size={15} />
          </span>
          <span className="plan-run__done-text">
            {finishNote ?? "已生成"}，已写入记忆库。
          </span>
          {onNavigate && (
            <button
              className="btn btn--sm plan-run__jump"
              onClick={() => onNavigate(action.target)}
            >
              <action.Icon size={14} />
              去查看 {action.targetLabel}
            </button>
          )}
        </div>
      )}

      {steps.length > 0 && (
        <AgentFeed
          steps={steps}
          running={running}
          pendingText={`AI 正在斟酌下一条…`}
          tailRef={liveTailRef}
        />
      )}

      {running && steps.length === 0 && (
        <div className="plan-run__warming">
          <span className="plan-run__warming-orb">
            <BrushStroke className="studio__warming-brush" aria-hidden="true" />
            <Spinner size={30} />
          </span>
          <span className="plan-run__warming-text">
            正在唤起模型，构思{action.label}…
          </span>
        </div>
      )}
    </div>
  );
}
