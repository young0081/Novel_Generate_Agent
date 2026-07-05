// 协作 / 版本 — async team collaboration via the fiction version-control tools.
//
// A team can point several clients at one shared backend/workspace and work
// asynchronously: each writer commits snapshots of the manuscript, branches to
// explore alternate plot lines, compares revisions, and restores when needed.
//
// Backed by the vcs_* tools through `invokeTool` (core.ts):
//   vcs_commit / vcs_log / vcs_diff / vcs_branch / vcs_restore

import { useCallback, useEffect, useMemo, useState } from "react";
import Panel from "../components/Panel";
import { LoadingBlock, Spinner } from "../components/Spinner";
import EmptyState from "../components/EmptyState";
import ConfirmModal from "../components/ConfirmModal";
import {
  IconRefresh,
  IconCommit,
  IconBranch,
  IconDiff,
  IconRestore,
  IconPlus,
  IconInfo,
  IconUsers,
} from "../components/icons";
import { invokeTool, describeError } from "../lib/core";
import { useToast } from "../components/Toast";

// ---- loosely-typed views over the tool `data` payloads ----
interface CommitData {
  id?: string;
  total_words?: number;
  [k: string]: unknown;
}

interface LogEntry {
  id?: string;
  message?: string;
  author?: string;
  time?: string | number;
  created_ms?: number;
  word_delta?: number;
  words?: number;
  total_words?: number;
  [k: string]: unknown;
}

interface BranchData {
  action?: string;
  current?: string;
  branches?: string[];
  [k: string]: unknown;
}

/** Pull a commit list out of whatever shape the core returns. */
function pickLog(data: unknown): LogEntry[] {
  if (Array.isArray(data)) return data as LogEntry[];
  if (data && typeof data === "object") {
    const o = data as Record<string, unknown>;
    for (const key of ["log", "commits", "entries", "history", "items"]) {
      if (Array.isArray(o[key])) return o[key] as LogEntry[];
    }
  }
  return [];
}

