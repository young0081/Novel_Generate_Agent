// 工具 — list every tool the Rust core exposes, as paper-slip cards.

import { useCallback, useEffect, useMemo, useState } from "react";
import Panel from "../components/Panel";
import { SkeletonGrid } from "../components/Skeleton";
import EmptyState from "../components/EmptyState";
import { IconRefresh, IconSearch, IconTools } from "../components/icons";
import { listTools, describeError, type ToolSpec } from "../lib/core";

function capLabel(cap: string): string {
  const map: Record<string, string> = {
    fs: "文件",
    read: "读取",
    write: "写入",
    net: "网络",
    network: "网络",
    vcs: "版本",
    memory: "记忆",
    exec: "执行",
    process: "进程",
    web: "联网",
  };
  return map[cap] ?? cap;
}

export default function ToolsScreen() {
  const [tools, setTools] = useState<ToolSpec[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const list = await listTools();
      setTools(list);
    } catch (e) {
      setError(describeError(e));
      setTools([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return tools;
    return tools.filter(
      (t) =>
        t.name.toLowerCase().includes(q) ||
        t.description.toLowerCase().includes(q),
    );
  }, [tools, query]);

  const mutating = tools.filter((t) => t.mutating).length;

  const headerActions = (
    <button
      className="btn btn--ghost btn--icon"
      onClick={() => void refresh()}
      title="刷新"
      aria-label="刷新"
    >
      <IconRefresh size={16} />
    </button>
  );

  const toolbar = (
    <div className="toolbar">
      <div className="search-box">
        <IconSearch size={16} />
        <input
          className="input"
          placeholder="搜索工具名称或说明…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </div>
      <span className="chip chip--accent">写 {mutating}</span>
      <span className="chip chip--jade">只读 {tools.length - mutating}</span>
      <span className="count-pill">{filtered.length} 个工具</span>
    </div>
  );

  return (
    <Panel
      title="工具"
      en="Tools"
      subtitle="创作核心对外开放的能力清单 · 由 Rust 引擎驱动"
      actions={headerActions}
      toolbar={toolbar}
    >
      <div className="scroll-area">
        {loading ? (
          <SkeletonGrid count={9} />
        ) : error ? (
          <div className="banner banner--warn">{error}</div>
        ) : filtered.length === 0 ? (
          <EmptyState
            title="未找到工具"
            text={
              tools.length === 0
                ? "核心暂未上报任何工具，请确认已在桌面应用中运行。"
                : "没有匹配的工具，换个关键词试试。"
            }
          />
        ) : (
          <div className="slip-grid">
            {filtered.map((t) => (
              <article className="tool-card" key={t.name}>
                <div className="tool-card__head">
                  <span className="tool-card__name">
                    <IconTools
                      size={14}
                      style={{ verticalAlign: "-2px", marginRight: 6, opacity: 0.6 }}
                    />
                    {t.name}
                  </span>
                  <span
                    className={`badge ${t.mutating ? "badge--write" : "badge--read"}`}
                  >
                    {t.mutating ? "写" : "只读"}
                  </span>
                </div>
                <p className="tool-card__desc">
                  {t.description || <span className="muted">（暂无说明）</span>}
                </p>
                {t.capabilities.length > 0 && (
                  <div className="tool-card__chips">
                    {t.capabilities.map((c) => (
                      <span className="chip chip--jade" key={c}>
                        {capLabel(c)}
                      </span>
                    ))}
                  </div>
                )}
              </article>
            ))}
          </div>
        )}
      </div>
    </Panel>
  );
}
