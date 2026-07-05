// Custom title bar for the frameless window (v3 layout).
// Left section (64px) aligns with the nav-spine; right section is draggable.

import { useEffect, useState } from "react";
import Seal from "./Seal";
import { WinMin, WinMax, WinRestore, WinCloseIcon } from "./icons";
import { useWork } from "./WorkContext";
import {
  minimizeWindow,
  toggleMaximizeWindow,
  closeWindow,
  isWindowMaximized,
  onWindowResized,
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
  const { current } = useWork();

  useEffect(() => {
    let alive = true;
    void isWindowMaximized().then((v) => { if (alive) setMaximized(v); });
    const unlisten = onWindowResized(async () => {
      const v = await isWindowMaximized();
      if (alive) setMaximized(v);
    });
    return () => {
      alive = false;
      void unlisten.then((fn) => fn());
    };
  }, []);

  return (
    <header className="titlebar" data-tauri-drag-region>
      {/* Brand — aligns with 64px nav sidebar */}
      <div className="titlebar__brand">
        <Seal size={22} char="墨" />
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
        <span className={healthDotClass(health)} />
        <span className="titlebar__status-text">{healthLabel(health)}</span>
      </div>

      {/* Window controls */}
      <div className="titlebar__controls">
        <button className="titlebar__btn" onClick={() => void minimizeWindow()} aria-label="最小化">
          <WinMin />
        </button>
        <button className="titlebar__btn" onClick={() => void toggleMaximizeWindow()} aria-label={maximized ? "还原" : "最大化"}>
          {maximized ? <WinRestore /> : <WinMax />}
        </button>
        <button className="titlebar__btn titlebar__btn--close" onClick={() => void closeWindow()} aria-label="关闭">
          <WinCloseIcon />
        </button>
      </div>
    </header>
  );
}