function fmtWhen(v: LogEntry["time"] | number | undefined): string {
  if (v == null) return "";
  let d: Date;
  if (typeof v === "number") {
    d = new Date(v > 1e12 ? v : v * 1000);
  } else {
    const n = Number(v);
    d =
      Number.isFinite(n) && String(v).trim() !== ""
        ? new Date(n > 1e12 ? n : n * 1000)
        : new Date(v);
  }
  if (Number.isNaN(d.getTime())) return String(v);
  return d.toLocaleString("zh-CN", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function fmtDelta(n: number | undefined): { text: string; cls: string } | null {
  if (typeof n !== "number" || !Number.isFinite(n) || n === 0) {
    if (n === 0) return { text: "±0 字", cls: "is-flat" };
    return null;
  }
  return n > 0
    ? { text: `+${n} 字`, cls: "is-add" }
    : { text: `${n} 字`, cls: "is-del" };
}

function shortId(id: string | undefined): string {
  if (!id) return "";
  return id.length > 10 ? id.slice(0, 10) : id;
}

/** Classify one diff line for ink-coloured rendering. */
type DiffLineKind = "add" | "del" | "hunk" | "meta" | "ctx";
function classifyDiffLine(line: string): DiffLineKind {
  if (line.startsWith("@@")) return "hunk";
  if (/^(diff |index |--- |\+\+\+ |File:|文件[:：]|===)/.test(line)) {
    return "meta";
  }
  if (line.startsWith("+")) return "add";
  if (line.startsWith("-")) return "del";
  return "ctx";
}

/**
 * Render a diff/patch text with line-level ink colouring: additions in jade,
 * deletions in cinnabar, hunk markers tinted, everything else calm ink. Falls
 * back gracefully for non-patch text (a plain word-delta summary still reads
 * fine as context lines).
 */
function DiffView({ text }: { text: string }) {
  const lines = text.replace(/\n$/, "").split("\n");
  let adds = 0;
  let dels = 0;
  for (const l of lines) {
    const k = classifyDiffLine(l);
    if (k === "add") adds++;
    else if (k === "del") dels++;
  }
  const looksLikePatch = adds + dels > 0;
  return (
    <div className="diffview">
      {looksLikePatch && (
        <div className="diffview__stat">
          <span className="diffview__stat-add">+{adds}</span>
          <span className="diffview__stat-del">−{dels}</span>
          <span className="diffview__stat-lbl">行变动</span>
        </div>
      )}
      <pre className="diffview__body">
        {lines.map((l, i) => {
          const kind = classifyDiffLine(l);
          const sign = kind === "add" ? "+" : kind === "del" ? "−" : " ";
          // the marker char is shown in the sign column, so drop it from the
          // text for add/del lines (avoids a doubled "++"/"--").
          const body =
            kind === "add" || kind === "del" ? l.slice(1) : l;
          return (
            <span className={`diffline diffline--${kind}`} key={i}>
              <span className="diffline__sign" aria-hidden="true">
                {sign}
              </span>
              <span className="diffline__text">{body || " "}</span>
            </span>
          );
        })}
      </pre>
    </div>
  );
}

export default function CollabScreen() {
  const toast = useToast();

  // commit
  const [commitMsg, setCommitMsg] = useState("");
  const [committing, setCommitting] = useState(false);

  // log
  const [log, setLog] = useState<LogEntry[]>([]);
  const [logText, setLogText] = useState("");
  const [loadingLog, setLoadingLog] = useState(true);
  const [logError, setLogError] = useState<string | null>(null);

  // branches
  const [branches, setBranches] = useState<string[]>([]);
  const [current, setCurrent] = useState<string | null>(null);
  const [loadingBranches, setLoadingBranches] = useState(true);
  const [branchError, setBranchError] = useState<string | null>(null);
  const [newBranch, setNewBranch] = useState("");
  const [branchBusy, setBranchBusy] = useState(false);
  const [switchingTo, setSwitchingTo] = useState<string | null>(null);

  // diff
  const [diffId, setDiffId] = useState("");
  const [diffText, setDiffText] = useState<string | null>(null);
  const [diffError, setDiffError] = useState<string | null>(null);
  const [diffing, setDiffing] = useState(false);

  // restore
  const [restoreTarget, setRestoreTarget] = useState<LogEntry | null>(null);
  const [restoring, setRestoring] = useState(false);

  // ---- loaders ----
  const loadLog = useCallback(async () => {
    setLoadingLog(true);
    setLogError(null);
    try {
      const res = await invokeTool<unknown>("vcs_log", {});
      if (!res.ok) {
        setLogError(res.content || "无法读取提交历史");
        setLog([]);
        setLogText("");
        return;
      }
      setLog(pickLog(res.data));
      setLogText(res.content || "");
    } catch (e) {
      setLogError(describeError(e));
      setLog([]);
      setLogText("");
    } finally {
      setLoadingLog(false);
    }
  }, []);

  const loadBranches = useCallback(async () => {
    setLoadingBranches(true);
    setBranchError(null);
    try {
      const res = await invokeTool<BranchData>("vcs_branch", {
        action: "list",
      });
      if (!res.ok) {
        setBranchError(res.content || "无法读取分支");
        setBranches([]);
        setCurrent(null);
        return;
      }
      const d = res.data ?? {};
      setBranches(Array.isArray(d.branches) ? d.branches : []);
      setCurrent(d.current ?? null);
    } catch (e) {
      setBranchError(describeError(e));
      setBranches([]);
      setCurrent(null);
    } finally {
      setLoadingBranches(false);
    }
  }, []);

  const refreshAll = useCallback(async () => {
    await Promise.all([loadLog(), loadBranches()]);
  }, [loadLog, loadBranches]);

  useEffect(() => {
    void refreshAll();
  }, [refreshAll]);

  // ---- actions ----
  const commit = useCallback(async () => {
    const msg = commitMsg.trim();
    if (!msg) {
      toast.err("请先写一句提交说明");
      return;
    }
    setCommitting(true);
    try {
      const res = await invokeTool<CommitData>("vcs_commit", { message: msg });
      if (!res.ok) {
        toast.err(res.content || "提交失败");
        return;
      }
      const id = res.data?.id;
      const words = res.data?.total_words;
      const idPart = id ? `#${shortId(id)}` : "";
      const wordPart =
        typeof words === "number" ? ` · 全文 ${words} 字` : "";
      toast.ok(`已提交快照 ${idPart}${wordPart}`.trim());
      setCommitMsg("");
      await loadLog();
    } catch (e) {
      toast.err(describeError(e));
    } finally {
      setCommitting(false);
    }
  }, [commitMsg, loadLog, toast]);

  const createBranch = useCallback(async () => {
    const name = newBranch.trim();
    if (!name) {
      toast.err("请填写新分支名称");
      return;
    }
    setBranchBusy(true);
    try {
      const res = await invokeTool<BranchData>("vcs_branch", {
        action: "create",
        name,
      });
      if (!res.ok) {
        toast.err(res.content || "创建分支失败");
        return;
      }
      toast.ok(`已创建分支「${name}」`);
      setNewBranch("");
      await loadBranches();
    } catch (e) {
      toast.err(describeError(e));
    } finally {
      setBranchBusy(false);
    }
  }, [newBranch, loadBranches, toast]);

  const switchBranch = useCallback(
    async (name: string) => {
      if (name === current) return;
      setSwitchingTo(name);
      try {
        const res = await invokeTool<BranchData>("vcs_branch", {
          action: "switch",
          name,
        });
        if (!res.ok) {
          toast.err(res.content || "切换分支失败");
          return;
        }
        toast.ok(`已切换到「${name}」`);
        await refreshAll();
      } catch (e) {
        toast.err(describeError(e));
      } finally {
        setSwitchingTo(null);
      }
    },
    [current, refreshAll, toast],
  );

  const runDiff = useCallback(
    async (rawId?: string) => {
      const id = (rawId ?? diffId).trim();
      if (!id) {
        toast.err("请填写要对比的提交 id");
        return;
      }
      setDiffId(id);
      setDiffing(true);
      setDiffError(null);
      setDiffText(null);
      try {
        const res = await invokeTool<unknown>("vcs_diff", { id });
        if (!res.ok) {
          setDiffError(res.content || "无法生成对比");
          return;
        }
        setDiffText(res.content || "（这次提交没有文本变化）");
      } catch (e) {
        setDiffError(describeError(e));
      } finally {
        setDiffing(false);
      }
    },
    [diffId, toast],
  );

  const restore = useCallback(async () => {
    if (!restoreTarget?.id) return;
    setRestoring(true);
    try {
      const res = await invokeTool("vcs_restore", { id: restoreTarget.id });
      if (!res.ok) {
        toast.err(res.content || "回滚失败");
        return;
      }
      toast.ok("已回滚到该提交");
      setRestoreTarget(null);
      await loadLog();
    } catch (e) {
      toast.err(describeError(e));
    } finally {
      setRestoring(false);
    }
  }, [restoreTarget, loadLog, toast]);

  const ordered = useMemo(() => {
    // newest first; the core usually appends, so reverse a shallow copy
    return log.slice().reverse();
  }, [log]);

  const headerActions = (
    <button
      className="btn btn--ghost btn--icon"
      onClick={() => void refreshAll()}
      title="刷新"
      aria-label="刷新"
    >
      <IconRefresh size={16} />
    </button>
  );

  return (
    <Panel
      title="协作"
      en="Collaboration"
      subtitle="版本溯流 · 提交 / 分支 / 对比 / 回滚，支持团队异步共笔"
      actions={headerActions}
    >
      <div className="scroll-area">
        {/* team note */}
        <div className="banner banner--info" style={{ margin: "0 0 18px" }}>
          <IconUsers size={16} />
          团队可让多台客户端连到同一个后端 / 工作区，各自落笔、提交与切换分支，异步协作、互不打断。
        </div>

        <div className="collab-grid">
          {/* ===== commit ===== */}
          <section className="collab-card collab-card--commit">
            <div className="collab-card__head">
              <span className="collab-card__title">
                <IconCommit size={16} />
                提交快照
              </span>
            </div>
            <p className="collab-card__desc">
              为当前书稿留一个带说明的存档点，队友随时可见。
            </p>
            <div className="collab-commit__row">
              <input
                className="input"
                placeholder="这次写了什么？如：完成第三章·林惊羽夜入北境"
                value={commitMsg}
                onChange={(e) => setCommitMsg(e.target.value)}
                disabled={committing}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void commit();
                }}
              />
              <button
                className="btn btn--primary"
                onClick={() => void commit()}
                disabled={committing}
              >
                {committing ? <Spinner size={15} /> : <IconCommit size={16} />}
                提交快照
              </button>
            </div>
          </section>

          {/* ===== branches ===== */}
          <section className="collab-card collab-card--branch">
            <div className="collab-card__head">
              <span className="collab-card__title">
                <IconBranch size={16} />
                分支
                {!loadingBranches && branches.length > 0 && (
                  <span className="count-pill">{branches.length} 条</span>
                )}
              </span>
              {current && (
                <span className="chip chip--accent" title="当前所在分支">
                  <IconBranch size={11} />
                  {current}
                </span>
              )}
            </div>
            <p className="collab-card__desc">
              分支让队友各自探索不同的剧情走向，互不影响，日后再择优合流。
            </p>
            <div className="collab-commit__row">
              <input
                className="input"
                placeholder="新分支名，如：结局B-双双归隐"
                value={newBranch}
                onChange={(e) => setNewBranch(e.target.value)}
                disabled={branchBusy}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void createBranch();
                }}
              />
              <button
                className="btn"
                onClick={() => void createBranch()}
                disabled={branchBusy}
              >
                {branchBusy ? <Spinner size={15} /> : <IconPlus size={16} />}
                新建分支
              </button>
            </div>

            <div className="collab-branches">
              {loadingBranches ? (
                <div className="collab-branches__loading">
                  <Spinner size={16} />
                  正在读取分支…
                </div>
              ) : branchError ? (
                <div className="banner banner--warn" style={{ margin: "4px 0 0" }}>
                  {branchError}
                </div>
              ) : branches.length === 0 ? (
                <span className="collab-empty-line">
                  暂无分支记录，新建一个分支开始分头创作。
                </span>
              ) : (
                branches.map((b) => {
                  const isCurrent = b === current;
                  const isSwitching = switchingTo === b;
                  return (
                    <button
                      key={b}
                      className={`branch-chip${isCurrent ? " is-current" : ""}`}
                      onClick={() => void switchBranch(b)}
                      disabled={isCurrent || switchingTo !== null}
                      title={isCurrent ? "当前分支" : `切换到 ${b}`}
                    >
                      {isSwitching ? (
                        <Spinner size={12} />
                      ) : (
                        <IconBranch size={12} />
                      )}
                      <span className="branch-chip__name">{b}</span>
                      {isCurrent && (
                        <span className="branch-chip__cur">当前</span>
                      )}
                    </button>
                  );
                })
              )}
            </div>
          </section>

          {/* ===== diff ===== */}
          <section className="collab-card collab-card--diff">
            <div className="collab-card__head">
              <span className="collab-card__title">
                <IconDiff size={16} />
                对比改动
              </span>
              {diffText !== null && !diffError && diffId.trim() && (
                <span className="chip chip--jade" title="正在对比的提交">
                  <IconCommit size={11} />
                  {shortId(diffId.trim())}
                </span>
              )}
            </div>
            <p className="collab-card__desc">
              输入某次提交的 id，查看它相对上一版的逐行改动与字数增减。
            </p>
            <div className="collab-commit__row">
              <input
                className="input"
                placeholder="提交 id（可从右侧历史点「对比」自动填入）"
                value={diffId}
                onChange={(e) => setDiffId(e.target.value)}
                disabled={diffing}
                spellCheck={false}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void runDiff();
                }}
              />
              <button
                className="btn"
                onClick={() => void runDiff()}
                disabled={diffing}
              >
                {diffing ? <Spinner size={15} /> : <IconDiff size={16} />}
                对比
              </button>
            </div>
            {diffError ? (
              <div className="banner banner--warn" style={{ margin: "10px 0 0" }}>
                {diffError}
              </div>
            ) : diffing ? (
              <div className="collab-branches__loading" style={{ marginTop: 12 }}>
                <Spinner size={16} />
                正在比对墨迹…
              </div>
            ) : diffText !== null ? (
              <DiffView text={diffText} />
            ) : (
              <p className="field__hint">
                <IconInfo size={12} className="field__hint-icon" />
                对比结果会以行级差异呈现：新增、删除与字数变化一目了然。
              </p>
            )}
          </section>
        </div>

        {/* ===== history ===== */}
        <section className="collab-history">
          <div className="collab-history__head">
            <span className="collab-card__title">
              <IconCommit size={16} />
              提交历史
            </span>
            {!loadingLog && (
              <span className="count-pill">{log.length} 次提交</span>
            )}
          </div>

          {loadingLog ? (
            <LoadingBlock label="正在翻检提交历史…" />
          ) : logError ? (
            <div className="banner banner--warn">{logError}</div>
          ) : ordered.length === 0 ? (
            logText.trim() ? (
              // some cores return only a readable text log — show it verbatim
              <pre className="collab-diff">{logText}</pre>
            ) : (
              <EmptyState
                title="尚无提交"
                text="还没有任何快照。在上方写一句说明并「提交快照」，便能留下第一个存档点。"
              />
            )
          ) : (
            <div className="timeline">
              {ordered.map((c, i) => {
                const delta = fmtDelta(c.word_delta);
                const when = fmtWhen(c.time ?? c.created_ms);
                const id = c.id;
                const isNewest = i === 0;
                const isDiffed =
                  diffText !== null &&
                  !diffError &&
                  !!id &&
                  id.trim() === diffId.trim();
                const totalWords =
                  typeof c.total_words === "number"
                    ? c.total_words
                    : typeof c.words === "number"
                      ? c.words
                      : null;
                return (
                  <div
                    className={`cp${isDiffed ? " is-diffed" : ""}`}
                    key={id || i}
                  >
                    <div className="cp__rail">
                      <span className="cp__node" />
                    </div>
                    <div className="cp__card">
                      <div style={{ minWidth: 0 }}>
                        <div className="cp__label">
                          {isNewest && (
                            <span className="cp__latest">最新</span>
                          )}
                          {c.message || "（无说明）"}
                        </div>
                        <div className="cp__meta">
                          {c.author && (
                            <>
                              <IconUsers
                                size={12}
                                style={{ verticalAlign: "-2px", marginRight: 4 }}
                              />
                              {c.author}
                              <span style={{ margin: "0 8px", opacity: 0.5 }}>
                                ·
                              </span>
                            </>
                          )}
                          {when || "时间未知"}
                          {id && (
                            <>
                              <span style={{ margin: "0 8px", opacity: 0.5 }}>
                                ·
                              </span>
                              <code style={{ fontSize: 11 }}>{shortId(id)}</code>
                            </>
                          )}
                          {delta && (
                            <span className={`word-delta ${delta.cls}`}>
                              {delta.text}
                            </span>
                          )}
                          {totalWords != null && (
                            <span className="cp__total">
                              全文 {totalWords} 字
                            </span>
                          )}
                        </div>
                      </div>
                      <div className="collab-actions">
                        <button
                          className={`btn btn--sm${isDiffed ? " is-active" : ""}`}
                          onClick={() => void runDiff(id)}
                          disabled={!id}
                          title="查看相对上一版的改动"
                        >
                          <IconDiff size={14} />
                          对比
                        </button>
                        <button
                          className="btn btn--danger btn--sm"
                          onClick={() => setRestoreTarget(c)}
                          disabled={!id}
                          title="把书稿回滚到这次提交"
                        >
                          <IconRestore size={14} />
                          回滚
                        </button>
                      </div>
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </section>
      </div>

      <ConfirmModal
        open={restoreTarget !== null}
        title="回滚到此提交？"
        sealChar="溯"
        danger
        busy={restoring}
        confirmLabel="确认回滚"
        body={
          <>
            书稿将被还原到提交
            <br />
            <code>
              {restoreTarget?.message || shortId(restoreTarget?.id) || "（该提交）"}
            </code>
            <br />
            当前未提交的改动可能会丢失，建议先「提交快照」再回滚。
          </>
        }
        onConfirm={() => void restore()}
        onCancel={() => {
          if (!restoring) setRestoreTarget(null);
        }}
      />
    </Panel>
  );
}
