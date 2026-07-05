"use client";

import { type CSSProperties, useCallback, useEffect, useRef, useState } from "react";

import Button from "@/components/Button";
import { useConnection } from "@/components/Connection";
import EmptyState from "@/components/EmptyState";
import { useToast } from "@/components/Toast";
import { invokeTool } from "@/lib/rpcClient";

type Job = "list" | "open" | "save" | null;

export default function FileWorkbench() {
  const toast = useToast();
  const { reportError } = useConnection();

  const [dirPath, setDirPath] = useState("");
  const [listing, setListing] = useState("");
  const [listed, setListed] = useState(false);
  const [filePath, setFilePath] = useState("book/ch1.md");
  const [content, setContent] = useState("");
  const [job, setJob] = useState<Job>(null);

  const fail = useCallback(
    (e: unknown) => {
      const message = e instanceof Error ? e.message : String(e);
      reportError(message);
      toast.error("错误：" + message);
    },
    [reportError, toast],
  );

  const doList = useCallback(async () => {
    setJob("list");
    try {
      const r = await invokeTool("list_dir", { path: dirPath });
      setListing(r.content || "");
      setListed(true);
    } catch (e) {
      fail(e);
    } finally {
      setJob(null);
    }
  }, [dirPath, fail]);

  const doOpen = useCallback(async () => {
    setJob("open");
    try {
      const r = await invokeTool("read_file", { path: filePath });
      if (!r.ok) {
        toast.error(r.content);
      } else {
        setContent(r.content);
        toast.success(`已打开 ${filePath}（${r.metadata.bytes} 字节）`);
      }
    } catch (e) {
      fail(e);
    } finally {
      setJob(null);
    }
  }, [filePath, fail, toast]);

  const doSave = useCallback(async () => {
    setJob("save");
    try {
      const r = await invokeTool("write_file", { path: filePath, content });
      if (r.ok) {
        toast.success(`已保存：${r.summary ?? r.content}`);
      } else {
        toast.error(r.content);
      }
    } catch (e) {
      fail(e);
    } finally {
      setJob(null);
    }
  }, [filePath, content, fail, toast]);

  // Keep a live ref so the keydown handler always saves the latest content.
  const saveRef = useRef(doSave);
  saveRef.current = doSave;
  const busyRef = useRef<Job>(job);
  busyRef.current = job;

  // Ctrl/Cmd+S saves the open file.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        if (busyRef.current === null) void saveRef.current();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const busy = job !== null;

  return (
    <div>
      <div className="card" style={{ "--i": 0 } as CSSProperties}>
        <div className="card-head">
          <h3>目录浏览</h3>
        </div>
        <div className="row">
          <input
            className="grow"
            placeholder="目录路径（留空=工作区根目录），例如 book"
            value={dirPath}
            onChange={(e) => setDirPath(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && !busy && void doList()}
          />
          <Button onClick={doList} loading={job === "list"} disabled={busy}>
            列目录
          </Button>
        </div>
        {listed &&
          (listing ? (
            <pre className="output">{listing}</pre>
          ) : (
            <EmptyState icon="📂" title="这个目录是空的" hint="还没有文件，换个路径或先去保存一篇稿件。" />
          ))}
      </div>

      <div className="card" style={{ "--i": 1 } as CSSProperties}>
        <div className="card-head">
          <h3>编辑文件</h3>
          <span className="hint">提示：编辑时按 Ctrl/Cmd + S 可快速保存</span>
        </div>
        <div className="row">
          <input
            className="grow"
            placeholder="文件路径，例如 book/ch1.md"
            value={filePath}
            onChange={(e) => setFilePath(e.target.value)}
          />
          <Button variant="ghost" onClick={doOpen} loading={job === "open"} disabled={busy}>
            打开
          </Button>
          <Button onClick={doSave} loading={job === "save"} disabled={busy}>
            保存
          </Button>
        </div>
        <textarea
          style={{ marginTop: 12, minHeight: 280 }}
          placeholder="文件内容…"
          value={content}
          onChange={(e) => setContent(e.target.value)}
        />
      </div>
    </div>
  );
}
