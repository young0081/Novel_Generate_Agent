// 供应商 — configure model providers (OpenAI-compatible / Anthropic) with
// multi-provider + multi-model management, an active provider+model picker,
// and a live "测试连接" probe. Backed by the providers_* Tauri commands.

import "../styles/providers.css";

import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  type KeyboardEvent,
} from "react";
import Panel from "../components/Panel";
import { Spinner } from "../components/Spinner";
import { SkeletonGrid } from "../components/Skeleton";
import EmptyState from "../components/EmptyState";
import ConfirmModal from "../components/ConfirmModal";
import { useToast } from "../components/Toast";
import {
  IconPlus,
  IconRefresh,
  IconClose,
  IconKey,
  IconEye,
  IconEyeOff,
  IconPlug,
  IconTrash,
  IconPencil,
  IconStar,
  IconCheck,
  IconWarn,
  IconInfo,
  IconProviders,
  IconTools,
} from "../components/icons";
import { describeError } from "../lib/core";
import {
  getProviders,
  saveProvider,
  deleteProvider,
  setActiveProvider,
  testProvider,
  newProviderId,
  maskBaseUrl,
  PROTOCOL_LABEL,
  PROTOCOL_PLACEHOLDER,
  TOOL_MODE_HINT,
  TOOL_MODE_LABEL,
  PROVIDER_PRESETS,
  type ProviderPreset,
  type ProviderConfig,
  type ProviderProtocol,
  type ProviderToolMode,
  type ProviderSettings,
} from "../lib/providers";
import "../styles/providers.css";

// ---- the editable form shape (max_tokens kept as text for a clean input) ----
interface ProviderForm {
  id: string;
  name: string;
  protocol: ProviderProtocol;
  tool_mode: ProviderToolMode;
  base_url: string;
  api_key: string;
  models: string[];
  default_model: string;
  max_tokens: string;
  temperature: number;
  top_p: number;
  top_k: number;
  presence_penalty: number;
  frequency_penalty: number;
}

function emptyForm(): ProviderForm {
  return {
    id: newProviderId(),
    name: "",
    protocol: "open_ai",
    tool_mode: "auto",
    base_url: "",
    api_key: "",
    models: [],
    default_model: "",
    max_tokens: "",
    temperature: 1.0,
    top_p: 1.0,
    top_k: 0,
    presence_penalty: 0.0,
    frequency_penalty: 0.0,
  };
}

function toForm(p: ProviderConfig): ProviderForm {
  const sampling = p.sampling || {
    temperature: 1.0,
    top_p: 1.0,
    top_k: 0,
    presence_penalty: 0.0,
    frequency_penalty: 0.0,
  };
  return {
    id: p.id,
    name: p.name,
    protocol: p.protocol,
    tool_mode: p.tool_mode ?? "auto",
    base_url: p.base_url,
    api_key: p.api_key,
    models: [...p.models],
    default_model: p.default_model ?? "",
    max_tokens: p.max_tokens != null ? String(p.max_tokens) : "",
    temperature: sampling.temperature,
    top_p: sampling.top_p,
    top_k: sampling.top_k,
    presence_penalty: sampling.presence_penalty,
    frequency_penalty: sampling.frequency_penalty,
  };
}

function toConfig(f: ProviderForm): ProviderConfig {
  const models = f.models.map((m) => m.trim()).filter(Boolean);
  const def = f.default_model.trim();
  const mt = f.max_tokens.trim();
  const parsedMt = mt ? Number(mt) : NaN;
  return {
    id: f.id,
    name: f.name.trim(),
    protocol: f.protocol,
    tool_mode: f.tool_mode,
    base_url: f.base_url.trim(),
    api_key: f.api_key.trim(),
    models,
    default_model: def && models.includes(def) ? def : models[0] ?? null,
    max_tokens: Number.isFinite(parsedMt) && parsedMt > 0 ? parsedMt : null,
    sampling: {
      temperature: f.temperature,
      top_p: f.top_p,
      top_k: f.top_k,
      presence_penalty: f.presence_penalty,
      frequency_penalty: f.frequency_penalty,
    },
  };
}

