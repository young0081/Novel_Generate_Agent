// Navigation — Vertical book-spine sidebar
// 64px narrow sidebar with icon + vertical text label per mode

import {
  IconArchive,
  IconCompass,
  IconChat,
  IconSimulate,
  IconBrush,
  IconPencil,
  IconScroll,
  IconSeed,
  IconRestore,
  IconUser,
  IconSettings,
} from "./icons";

type WorkMode =
  | "library"
  | "planning"
  | "discuss"
  | "simulate"
  | "studio"
  | "ide"
  | "revision"
  | "knowledge";

interface NavigationProps {
  active: WorkMode;
  onSelect: (mode: WorkMode) => void;
  onSettings: () => void;
  onSessions: () => void;
  onMemory: () => void;
}

const NAV_ITEMS: Array<{
  id: WorkMode;
  label: string;
  Icon: typeof IconArchive;
}> = [
  { id: "library", label: "书库", Icon: IconArchive },
  { id: "planning", label: "策划", Icon: IconCompass },
  { id: "discuss", label: "探讨", Icon: IconChat },
  { id: "simulate", label: "推演", Icon: IconSimulate },
  { id: "studio", label: "创作", Icon: IconBrush },
  { id: "ide", label: "编辑", Icon: IconPencil },
  { id: "revision", label: "修订", Icon: IconScroll },
  { id: "knowledge", label: "知识库", Icon: IconSeed },
];

export default function Navigation({
  active,
  onSelect,
  onSettings,
  onSessions,
  onMemory,
}: NavigationProps) {
  return (
    <nav className="nav-spine">
      {/* 可滚动的导航项区域 — 小窗口时在此区域内滚动，底部按钮始终可见 */}
      <div className="nav-spine__items">
        {NAV_ITEMS.map((item) => {
          const Icon = item.Icon;
          return (
            <button
              key={item.id}
              className={`nav-item${active === item.id ? " is-active" : ""}`}
              onClick={() => onSelect(item.id)}
              title={item.label}
            >
              <span className="nav-item__icon">
                <Icon size={18} />
              </span>
              <span className="nav-item__label">{item.label}</span>
            </button>
          );
        })}
      </div>

      {/* 底部辅助按钮 — flex-shrink:0，永远贴底 */}
      <div className="nav-spine__bottom">
        <button className="nav-aux" onClick={onSessions} title="会话历史">
          <IconRestore size={16} />
          <span className="nav-aux__label">会话</span>
        </button>
        <button className="nav-aux" onClick={onMemory} title="记忆库">
          <IconUser size={16} />
          <span className="nav-aux__label">记忆</span>
        </button>
        <button className="nav-aux nav-aux--settings" onClick={onSettings} title="设置">
          <IconSettings size={16} />
          <span className="nav-aux__label">设置</span>
        </button>
      </div>
    </nav>
  );
}
