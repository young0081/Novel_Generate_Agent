// WorkTabs: horizontal tab navigation for the work modes

import {
  IconCompass,
  IconBrush,
  IconScroll,
  IconChat,
  IconProviders,
  IconRestore,
  IconUser,
  IconArchive,
  IconSeed,
  IconSimulate,
  IconPencil,
} from "../icons";

type WorkMode =
  | "library"
  | "planning"
  | "discuss"
  | "simulate"
  | "studio"
  | "revision"
  | "knowledge"
  | "ide";

interface WorkTabsProps {
  active: WorkMode;
  onSelect: (mode: WorkMode) => void;
  onSettings: () => void;
  onSessions: () => void;
  onMemory: () => void;
}

const TABS: Array<{ id: WorkMode; label: string; Icon: typeof IconCompass }> = [
  { id: "library", label: "书库", Icon: IconArchive },
  { id: "planning", label: "策划", Icon: IconCompass },
  { id: "discuss", label: "探讨", Icon: IconChat },
  { id: "simulate", label: "推演", Icon: IconSimulate },
  { id: "studio", label: "创作", Icon: IconBrush },
  { id: "ide", label: "编辑", Icon: IconPencil },
  { id: "revision", label: "修订", Icon: IconScroll },
  { id: "knowledge", label: "知识库", Icon: IconSeed },
];

export default function WorkTabs({ active, onSelect, onSettings, onSessions, onMemory }: WorkTabsProps) {
  return (
    <div className="work-tabs">
      {TABS.map((tab) => {
        const Icon = tab.Icon;
        return (
          <button
            key={tab.id}
            className={`work-tab${active === tab.id ? " is-active" : ""}`}
            onClick={() => onSelect(tab.id)}
          >
            <span className="work-tab__icon">
              <Icon size={18} />
            </span>
            {tab.label}
          </button>
        );
      })}
      <button
        className="work-tabs__aux"
        onClick={onSessions}
        aria-label="会话历史"
        title="会话历史"
      >
        <IconRestore size={18} />
      </button>
      <button
        className="work-tabs__aux"
        onClick={onMemory}
        aria-label="记忆库"
        title="记忆库"
      >
        <IconUser size={18} />
      </button>
      <button
        className="work-tabs__settings"
        onClick={onSettings}
        aria-label="设置"
        title="设置"
      >
        <IconProviders size={18} />
      </button>
    </div>
  );
}