interface TestState {
  busy: boolean;
  ok: boolean | null;
  message: string;
  model: string;
}

const IDLE_TEST: TestState = { busy: false, ok: null, message: "", model: "" };

export default function ProvidersScreen() {
  const toast = useToast();
  const [settings, setSettings] = useState<ProviderSettings>({
    providers: [],
    active_provider: null,
    active_model: null,
  });
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // drawer / editor
  const [editing, setEditing] = useState<ProviderForm | null>(null);
  const [isNew, setIsNew] = useState(false);
  const [showKey, setShowKey] = useState(false);
  const [modelDraft, setModelDraft] = useState("");
  const [saving, setSaving] = useState(false);
  const [test, setTest] = useState<TestState>(IDLE_TEST);

  // delete confirm
  const [pendingDelete, setPendingDelete] = useState<ProviderConfig | null>(
    null,
  );
  const [deleting, setDeleting] = useState(false);

  // per-card "applying active" busy id
  const [applyingId, setApplyingId] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const s = await getProviders();
      setSettings(s);
    } catch (e) {
      setError(describeError(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const providers = settings.providers;
  const activeProvider = settings.active_provider ?? null;
  const activeModel = settings.active_model ?? null;

  const activeProviderName = useMemo(() => {
    const p = providers.find((x) => x.id === activeProvider);
    return p?.name ?? null;
  }, [providers, activeProvider]);

  // ---- drawer open/close ----
  const openCreate = useCallback(() => {
    setEditing(emptyForm());
    setIsNew(true);
    setShowKey(false);
    setModelDraft("");
    setTest(IDLE_TEST);
  }, []);

  const openEdit = useCallback((p: ProviderConfig) => {
    setEditing(toForm(p));
    setIsNew(false);
    setShowKey(false);
    setModelDraft("");
    setTest(IDLE_TEST);
  }, []);

  const closeDrawer = useCallback(() => {
    if (saving || test.busy) return;
    setEditing(null);
  }, [saving, test.busy]);

  // ---- apply a quick-fill preset (fills the empty/blank fields only) ----
  const applyPreset = useCallback((preset: ProviderPreset) => {
    setEditing((prev) => {
      if (!prev) return prev;
      const models =
        prev.models.length > 0
          ? prev.models
          : preset.models
            ? [...preset.models]
            : [];
      return {
        ...prev,
        name: prev.name.trim() ? prev.name : preset.name,
        protocol: preset.protocol,
        tool_mode:
          prev.tool_mode === "auto" && preset.tool_mode
            ? preset.tool_mode
            : prev.tool_mode,
        base_url: preset.base_url,
        models,
        default_model:
          prev.default_model && models.includes(prev.default_model)
            ? prev.default_model
            : models[0] ?? "",
      };
    });
    setTest(IDLE_TEST);
  }, []);

  // ---- models editor ----
  const addModel = useCallback(() => {
    const name = modelDraft.trim();
    if (!name) return;
    setEditing((prev) => {
      if (!prev) return prev;
      if (prev.models.includes(name)) return prev;
      const models = [...prev.models, name];
      return {
        ...prev,
        models,
        default_model: prev.default_model || name,
      };
    });
    setModelDraft("");
  }, [modelDraft]);

  const removeModel = useCallback((name: string) => {
    setEditing((prev) => {
      if (!prev) return prev;
      const models = prev.models.filter((m) => m !== name);
      const default_model =
        prev.default_model === name ? models[0] ?? "" : prev.default_model;
      return { ...prev, models, default_model };
    });
  }, []);

  const setDefaultModel = useCallback((name: string) => {
    setEditing((prev) => (prev ? { ...prev, default_model: name } : prev));
  }, []);

  const onModelKey = useCallback(
    (e: KeyboardEvent<HTMLInputElement>) => {
      if (e.key === "Enter") {
        e.preventDefault();
        addModel();
      }
    },
    [addModel],
  );

  // ---- validation shared by save + test ----
  const validate = useCallback((f: ProviderForm): string | null => {
    if (!f.name.trim()) return "请填写供应商名称";
    if (!f.base_url.trim()) return "请填写 Base URL";
    if (!f.api_key.trim()) return "请填写 API Key";
    if (f.models.map((m) => m.trim()).filter(Boolean).length === 0) {
      return "请至少添加一个模型";
    }
    return null;
  }, []);

  // ---- save ----
  const submit = useCallback(async () => {
    if (!editing) return;
    const problem = validate(editing);
    if (problem) {
      toast.err(problem);
      return;
    }
    setSaving(true);
    try {
      const config = toConfig(editing);
      let next = await saveProvider(config);
      // If nothing is active yet but this provider has a usable model, adopt it
      // as the active selection so 当前模型 reflects a real choice (the core
      // already falls back at runtime; this keeps the UI honest).
      if (!next.active_provider) {
        const pick =
          (config.default_model && config.models.includes(config.default_model)
            ? config.default_model
            : config.models[0]) ?? "";
        if (pick) {
          try {
            next = await setActiveProvider(config.id, pick);
          } catch {
            // non-fatal: the save itself succeeded; leave active as-is
          }
        }
      }
      setSettings(next);
      toast.ok(isNew ? "已添加供应商" : "已保存修改");
      setEditing(null);
    } catch (e) {
      toast.err(describeError(e));
    } finally {
      setSaving(false);
    }
  }, [editing, isNew, validate, toast]);

  // ---- delete ----
  const confirmDelete = useCallback(async () => {
    if (!pendingDelete) return;
    setDeleting(true);
    try {
      const next = await deleteProvider(pendingDelete.id);
      setSettings(next);
      toast.ok("已删除供应商");
      // if the open drawer was editing this provider, close it
      setEditing((prev) => (prev && prev.id === pendingDelete.id ? null : prev));
      setPendingDelete(null);
    } catch (e) {
      toast.err(describeError(e));
    } finally {
      setDeleting(false);
    }
  }, [pendingDelete, toast]);

  // ---- pick active provider+model ----
  const applyActive = useCallback(
    async (providerId: string, model: string) => {
      if (!model) return;
      setApplyingId(providerId);
      try {
        const next = await setActiveProvider(providerId, model);
        setSettings(next);
        toast.ok(`已启用：${model}`);
      } catch (e) {
        toast.err(describeError(e));
      } finally {
        setApplyingId(null);
      }
    },
    [toast],
  );

  // the model chosen in the drawer for testing (defaults to default/first)
  const [testModel, setTestModel] = useState("");
  useEffect(() => {
    if (!editing) {
      setTestModel("");
      return;
    }
    // keep the selection valid as models change
    setTestModel((cur) => {
      if (cur && editing.models.includes(cur)) return cur;
      return editing.default_model || editing.models[0] || "";
    });
  }, [editing]);

  // ---- test connection (network) ----
  const runTest = useCallback(
    async (model: string) => {
      if (!editing) return;
      const problem = validate(editing);
      if (problem) {
        toast.err(problem);
        return;
      }
      if (!model) {
        toast.err("请先选择要测试的模型");
        return;
      }
      setTest({ busy: true, ok: null, message: "", model });
      try {
        const reply = await testProvider(toConfig(editing), model);
        const text = reply.trim() || "（模型返回了空内容，但连接成功）";
        setTest({ busy: false, ok: true, message: text, model });
        toast.ok("连接成功，模型已响应");
      } catch (e) {
        const msg = describeError(e);
        setTest({ busy: false, ok: false, message: msg, model });
        toast.err(`连接失败：${msg}`);
      }
    },
    [editing, validate, toast],
  );

  const presetList = editing
    ? PROVIDER_PRESETS.filter((p) => p.protocol === editing.protocol)
    : [];

  const headerActions = (
    <>
      <button
        className="btn btn--ghost btn--icon"
        onClick={() => void refresh()}
        title="刷新"
        aria-label="刷新"
      >
        <IconRefresh size={16} />
      </button>
      <button className="btn btn--primary" onClick={openCreate}>
        <IconPlus size={16} />
        添加供应商
      </button>
    </>
  );

  const toolbar = (
    <div className="toolbar">
      <span className="toolbar__label">当前模型</span>
      {activeProvider && activeModel ? (
        <span className="chip chip--accent">
          <IconStar size={11} />
          {activeProviderName} · {activeModel}
        </span>
      ) : (
        <span className="chip">尚未选用</span>
      )}
      <span className="count-pill" style={{ marginLeft: "auto" }}>
        {providers.length} 个供应商
      </span>
    </div>
  );

  return (
    <Panel
      title="供应商"
      en="Providers"
      subtitle="配置接入的模型供应商 · 多供应商 / 多模型，随时切换与测试"
      actions={headerActions}
      toolbar={toolbar}
    >
      <div className="scroll-area">
        {loading ? (
          <SkeletonGrid count={4} />
        ) : error ? (
          <div className="banner banner--warn">
            <IconWarn size={16} />
            {error}
          </div>
        ) : providers.length === 0 ? (
          <EmptyState
            title="尚未配置供应商"
            text="添加一个模型供应商即可开始创作。支持 OpenAI / DeepSeek / Kimi / 智谱 / Ollama / OpenRouter（OpenAI 兼容）、Anthropic（Claude）以及 Gemini 原生接口。"
            action={
              <button className="btn btn--primary" onClick={openCreate}>
                <IconPlus size={16} />
                添加供应商
              </button>
            }
          />
        ) : (
          <div className="slip-grid">
            {providers.map((p) => {
              const isActiveProv = p.id === activeProvider;
              const curModel = isActiveProv ? activeModel : null;
              const pickValue =
                (isActiveProv && activeModel) ||
                p.default_model ||
                p.models[0] ||
                "";
              return (
                <article
                  className={`prov-card${isActiveProv ? " is-active" : ""}`}
                  key={p.id}
                >
                  <div className="prov-card__head">
                    <span className="prov-card__name">
                      <IconProviders size={16} className="prov-card__glyph" />
                      <span className="prov-card__name-text">
                        {p.name || "（未命名供应商）"}
                      </span>
                    </span>
                    <span
                      className={`badge badge--proto${
                        p.protocol === "anthropic"
                          ? " is-anthropic"
                          : p.protocol === "gemini"
                            ? " is-gemini"
                            : ""
                      }`}
                    >
                      {PROTOCOL_LABEL[p.protocol]}
                    </span>
                  </div>

                  <div className="prov-card__url" title={p.base_url}>
                    {maskBaseUrl(p.base_url)}
                  </div>

                  <div
                    className="prov-card__mode"
                    title={TOOL_MODE_HINT[p.tool_mode ?? "auto"]}
                  >
                    <IconTools size={13} />
                    {TOOL_MODE_LABEL[p.tool_mode ?? "auto"]}
                  </div>

                  <div className="prov-card__section">
                    <div className="prov-card__rowlabel">
                      模型（{p.models.length}）
                    </div>
                    <div className="prov-card__models">
                      {p.models.length === 0 ? (
                        <span className="prov-card__empty">暂无模型</span>
                      ) : (
                        p.models.map((m) => {
                          const isCur = isActiveProv && m === curModel;
                          const isDef = m === p.default_model;
                          return (
                            <span
                              className={`tag-chip${
                                isCur ? " is-current" : isDef ? " is-default" : ""
                              }`}
                              key={m}
                              title={
                                isCur
                                  ? "当前启用的模型"
                                  : isDef
                                    ? "默认模型"
                                    : undefined
                              }
                            >
                              {isCur ? (
                                <span className="tag-chip__star">
                                  <IconStar size={11} />
                                </span>
                              ) : isDef ? (
                                <span className="tag-chip__star is-muted">
                                  <IconStar size={11} />
                                </span>
                              ) : null}
                              {m}
                            </span>
                          );
                        })
                      )}
                    </div>
                  </div>

                  <div className="prov-card__pick">
                    <select
                      className="select"
                      value={pickValue}
                      disabled={p.models.length === 0 || applyingId === p.id}
                      onChange={(e) => void applyActive(p.id, e.target.value)}
                      aria-label={`为 ${p.name} 选择当前模型`}
                    >
                      {p.models.length === 0 ? (
                        <option value="">（无可用模型）</option>
                      ) : (
                        p.models.map((m) => (
                          <option key={m} value={m}>
                            {m}
                          </option>
                        ))
                      )}
                    </select>
                    {applyingId === p.id ? (
                      <Spinner size={16} />
                    ) : isActiveProv ? (
                      <span className="prov-card__current">
                        <IconStar size={11} />
                        当前
                      </span>
                    ) : (
                      <span className="prov-card__hint">点选启用</span>
                    )}
                  </div>

                  <div className="prov-card__foot">
                    <button
                      className="btn btn--sm"
                      onClick={() => openEdit(p)}
                    >
                      <IconPencil size={14} />
                      编辑
                    </button>
                    <button
                      className="btn btn--danger btn--sm"
                      onClick={() => setPendingDelete(p)}
                    >
                      <IconTrash size={14} />
                      删除
                    </button>
                  </div>
                </article>
              );
            })}
          </div>
        )}
      </div>

      {editing && (
        <aside className="drawer">
          <div className="drawer__head">
            <h3>{isNew ? "添加供应商" : "编辑供应商"}</h3>
            <button
              className="btn btn--ghost btn--icon"
              onClick={closeDrawer}
              aria-label="关闭"
              disabled={saving || test.busy}
            >
              <IconClose size={16} />
            </button>
          </div>

          <div className="drawer__body">
            {presetList.length > 0 && (
              <div className="field">
                <label className="field__label">快速填充</label>
                <div className="preset-row">
                  {presetList.map((preset) => (
                    <button
                      type="button"
                      key={preset.name}
                      className="preset-chip"
                      onClick={() => applyPreset(preset)}
                      title={`填入 ${preset.name} 的接入地址${
                        preset.models ? "与推荐模型" : ""
                      }`}
                    >
                      {preset.name}
                    </button>
                  ))}
                </div>
                <p className="field__hint">
                  一键填入接入地址，再补上你的 API Key 即可。
                </p>
              </div>
            )}

            <div className="drawer__section-label">凭据</div>

            <div className="field">
              <label className="field__label">名称</label>
              <input
                className="input"
                value={editing.name}
                onChange={(e) =>
                  setEditing({ ...editing, name: e.target.value })
                }
                placeholder="如：OpenAI、DeepSeek、Claude…"
              />
            </div>

            <div className="field">
              <label className="field__label">协议</label>
              <select
                className="select"
                value={editing.protocol}
                onChange={(e) =>
                  setEditing({
                    ...editing,
                    protocol: e.target.value as ProviderProtocol,
                  })
                }
              >
                <option value="open_ai">{PROTOCOL_LABEL.open_ai}</option>
                <option value="anthropic">{PROTOCOL_LABEL.anthropic}</option>
                <option value="gemini">{PROTOCOL_LABEL.gemini}</option>
              </select>
              <p className="field__hint">
                {editing.protocol === "anthropic"
                  ? "Claude 官方接口。"
                  : editing.protocol === "gemini"
                    ? "Google Gemini 原生 generateContent 接口，支持原生函数调用。"
                    : "适用于 OpenAI、DeepSeek、Kimi、智谱、Ollama、OpenRouter 等兼容服务。"}
              </p>
            </div>

            <div className="field">
              <label className="field__label">工具调用模式</label>
              <select
                className="select"
                value={editing.tool_mode}
                onChange={(e) =>
                  setEditing({
                    ...editing,
                    tool_mode: e.target.value as ProviderToolMode,
                  })
                }
              >
                <option value="auto">{TOOL_MODE_LABEL.auto}（推荐）</option>
                <option value="text">{TOOL_MODE_LABEL.text}</option>
                <option value="native">{TOOL_MODE_LABEL.native}</option>
              </select>
              <p className="field__hint">
                <IconTools size={12} className="field__hint-icon" />
                {TOOL_MODE_HINT[editing.tool_mode]}
              </p>
            </div>

            <div className="field">
              <label className="field__label">Base URL</label>
              <input
                className="input"
                value={editing.base_url}
                onChange={(e) =>
                  setEditing({ ...editing, base_url: e.target.value })
                }
                placeholder={PROTOCOL_PLACEHOLDER[editing.protocol]}
                spellCheck={false}
              />
            </div>

            <div className="field">
              <label className="field__label">API Key</label>
              <div className="input-affix">
                <input
                  className="input"
                  type={showKey ? "text" : "password"}
                  value={editing.api_key}
                  onChange={(e) =>
                    setEditing({ ...editing, api_key: e.target.value })
                  }
                  placeholder="sk-…"
                  spellCheck={false}
                  autoComplete="off"
                />
                <button
                  type="button"
                  className="input-affix__btn"
                  onClick={() => setShowKey((v) => !v)}
                  title={showKey ? "隐藏" : "显示"}
                  aria-label={showKey ? "隐藏密钥" : "显示密钥"}
                >
                  {showKey ? <IconEyeOff size={16} /> : <IconEye size={16} />}
                </button>
              </div>
              <p className="field__hint">
                <IconKey size={12} className="field__hint-icon" />
                密钥仅保存在本机，不会上传。
              </p>
            </div>

            <div className="drawer__section-label">模型</div>

            <div className="field">
              <label className="field__label">模型列表（可添加多个）</label>
              <div className="model-editor__add">
                <input
                  className="input"
                  value={modelDraft}
                  onChange={(e) => setModelDraft(e.target.value)}
                  onKeyDown={onModelKey}
                  placeholder={
                    editing.protocol === "anthropic"
                      ? "如：claude-3-5-sonnet-latest"
                      : editing.protocol === "gemini"
                        ? "如：gemini-2.5-flash"
                        : "如：gpt-4o-mini、deepseek-chat"
                  }
                  spellCheck={false}
                />
                <button
                  type="button"
                  className="btn"
                  onClick={addModel}
                  disabled={!modelDraft.trim()}
                >
                  <IconPlus size={16} />
                  添加
                </button>
              </div>
              <div className="model-editor__list">
                {editing.models.length === 0 ? (
                  <span className="model-editor__empty">
                    还没有模型，在上方输入名称后回车添加。
                  </span>
                ) : (
                  editing.models.map((m) => {
                    const isDef = m === editing.default_model;
                    return (
                      <span
                        className={`tag-chip is-removable${
                          isDef ? " is-default" : ""
                        }`}
                        key={m}
                      >
                        <button
                          type="button"
                          className={`tag-chip__starbtn${
                            isDef ? " is-on" : ""
                          }`}
                          onClick={() => setDefaultModel(m)}
                          title={isDef ? "默认模型" : "设为默认模型"}
                          aria-label={isDef ? "默认模型" : "设为默认模型"}
                          aria-pressed={isDef}
                        >
                          <IconStar size={12} />
                        </button>
                        {m}
                        <button
                          type="button"
                          className="tag-chip__x"
                          onClick={() => removeModel(m)}
                          title="移除"
                          aria-label={`移除 ${m}`}
                        >
                          <IconClose size={12} />
                        </button>
                      </span>
                    );
                  })
                )}
              </div>
              {editing.models.length > 0 && (
                <p className="field__hint">点亮 ★ 可设为默认模型。</p>
              )}
            </div>

            <div className="field-row">
              <div className="field">
                <label className="field__label">默认模型</label>
                <select
                  className="select"
                  value={editing.default_model}
                  onChange={(e) =>
                    setEditing({ ...editing, default_model: e.target.value })
                  }
                  disabled={editing.models.length === 0}
                >
                  {editing.models.length === 0 ? (
                    <option value="">（先添加模型）</option>
                  ) : (
                    editing.models.map((m) => (
                      <option key={m} value={m}>
                        {m}
                      </option>
                    ))
                  )}
                </select>
              </div>
              <div className="field">
                <label className="field__label">
                  max_tokens
                  {editing.protocol === "anthropic" ? "（默认 4096）" : "（可选）"}
                </label>
                <input
                  className="input"
                  inputMode="numeric"
                  value={editing.max_tokens}
                  onChange={(e) => {
                    const v = e.target.value.replace(/[^\d]/g, "");
                    setEditing({ ...editing, max_tokens: v });
                  }}
                  placeholder={editing.protocol === "anthropic" ? "4096" : "留空"}
                />
              </div>
            </div>

            <div className="drawer__section-label">采样参数</div>
            <p className="field__hint" style={{ marginTop: "-0.5rem", marginBottom: "1rem" }}>
              <IconInfo size={12} className="field__hint-icon" />
              控制生成文本的随机性和多样性。Temperature 越低越确定，越高越随机。
            </p>

            <div className="sampling-controls">
              {/* Temperature */}
              <div className="field">
                <div style={{ display: "flex", justifyContent: "space-between", marginBottom: "0.5rem" }}>
                  <label className="field__label">Temperature</label>
                  <span className="sampling-value">{editing.temperature.toFixed(2)}</span>
                </div>
                <input
                  type="range"
                  className="slider"
                  min="0"
                  max="2"
                  step="0.01"
                  value={editing.temperature}
                  onChange={(e) =>
                    setEditing({ ...editing, temperature: parseFloat(e.target.value) })
                  }
                />
                <div className="slider-labels">
                  <span>确定 (0.0)</span>
                  <span>随机 (2.0)</span>
                </div>
              </div>

              {/* Top P */}
              <div className="field">
                <div style={{ display: "flex", justifyContent: "space-between", marginBottom: "0.5rem" }}>
                  <label className="field__label">Top P (核采样)</label>
                  <span className="sampling-value">{editing.top_p.toFixed(2)}</span>
                </div>
                <input
                  type="range"
                  className="slider"
                  min="0"
                  max="1"
                  step="0.01"
                  value={editing.top_p}
                  onChange={(e) =>
                    setEditing({ ...editing, top_p: parseFloat(e.target.value) })
                  }
                />
                <div className="slider-labels">
                  <span>严格 (0.0)</span>
                  <span>宽松 (1.0)</span>
                </div>
              </div>

              {/* Top K */}
              <div className="field">
                <div style={{ display: "flex", justifyContent: "space-between", marginBottom: "0.5rem" }}>
                  <label className="field__label">Top K</label>
                  <span className="sampling-value">{editing.top_k === 0 ? "关闭" : editing.top_k}</span>
                </div>
                <input
                  type="range"
                  className="slider"
                  min="0"
                  max="100"
                  step="1"
                  value={editing.top_k}
                  onChange={(e) =>
                    setEditing({ ...editing, top_k: parseInt(e.target.value) })
                  }
                />
                <div className="slider-labels">
                  <span>关闭 (0)</span>
                  <span>限制候选 (100)</span>
                </div>
              </div>

              {/* Presence Penalty */}
              <div className="field">
                <div style={{ display: "flex", justifyContent: "space-between", marginBottom: "0.5rem" }}>
                  <label className="field__label">Presence Penalty (出现惩罚)</label>
                  <span className="sampling-value">{editing.presence_penalty.toFixed(2)}</span>
                </div>
                <input
                  type="range"
                  className="slider"
                  min="-2"
                  max="2"
                  step="0.01"
                  value={editing.presence_penalty}
                  onChange={(e) =>
                    setEditing({ ...editing, presence_penalty: parseFloat(e.target.value) })
                  }
                />
                <div className="slider-labels">
                  <span>重复 (-2.0)</span>
                  <span>避免 (2.0)</span>
                </div>
              </div>

              {/* Frequency Penalty */}
              <div className="field">
                <div style={{ display: "flex", justifyContent: "space-between", marginBottom: "0.5rem" }}>
                  <label className="field__label">Frequency Penalty (频率惩罚)</label>
                  <span className="sampling-value">{editing.frequency_penalty.toFixed(2)}</span>
                </div>
                <input
                  type="range"
                  className="slider"
                  min="-2"
                  max="2"
                  step="0.01"
                  value={editing.frequency_penalty}
                  onChange={(e) =>
                    setEditing({ ...editing, frequency_penalty: parseFloat(e.target.value) })
                  }
                />
                <div className="slider-labels">
                  <span>允许重复 (-2.0)</span>
                  <span>减少重复 (2.0)</span>
                </div>
              </div>

              <button
                type="button"
                className="btn btn--ghost btn--sm"
                style={{ width: "100%", marginTop: "0.5rem" }}
                onClick={() => {
                  setEditing({
                    ...editing,
                    temperature: 1.0,
                    top_p: 1.0,
                    top_k: 0,
                    presence_penalty: 0.0,
                    frequency_penalty: 0.0,
                  });
                }}
              >
                重置为默认值
              </button>
            </div>

            <div className="drawer__section-label">测试连接</div>

            {/* test connection */}
            <div className="field">
              <label className="field__label">向所选模型发送一条测试消息</label>
              <div className="model-editor__add">
                <select
                  className="select"
                  value={testModel}
                  onChange={(e) => setTestModel(e.target.value)}
                  disabled={editing.models.length === 0 || test.busy}
                  aria-label="选择测试用模型"
                >
                  {editing.models.length === 0 ? (
                    <option value="">（先添加模型）</option>
                  ) : (
                    editing.models.map((m) => (
                      <option key={m} value={m}>
                        {m}
                      </option>
                    ))
                  )}
                </select>
                <button
                  type="button"
                  className="btn"
                  onClick={() => void runTest(testModel)}
                  disabled={test.busy || editing.models.length === 0}
                >
                  {test.busy ? <Spinner size={15} /> : <IconPlug size={16} />}
                  测试连接
                </button>
              </div>
              {test.busy ? (
                <div className="test-result">
                  <div className="test-result__busy">
                    <Spinner size={16} />
                    正在向 <code>{test.model}</code> 发送测试消息…
                  </div>
                </div>
              ) : test.ok !== null ? (
                <div
                  className={`test-result ${test.ok ? "is-ok" : "is-err"}`}
                >
                  <div className="test-result__title">
                    {test.ok ? (
                      <>
                        <IconCheck size={14} />
                        连接成功 · {test.model} 回复
                      </>
                    ) : (
                      <>
                        <IconWarn size={14} />
                        连接失败 · {test.model}
                      </>
                    )}
                  </div>
                  <div className="test-result__body">{test.message}</div>
                </div>
              ) : (
                <p className="field__hint">
                  <IconInfo size={12} className="field__hint-icon" />
                  这里验证基础聊天；创作/策划是否稳定，主要取决于上方工具调用模式。
                </p>
              )}
            </div>
          </div>

          <div className="drawer__foot">
            {!isNew && (
              <button
                className="btn btn--danger btn--icon"
                onClick={() => {
                  const p = providers.find((x) => x.id === editing.id);
                  if (p) setPendingDelete(p);
                }}
                disabled={saving || test.busy}
                title="删除该供应商"
                aria-label="删除该供应商"
              >
                <IconTrash size={16} />
              </button>
            )}
            <button
              className="btn"
              onClick={closeDrawer}
              disabled={saving || test.busy}
            >
              取消
            </button>
            <button
              className="btn btn--primary"
              style={{ flex: 1 }}
              onClick={() => void submit()}
              disabled={saving || test.busy}
            >
              {saving ? <Spinner size={15} /> : <IconCheck size={16} />}
              {isNew ? "添加供应商" : "保存修改"}
            </button>
          </div>
        </aside>
      )}

      <ConfirmModal
        open={pendingDelete !== null}
        title="删除该供应商？"
        sealChar="删"
        danger
        busy={deleting}
        confirmLabel="确认删除"
        body={
          <>
            将删除供应商
            <br />
            <code>{pendingDelete?.name || pendingDelete?.id}</code>
            <br />
            其下的模型配置一并移除，此操作不可撤销。
          </>
        }
        onConfirm={() => void confirmDelete()}
        onCancel={() => {
          if (!deleting) setPendingDelete(null);
        }}
      />
    </Panel>
  );
}
