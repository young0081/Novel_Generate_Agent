// Navigation — a calm, scan-friendly workspace rail.

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
  IconBranch,
  IconClock,
  IconStar,
  IconTools,
} from "./icons";

type WorkMode =
  | "library"
  | "planning"
  | "harness"
  | "discuss"
  | "simulate"
  | "studio"
  | "ide"
  | "revision"
  | "knowledge"
  | "style"
  | "collab"
  | "checkpoints";

interface NavigationProps {
  active: WorkMode;
  pending?: WorkMode | null;
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
  { id: "harness", label: "Harness", Icon: IconTools },
  { id: "discuss", label: "探讨", Icon: IconChat },
  { id: "simulate", label: "推演", Icon: IconSimulate },
  { id: "studio", label: "创作", Icon: IconBrush },
  { id: "ide", label: "编辑", Icon: IconPencil },
  { id: "revision", label: "修订", Icon: IconScroll },
  { id: "knowledge", label: "知识库", Icon: IconSeed },
  { id: "style", label: "文风", Icon: IconStar },
  { id: "collab", label: "协作", Icon: IconBranch },
  { id: "checkpoints", label: "快照", Icon: IconClock },
];

const NAV_GROUPS = [
  { label: "创作工作区", items: NAV_ITEMS.slice(0, 10) },
  { label: "项目与版本", items: NAV_ITEMS.slice(10) },
] as const;

export default function Navigation({
  active,
  pending = null,
  onSelect,
  onSettings,
  onSessions,
  onMemory,
}: NavigationProps) {
  return (
    <nav className="nav-spine" aria-label="工作区导航">
      <div className="nav-spine__intro">
        <div className="nav-spine__eyebrow">WORKSPACE</div>
        <div className="nav-spine__intro-title">故事工作区</div>
        <p className="nav-spine__intro-sub">从设定到成稿，一处完成</p>
      </div>
      <div className="nav-spine__items">
        {NAV_GROUPS.map((group, groupIndex) => (
          <div
            className="nav-group"
            role="group"
            aria-label={group.label}
            key={group.label}
          >
            {groupIndex > 0 ? <div className="nav-spine__divider" aria-hidden="true" /> : null}
            <div className="nav-group__label">{group.label}</div>
            {group.items.map((item) => {
              const Icon = item.Icon;
              return (
                <button
                  type="button"
                  key={item.id}
                  className={`nav-item${active === item.id ? " is-active" : ""}${pending === item.id ? " is-pending" : ""}`}
                  onClick={() => onSelect(item.id)}
                  title={item.label}
                  aria-current={active === item.id ? "page" : undefined}
                  aria-busy={pending === item.id || undefined}
                  aria-label={pending === item.id ? `${item.label}，正在打开` : item.label}
                >
                  <span className="nav-item__icon" aria-hidden="true">
                    <Icon size={18} />
                  </span>
                  <span className="nav-item__label">{item.label}</span>
                </button>
              );
            })}
          </div>
        ))}
      </div>

      <div className="nav-spine__bottom">
        <div className="nav-spine__bottom-label">辅助工具</div>
        <button
          type="button"
          className="nav-aux"
          onClick={onSessions}
          title="会话历史"
        >
          <IconRestore size={16} aria-hidden="true" />
          <span className="nav-aux__label">会话</span>
        </button>
        <button type="button" className="nav-aux" onClick={onMemory} title="记忆库">
          <IconUser size={16} aria-hidden="true" />
          <span className="nav-aux__label">记忆</span>
        </button>
        <button
          type="button"
          className="nav-aux nav-aux--settings"
          onClick={onSettings}
          title="设置"
        >
          <IconSettings size={16} aria-hidden="true" />
          <span className="nav-aux__label">设置</span>
        </button>
      </div>
    </nav>
  );
}
