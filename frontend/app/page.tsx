"use client";

import { type CSSProperties, useState } from "react";

import CheckpointPanel from "@/components/CheckpointPanel";
import ConnectionBanner from "@/components/ConnectionBanner";
import FileWorkbench from "@/components/FileWorkbench";
import MemoryPanel from "@/components/MemoryPanel";
import Providers from "@/components/Providers";
import RpcConsole from "@/components/RpcConsole";
import StatusBar from "@/components/StatusBar";
import TitleBar from "@/components/TitleBar";
import ToolCatalog from "@/components/ToolCatalog";
import ViewTransition from "@/components/ViewTransition";

type TabId = "tools" | "files" | "memory" | "checkpoints" | "rpc";

const TABS: { id: TabId; label: string; emoji: string; desc: string }[] = [
  {
    id: "files",
    label: "文件工作台",
    emoji: "📝",
    desc: "浏览、打开、编辑并保存工作区里的稿件文件——所有路径都被关在沙箱里。",
  },
  {
    id: "memory",
    label: "记忆库",
    emoji: "🧠",
    desc: "长期记忆（人物 / 设定 / 伏笔…），中文友好的 BM25 检索，只返回结构化摘要。",
  },
  {
    id: "checkpoints",
    label: "检查点",
    emoji: "💾",
    desc: "给整份稿件拍字节级快照，写崩了一键回滚——只还原稿件，记忆与日志不受影响。",
  },
  {
    id: "tools",
    label: "工具目录",
    emoji: "🧰",
    desc: "核心层暴露给 AI 的全部工具：读写稿件、检索记忆、管理版本、派生子代理等。",
  },
  {
    id: "rpc",
    label: "RPC 控制台",
    emoji: "🔌",
    desc: "直接给 Rust 核心发任意 JSON-RPC 请求（高级用法）。",
  },
];

export default function Home() {
  const [tab, setTab] = useState<TabId>("files");
  const active = TABS.find((t) => t.id === tab) ?? TABS[0];

  return (
    <Providers>
      <div className="shell">
        <TitleBar />
        <div className="app">
          <aside className="sidebar">
            <div className="brand">
              Novel Generate Team
              <span className="sub">团队协作同人小说 · 创作工作台</span>
            </div>
            <nav className="nav">
              {TABS.map((t, i) => (
                <button
                  key={t.id}
                  className={`navbtn${tab === t.id ? " active" : ""}`}
                  style={{ "--i": i } as CSSProperties}
                  onClick={() => setTab(t.id)}
                  aria-current={tab === t.id ? "page" : undefined}
                >
                  <span className="emoji" aria-hidden>
                    {t.emoji}
                  </span>
                  {t.label}
                </button>
              ))}
            </nav>
            <StatusBar />
          </aside>

          <main className="main">
            <header className="main-header" key={tab}>
              <h1 className="main-title">
                <span className="main-title-emoji" aria-hidden>
                  {active.emoji}
                </span>
                {active.label}
              </h1>
              <p className="main-desc">{active.desc}</p>
            </header>

            <ConnectionBanner />

            <div className="main-body">
              <ViewTransition viewKey={tab}>
                {tab === "files" && <FileWorkbench />}
                {tab === "memory" && <MemoryPanel />}
                {tab === "checkpoints" && <CheckpointPanel />}
                {tab === "tools" && <ToolCatalog />}
                {tab === "rpc" && <RpcConsole />}
              </ViewTransition>
            </div>
          </main>
        </div>
      </div>
    </Providers>
  );
}
