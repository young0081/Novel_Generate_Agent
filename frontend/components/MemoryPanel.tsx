"use client";

import { type CSSProperties, useCallback, useState } from "react";

import Button from "@/components/Button";
import { useConnection } from "@/components/Connection";
import EmptyState from "@/components/EmptyState";
import { useToast } from "@/components/Toast";
import { invokeTool } from "@/lib/rpcClient";

const KINDS = [
  "character",
  "setting",
  "worldbuilding",
  "plot",
  "outline",
  "foreshadow",
  "dialogue",
  "lore",
  "other",
] as const;

const KIND_LABEL: Record<string, string> = {
  character: "人物",
  setting: "场景设定",
  worldbuilding: "世界观",
  plot: "情节",
  outline: "大纲",
  foreshadow: "伏笔",
  dialogue: "对话",
  lore: "传说设定",
  other: "其他",
};

export default function MemoryPanel() {
  const toast = useToast();
  const { reportError } = useConnection();

  const [kind, setKind] = useState("character");
  const [title, setTitle] = useState("");
  const [summary, setSummary] = useState("");
  const [content, setContent] = useState("");
  const [tags, setTags] = useState("");
  const [importance, setImportance] = useState(3);

  const [query, setQuery] = useState("");
  const [hits, setHits] = useState("");
  const [searched, setSearched] = useState(false);
  const [saving, setSaving] = useState(false);
  const [recalling, setRecalling] = useState(false);

  const fail = useCallback(
    (e: unknown) => {
      const message = e instanceof Error ? e.message : String(e);
      reportError(message);
      toast.error("错误：" + message);
    },
    [reportError, toast],
  );

  const save = useCallback(async () => {
    setSaving(true);
    try {
      const r = await invokeTool("memory_save", {
        kind,
        title,
        summary,
        content,
        tags: tags
          .split(/[,，]/)
          .map((t) => t.trim())
          .filter(Boolean),
        importance,
      });
      if (r.ok) {
        toast.success(`已保存：${r.summary ?? r.content}`);
      } else {
        toast.error(r.content);
      }
    } catch (e) {
      fail(e);
    } finally {
      setSaving(false);
    }
  }, [kind, title, summary, content, tags, importance, fail, toast]);

  const recall = useCallback(async () => {
    setRecalling(true);
    try {
      const r = await invokeTool("memory_recall", { query, k: 8 });
      setHits(r.content || "");
      setSearched(true);
    } catch (e) {
      fail(e);
    } finally {
      setRecalling(false);
    }
  }, [query, fail]);

  return (
    <div>
      <div className="card" style={{ "--i": 0 } as CSSProperties}>
        <div className="card-head">
          <h3>检索记忆（RAG）</h3>
          <span className="hint">中文友好 BM25 · 只返回结构化摘要</span>
        </div>
        <div className="row">
          <input
            className="grow"
            placeholder="搜索，例如：北境 剑客 主角"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && !recalling && void recall()}
          />
          <Button onClick={recall} loading={recalling}>
            检索
          </Button>
        </div>
        {searched &&
          (hits ? (
            <pre className="output">{hits}</pre>
          ) : (
            <EmptyState
              icon="🔍"
              title="没有匹配的记忆"
              hint="换个关键词试试，或先在下面新增一条记忆。"
            />
          ))}
      </div>

      <div className="card" style={{ "--i": 1 } as CSSProperties}>
        <div className="card-head">
          <h3>新增记忆</h3>
        </div>
        <div className="row">
          <label className="field" style={{ width: 140 }}>
            <span>类型</span>
            <select value={kind} onChange={(e) => setKind(e.target.value)}>
              {KINDS.map((k) => (
                <option key={k} value={k}>
                  {KIND_LABEL[k]}
                </option>
              ))}
            </select>
          </label>
          <label className="field grow">
            <span>标题</span>
            <input value={title} onChange={(e) => setTitle(e.target.value)} placeholder="例如：林惊羽" />
          </label>
          <label className="field" style={{ width: 130 }}>
            <span>重要度 {importance}</span>
            <input
              type="range"
              min={1}
              max={5}
              value={importance}
              onChange={(e) => setImportance(Number(e.target.value))}
            />
          </label>
        </div>
        <label className="field">
          <span>一句话摘要（检索时展示）</span>
          <input value={summary} onChange={(e) => setSummary(e.target.value)} placeholder="冷静果敢的年轻剑客，主角。" />
        </label>
        <label className="field">
          <span>详细内容（仅在显式取用时才返回全文）</span>
          <textarea value={content} onChange={(e) => setContent(e.target.value)} />
        </label>
        <label className="field">
          <span>标签（逗号分隔）</span>
          <input value={tags} onChange={(e) => setTags(e.target.value)} placeholder="主角, 剑客, 北境" />
        </label>
        <Button onClick={save} loading={saving} disabled={!title}>
          保存记忆
        </Button>
      </div>
    </div>
  );
}
