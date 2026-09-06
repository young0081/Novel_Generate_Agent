// Custom title bar for the frameless window.
// The brand block intentionally aligns with the full navigation rail below.

import { useEffect, useState } from "react";
import { WinMin, WinMax, WinRestore, WinCloseIcon } from "./icons";
import { useWork } from "./WorkContext";
import { Spinner } from "./Spinner";
import {
  minimizeWindow,
  toggleMaximizeWindow,
  closeWindow,
  isWindowMaximized,
  onWindowResized,
  guardWindowClose,
  onWindowCloseState,
  type WindowCloseState,
} from "../lib/window";

export type Health = "checking" | "ok" | "offline";

function healthDotClass(h: Health): string {
  switch (h) {
    case "ok":       return "titlebar__dot titlebar__dot--ok";
    case "offline":  return "titlebar__dot titlebar__dot--error";
    default:         return "titlebar__dot titlebar__dot--check";
  }
}

function healthLabel(h: Health): string {
  switch (h) {
    case "ok":       return "已就绪";
    case "offline":  return "浏览器预览";
    default:         return "连接中…";
  }
}

export default function TitleBar({ health }: { health: Health }) {
  const [maximized, setMaximized] = useState(false);
  const [closeState, setCloseState] = useState<WindowCloseState>("idle");
  const { current } = useWork();

  useEffect(() => {
    let alive = true;
    void isWindowMaximized().then((v) => { if (alive) setMaximized(v); });
    const unlisten = onWindowResized(async () => {
      const v = await isWindowMaximized();
      if (alive) setMaximized(v);
    });
    const unlistenClose = guardWindowClose();
    const unlistenCloseState = onWindowCloseState(setCloseState);
    return () => {
      alive = false;
      void unlisten.then((fn) => fn());
      void unlistenClose.then((fn) => fn());
      unlistenCloseState();
    };
  }, []);

  const closing = closeState === "preparing";
  const statusText = closing
    ? "正在保存并关闭…"
    : closeState === "failed"
      ? "关闭失败，可重试"
      : healthLabel(health);

  return (
    <header className="titlebar" data-tauri-drag-region aria-busy={closing || undefined}>
      {/* Brand — aligns with 64px nav sidebar */}
      <div className="titlebar__brand">
        <img className="brand-mark brand-mark--titlebar" src="/icon.png" alt="" aria-hidden="true" />
        <span className="titlebar__brand-copy">
          <strong>墨 · 创作</strong>
          <small>STORY WORKBENCH</small>
        </span>
      </div>

      {/* Draggable title area */}
      <div className="titlebar__drag" data-tauri-drag-region>
        <span className="titlebar__title">墨·创作</span>
        {current && (
          <span className="titlebar__work">
            <span className="titlebar__sep">—</span>
            {current.title}
          </span>
        )}
      </div>

      {/* Status */}
      <div className="titlebar__status" data-tauri-drag-region>
        <span className={healthDotClass(closing ? "checking" : closeState === "failed" ? "offline" : health)} />
        <span className="titlebar__status-text" role="status" aria-live="polite">{statusText}</span>
      </div>

      {/* Window controls */}
      <div className="titlebar__controls">
        <button className="titlebar__btn" onClick={() => void minimizeWindow()} aria-label="最小化" disabled={closing}>
          <WinMin />
        </button>
        <button className="titlebar__btn" onClick={() => void toggleMaximizeWindow()} aria-label={maximized ? "还原" : "最大化"} disabled={closing}>
          {maximized ? <WinRestore /> : <WinMax />}
        </button>
        <button className="titlebar__btn titlebar__btn--close" onClick={() => void closeWindow()} aria-label={closing ? "正在保存并关闭" : "关闭"} disabled={closing}>
          {closing ? <Spinner size={12} /> : <WinCloseIcon />}
        </button>
      </div>
    </header>
  );
}
