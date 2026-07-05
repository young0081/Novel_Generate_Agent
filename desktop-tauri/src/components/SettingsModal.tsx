// Settings overlay: provider configuration + tool catalog, in a full-bleed
// sheet that slides up over the workspace. Reuses the existing Providers and
// Tools screens verbatim (they bring their own Panel chrome).

import { useState } from "react";
import { WinCloseIcon, IconProviders, IconTools } from "./icons";
import ProvidersScreen from "../screens/ProvidersScreen";
import ToolsScreen from "../screens/ToolsScreen";

type SettingsTab = "providers" | "tools";

interface SettingsModalProps {
  onClose: () => void;
}

export default function SettingsModal({ onClose }: SettingsModalProps) {
  const [tab, setTab] = useState<SettingsTab>("providers");

  return (
    <div className="settings-overlay" onClick={onClose}>
      <div
        className="settings-sheet"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-label="设置"
      >
        <header className="settings-sheet__head">
          <div className="settings-sheet__tabs">
            <button
              className={`settings-sheet__tab${tab === "providers" ? " is-active" : ""}`}
              onClick={() => setTab("providers")}
            >
              <IconProviders size={16} />
              供应商
            </button>
            <button
              className={`settings-sheet__tab${tab === "tools" ? " is-active" : ""}`}
              onClick={() => setTab("tools")}
            >
              <IconTools size={16} />
              工具
            </button>
          </div>
          <button className="settings-sheet__close" onClick={onClose} aria-label="关闭设置">
            <WinCloseIcon />
          </button>
        </header>
        <div className="settings-sheet__body">
          {tab === "providers" ? <ProvidersScreen /> : <ToolsScreen />}
        </div>
      </div>
    </div>
  );
}
