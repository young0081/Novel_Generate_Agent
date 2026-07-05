"use client";

import { useEffect, useState } from "react";

import { getDesktopAPI, type DesktopAPI } from "@/lib/desktop";

export default function TitleBar() {
  const [api, setApi] = useState<DesktopAPI | null>(null);

  useEffect(() => {
    setApi(getDesktopAPI());
  }, []);

  // Plain browser: no frameless title bar.
  if (!api) return null;

  return (
    <div className="titlebar">
      <div className="titlebar-drag">
        <span className="titlebar-title">Novel Generate Team —— 创作工作台</span>
      </div>
      <div className="titlebar-controls">
        <button className="winbtn" onClick={() => api.minimize()} aria-label="最小化" title="最小化">
          &#x2014;
        </button>
        <button
          className="winbtn"
          onClick={() => api.maximizeToggle()}
          aria-label="最大化 / 还原"
          title="最大化 / 还原"
        >
          &#x25A1;
        </button>
        <button
          className="winbtn close"
          onClick={() => api.close()}
          aria-label="关闭"
          title="关闭"
        >
          &#x2715;
        </button>
      </div>
    </div>
  );
}
