import { useCallback, useEffect, useState, type Dispatch, type SetStateAction } from "react";
import { useToast } from "../components/Toast";
import { Spinner } from "../components/Spinner";
import {
  IconBrush,
  IconCheck,
  IconClose,
  IconSave,
  IconTrash,
} from "../components/icons";
import {
  analyzeStyle,
  deleteStyleProfile,
  getStyleProfiles,
  joinRules,
  saveStyleProfile,
  setActiveStyle,
  splitRules,
  type StylePayload,
  type StyleProfile,
} from "../lib/styles";
import { describeError, isDesktop } from "../lib/core";
import { useWork } from "../components/WorkContext";

const RULE_FIELDS: Array<[keyof StyleProfile, string]> = [
  ["sentence_patterns", "句式与段落规则"],
  ["dialogue_rules", "对话规则"],
  ["imagery_rules", "意象与修辞规则"],
  ["banned_patterns", "必须避免"],
  ["humanizer_rules", "去 AI 味规则"],
  ["sample_excerpts", "短句示例"],
];

function mergePayload(setPayload: Dispatch<SetStateAction<StylePayload | null>>, payload: StylePayload) {
  setPayload(payload);
}

export default function StyleScreen() {
  const toast = useToast();
  const { current } = useWork();
  const [payload, setPayload] = useState<StylePayload | null>(null);
  const [loading, setLoading] = useState(true);
  const [article, setArticle] = useState("");
  const [analyzing, setAnalyzing] = useState(false);
  const [saving, setSaving] = useState(false);
  const [draft, setDraft] = useState<StyleProfile | null>(null);

  const refresh = useCallback(async () => {
    if (!isDesktop()) {
      setLoading(false);
      return;
    }
    setLoading(true);
    try {
      setPayload(await getStyleProfiles());
    } catch (error) {
      toast.err(describeError(error));
    } finally {
      setLoading(false);
    }
  }, [toast]);

  useEffect(() => {
    void refresh();
    setDraft(null);
    setArticle("");
  }, [refresh, current?.id]);

  const runAnalysis = useCallback(async () => {
    if (article.trim().length < 80 || analyzing) return;
    setAnalyzing(true);
    try {
      setDraft(await analyzeStyle(article));
      toast.ok("文风分析完成，可在右侧调整后保存");
    } catch (error) {
      toast.err(describeError(error));
    } finally {
      setAnalyzing(false);
    }
  }, [analyzing, article, toast]);

  const save = useCallback(async () => {
    if (!draft || !draft.name.trim() || saving) return;
    setSaving(true);
    try {
      const next = await saveStyleProfile(draft);
      mergePayload(setPayload, next);
      setDraft(next.saved ?? next.profiles.find((profile) => profile.id === draft.id) ?? draft);
      toast.ok("文风档案已保存");
    } catch (error) {
      toast.err(describeError(error));
    } finally {
      setSaving(false);
    }
  }, [draft, saving, toast]);

  const choose = useCallback(async (id: string | null) => {
    try {
      const next = await setActiveStyle(id);
      setPayload(next);
      toast.ok(id ? "已启用该文风，后续创作会自动沿用" : "已取消当前文风");
    } catch (error) {
      toast.err(describeError(error));
    }
  }, [toast]);

  const remove = useCallback(async (profile: StyleProfile) => {
    if (!window.confirm(`删除文风“${profile.name}”？`)) return;
    try {
      setPayload(await deleteStyleProfile(profile.id));
      if (draft?.id === profile.id) setDraft(null);
      toast.ok("文风档案已删除");
    } catch (error) {
      toast.err(describeError(error));
    }
  }, [draft?.id, toast]);

  const updateDraft = useCallback(<K extends keyof StyleProfile>(key: K, value: StyleProfile[K]) => {
    setDraft((currentDraft) => currentDraft ? { ...currentDraft, [key]: value } : currentDraft);
  }, []);

  const editProfile = (profile: StyleProfile) => setDraft({ ...profile, source_article_count: profile.source_article_count || 1 });
  const activeName = payload?.active?.name ?? "默认写作规范";

  return (
    <div className="style-screen">
      <header className="style-screen__head">
        <div>
          <div className="style-screen__kicker">VOICE &amp; TONE</div>
          <h1>文风</h1>
          <p>投喂你喜欢的文章，提炼成整本书都能遵循的叙述声音。</p>
        </div>
        <div className="style-screen__active">
          <span className="style-screen__active-label">当前沿用</span>
          <strong>{activeName}</strong>
          {payload?.active && <button className="btn btn--ghost btn--sm" onClick={() => void choose(null)}>取消选用</button>}
        </div>
      </header>

      <div className="style-screen__grid">
        <section className="style-screen__source panel">
          <div className="panel__kicker"><IconBrush size={14} /> 投喂文章</div>
          <h2 className="panel__title">让模型先读懂你的声音</h2>
          <p className="panel__subtitle">建议投喂同一作者的完整片段。分析只提炼规则，不复制原文，也不带入原文剧情。</p>
          <textarea
            className="style-screen__article textarea"
            value={article}
            onChange={(event) => setArticle(event.target.value)}
            placeholder="粘贴文章片段（至少 80 字，最多 8 万字）…"
          />
          <div className="style-screen__source-foot">
            <span>{article.length.toLocaleString()} 字</span>
            <button className="btn btn--primary" onClick={() => void runAnalysis()} disabled={analyzing || article.trim().length < 80}>
              {analyzing ? <Spinner size={15} /> : <IconBrush size={15} />}
              {analyzing ? "正在提炼…" : "分析文风"}
            </button>
          </div>
          <div className="style-screen__principle">
            <IconCheck size={15} />
            <span>启用后，策划、创作、编辑和修订都会先对齐这份档案，并在生成后自检。</span>
          </div>
        </section>

        <section className="style-screen__editor panel">
          <div className="panel__kicker">STYLE PROFILE</div>
          <div className="style-screen__editor-head">
            <div>
              <h2 className="panel__title">{draft ? "编辑文风档案" : "选择或分析一个档案"}</h2>
              <p className="panel__subtitle">规则越具体，长篇连载中的声音越稳定。</p>
            </div>
            {draft && <button className="icon-btn" title="放弃编辑" aria-label="放弃编辑" onClick={() => setDraft(null)}><IconClose size={16} /></button>}
          </div>
          {draft ? (
            <div className="style-form">
              <label className="field"><span className="field__label">档案名称</span><input className="field__input" value={draft.name} onChange={(event) => updateDraft("name", event.target.value)} /></label>
              <label className="field"><span className="field__label">一句话概述</span><textarea className="field__input field__textarea" rows={2} value={draft.description} onChange={(event) => updateDraft("description", event.target.value)} /></label>
              <div className="field-row">
                <label className="field"><span className="field__label">基调与情绪</span><input className="field__input" value={draft.tone} onChange={(event) => updateDraft("tone", event.target.value)} /></label>
                <label className="field"><span className="field__label">叙事人称</span><input className="field__input" value={draft.narrative_person} onChange={(event) => updateDraft("narrative_person", event.target.value)} /></label>
              </div>
              <label className="field"><span className="field__label">节奏</span><input className="field__input" value={draft.pacing} onChange={(event) => updateDraft("pacing", event.target.value)} /></label>
              {RULE_FIELDS.map(([key, label]) => {
                const value = draft[key];
                return <label className="field" key={key}><span className="field__label">{label}</span><textarea className="field__input field__textarea" rows={3} value={Array.isArray(value) ? joinRules(value) : String(value)} onChange={(event) => updateDraft(key, Array.isArray(value) ? splitRules(event.target.value) : event.target.value as never)} placeholder="每行一条规则" /></label>;
              })}
              <div className="style-screen__editor-actions"><button className="btn btn--primary" onClick={() => void save()} disabled={saving || !draft.name.trim()}>{saving ? <Spinner size={15} /> : <IconSave size={15} />}保存档案</button></div>
            </div>
          ) : (
            <div className="style-screen__empty">先在左侧投喂文章开始分析，或从下方已有档案中打开编辑。</div>
          )}
        </section>
      </div>

      <section className="style-screen__library">
        <div className="style-screen__library-head"><div><div className="panel__kicker">SAVED VOICES</div><h2>已保存的文风</h2></div><span className="count-pill">{loading ? "读取中" : `${payload?.profiles.length ?? 0} 份`}</span></div>
        {!loading && payload?.profiles.length === 0 ? <p className="style-screen__library-empty">还没有文风档案。用一段你认可的文字开始。</p> : <div className="style-screen__cards">{payload?.profiles.map((profile) => <article className={`style-card${profile.id === payload.active_id ? " is-active" : ""}`} key={profile.id}><div className="style-card__head"><div><h3>{profile.name}</h3><p>{profile.description || "未填写概述"}</p></div>{profile.id === payload.active_id && <span className="style-card__badge"><IconCheck size={12} /> 当前</span>}</div><div className="style-card__meta">{profile.tone || "未定义基调"} · {profile.narrative_person || "未定义人称"}</div><div className="style-card__actions"><button className="btn btn--primary btn--sm" onClick={() => void choose(profile.id)} disabled={profile.id === payload.active_id}>{profile.id === payload.active_id ? "已启用" : "启用"}</button><button className="btn btn--ghost btn--sm" onClick={() => editProfile(profile)}>编辑规则</button><button className="icon-btn icon-btn--danger" title="删除文风" aria-label="删除文风" onClick={() => void remove(profile)}><IconTrash size={15} /></button></div></article>)}</div>}
      </section>
    </div>
  );
}
