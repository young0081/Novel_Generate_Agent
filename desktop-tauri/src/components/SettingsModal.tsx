// Settings overlay: provider configuration + tool catalog, in a full-bleed
// sheet that slides up over the workspace. Reuses the existing Providers and
// Tools screens verbatim (they bring their own Panel chrome).

import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { WinCloseIcon, IconProviders, IconTools } from "./icons";
import ProvidersScreen from "../screens/ProvidersScreen";
import ToolsScreen from "../screens/ToolsScreen";
import { isTopDialogLayer, layerExitDelay } from "../lib/dialogLayer";

type SettingsTab = "providers" | "tools";

interface SettingsModalProps {
  onClose: () => void;
}

export default function SettingsModal({ onClose }: SettingsModalProps) {
  const [tab, setTab] = useState<SettingsTab>("providers");
  const [closing, setClosing] = useState(false);
  const closingRef = useRef(false);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const sheetRef = useRef<HTMLDivElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const providersTabRef = useRef<HTMLButtonElement>(null);
  const toolsTabRef = useRef<HTMLButtonElement>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  const requestClose = useCallback(() => {
    if (closingRef.current) return;
    closingRef.current = true;
    setClosing(true);
    closeTimer.current = window.setTimeout(() => onCloseRef.current(), layerExitDelay(220));
  }, []);

  useEffect(() => {
    const previouslyFocused = document.activeElement as HTMLElement | null;
    const frame = window.requestAnimationFrame(() => closeRef.current?.focus());
    const onKeyDown = (event: KeyboardEvent) => {
      const sheet = sheetRef.current;
      if (!isTopDialogLayer(sheet)) return;
      if (event.key === "Escape") {
        event.preventDefault();
        requestClose();
        return;
      }
      if (event.key !== "Tab") return;
      const focusable = Array.from(sheet.querySelectorAll<HTMLElement>(
        'button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
      ));
      if (focusable.length === 0) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && (document.activeElement === first || !sheet.contains(document.activeElement))) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.cancelAnimationFrame(frame);
      window.removeEventListener("keydown", onKeyDown);
      if (closeTimer.current) window.clearTimeout(closeTimer.current);
      if (previouslyFocused?.isConnected) previouslyFocused.focus();
    };
  }, [requestClose]);

  const handleTabKey = useCallback((event: ReactKeyboardEvent<HTMLButtonElement>) => {
    let next: SettingsTab | null = null;
    if (event.key === "ArrowLeft" || event.key === "ArrowUp" || event.key === "Home") {
      next = "providers";
    } else if (event.key === "ArrowRight" || event.key === "ArrowDown" || event.key === "End") {
      next = "tools";
    }
    if (!next) return;
    event.preventDefault();
    setTab(next);
    window.requestAnimationFrame(() => {
      (next === "providers" ? providersTabRef.current : toolsTabRef.current)?.focus();
    });
  }, []);

  return (
    <div className={`settings-overlay${closing ? " is-closing" : ""}`} onClick={requestClose}>
      <div
        ref={sheetRef}
        className={`settings-sheet${closing ? " is-closing" : ""}`}
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label="设置"
        tabIndex={-1}
      >
        <header className="settings-sheet__head">
          <div className="settings-sheet__tabs" role="tablist" aria-label="设置分类">
            <button
              ref={providersTabRef}
              className={`settings-sheet__tab${tab === "providers" ? " is-active" : ""}`}
              onClick={() => setTab("providers")}
              onKeyDown={handleTabKey}
              role="tab"
              aria-selected={tab === "providers"}
              aria-controls="settings-tabpanel"
              tabIndex={tab === "providers" ? 0 : -1}
            >
              <IconProviders size={16} />
              供应商
            </button>
            <button
              ref={toolsTabRef}
              className={`settings-sheet__tab${tab === "tools" ? " is-active" : ""}`}
              onClick={() => setTab("tools")}
              onKeyDown={handleTabKey}
              role="tab"
              aria-selected={tab === "tools"}
              aria-controls="settings-tabpanel"
              tabIndex={tab === "tools" ? 0 : -1}
            >
              <IconTools size={16} />
              工具
            </button>
          </div>
          <button ref={closeRef} className="settings-sheet__close" onClick={requestClose} aria-label="关闭设置">
            <WinCloseIcon />
          </button>
        </header>
        <div
          id="settings-tabpanel"
          className="settings-sheet__body"
          role="tabpanel"
          aria-label={tab === "providers" ? "供应商设置" : "工具设置"}
        >
          {tab === "providers" ? <ProvidersScreen /> : <ToolsScreen />}
        </div>
      </div>
    </div>
  );
}
