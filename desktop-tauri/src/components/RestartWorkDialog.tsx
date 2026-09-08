import { useCallback, useRef, useState } from "react";
import { describeError } from "../lib/core";
import { useDialogFocus } from "../lib/dialogLayer";
import { acquireEditorWriteBarrier, flushPendingEditors, tryAcquireWorkspaceAction } from "../lib/editorPersistence";
import { restartWork, type WorkSummary } from "../lib/works";
import { IconClose, IconRefresh } from "./icons";
import { Spinner } from "./Spinner";
import { useToast } from "./Toast";

interface Props {
  source: WorkSummary;
  onClose: () => void;
  onCreated: () => Promise<void>;
}

export default function RestartWorkDialog({ source, onClose, onCreated }: Props) {
  const [title, setTitle] = useState(`${source.title}（重开）`);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const busyRef = useRef(false);
  const dialogRef = useRef<HTMLDivElement>(null);
  const toast = useToast();
  const close = useCallback(() => {
    if (!busyRef.current) onClose();
  }, [onClose]);
  useDialogFocus(true, dialogRef, close);

  const submit = async () => {
    if (busyRef.current || !title.trim()) return;
    const releaseAction = tryAcquireWorkspaceAction();
    if (!releaseAction) {
      setError("请等待当前文件操作完成后再重开。");
      return;
    }
    const releaseBarrier = acquireEditorWriteBarrier();
    busyRef.current = true;
    setBusy(true);
    setError(null);
    try {
      if (!(await flushPendingEditors())) throw new Error("当前编辑内容保存失败，请保存后再重开。");
      const result = await restartWork(source.id, title.trim());
      await onCreated();
      toast.ok(`已切换到《${result.work.title}》，保留 ${result.report.memory_count} 条记忆、${result.report.knowledge_base_count} 个知识库。可从第一章开始创作。`);
      onClose();
    } catch (failure) {
      setError(describeError(failure));
    } finally {
      busyRef.current = false;
      setBusy(false);
      releaseBarrier();
      releaseAction();
    }
  };

  return (
    <div className="library__overlay" onClick={close}>
      <div ref={dialogRef} className="library__sheet" role="dialog" aria-modal="true"
        aria-labelledby="restart-work-title" aria-describedby="restart-work-description" tabIndex={-1}
        onClick={(event) => event.stopPropagation()}>
        <header className="library__sheet-head">
          <h3 id="restart-work-title">保留记忆重开</h3>
          <button className="icon-btn" aria-label="关闭" disabled={busy} onClick={close}><IconClose size={16} /></button>
        </header>
        <form onSubmit={(event) => { event.preventDefault(); void submit(); }}>
          <div className="library__sheet-body">
            <p id="restart-work-description" className="library__restart-description">
              从《{source.title}》新建一部独立作品，原作品完整保留。
            </p>
            <label className="field">
              <span className="field__label">新作品名称 *</span>
              <input className="field__input" value={title} onChange={(event) => setTitle(event.target.value)}
                required disabled={busy} data-autofocus />
            </label>
            <dl className="library__restart-details">
              <div><dt>保留资料</dt><dd>记忆、知识库、大纲、文风，以及常用设定文档。</dd></div>
              <div><dt>重新开始</dt><dd>新书从第一章创作；旧章节、历史会话和快照留在原作品中。</dd></div>
              <div><dt>剧情参考</dt><dd>原剧情状态保存为参考记忆和文本，供 AI 查阅；新书重新记录剧情进度。</dd></div>
            </dl>
            {error && <p className="banner banner--warn" role="alert">重开失败：{error}</p>}
            {busy && <p className="library__hint" role="status">正在复制资料并创建新作品，请稍候…</p>}
          </div>
          <footer className="library__sheet-foot">
            <button type="button" className="btn btn--ghost" disabled={busy} onClick={close}>取消</button>
            <button type="submit" className="btn btn--primary" disabled={busy || !title.trim()}>
              {busy ? <Spinner size={14} /> : <IconRefresh size={16} />}创建并切换
            </button>
          </footer>
        </form>
      </div>
    </div>
  );
}
