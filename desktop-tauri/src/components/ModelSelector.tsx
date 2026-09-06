// ModelSelector — 快速模型/供应商切换下拉组件
// 设计：紧凑内联按钮，点击展开浮层列表；支持 size="sm"/"md" 两种尺寸。
// 外部通过 onChange 回调感知切换，也可直接用 onSettingsOpen 跳转到供应商设置。

import { useCallback, useEffect, useId, useRef, useState } from "react";
import {
  getProviders,
  PROVIDERS_CHANGED_EVENT,
  setActiveProvider,
  type ProviderConfig,
  type ProviderSettings,
} from "../lib/providers";
import { IconProviders, IconChevron } from "./icons";
import { Spinner } from "./Spinner";
import { describeError } from "../lib/core";

interface ModelSelectorProps {
  /** 切换成功后的回调，传出 providerId + model */
  onChange?: (providerId: string, model: string) => void;
  /** 点击"去设置"时的回调 */
  onSettingsOpen?: () => void;
  /** 展示尺寸 */
  size?: "sm" | "md";
  /** 额外 className */
  className?: string;
}

interface ActiveInfo {
  providerName: string;
  model: string;
  providerId: string;
}

export default function ModelSelector({
  onChange,
  onSettingsOpen,
  size = "md",
  className = "",
}: ModelSelectorProps) {
  const [settings, setSettings] = useState<ProviderSettings | null>(null);
  const [active, setActive] = useState<ActiveInfo | null>(null);
  const [open, setOpen] = useState(false);
  const [switching, setSwitching] = useState(false);
  const [switchError, setSwitchError] = useState<string | null>(null);
  const [focusedOption, setFocusedOption] = useState(0);
  const containerRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const optionRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const listboxId = useId();
  const errorId = useId();

  // 加载供应商列表
  const load = useCallback(async () => {
    try {
      const s = await getProviders();
      setSettings(s);

      // 解析当前激活的供应商+模型
      if (s.active_provider && s.active_model) {
        const prov = s.providers.find((p) => p.id === s.active_provider);
        if (prov) {
          setActive({
            providerName: prov.name,
            model: s.active_model,
            providerId: prov.id,
          });
          return;
        }
      }
      // Do not present an unpersisted fallback as active. The user can choose
      // one below, which calls providers_set_active and makes the state real.
      setActive(null);
    } catch {
      setActive(null);
    }
  }, []);

  useEffect(() => {
    void load();
    const handleProvidersChanged = () => { void load(); };
    window.addEventListener(PROVIDERS_CHANGED_EVENT, handleProvidersChanged);
    return () => window.removeEventListener(PROVIDERS_CHANGED_EVENT, handleProvidersChanged);
  }, [load]);

  // 点击外部或将焦点移出组件时关闭浮层。
  useEffect(() => {
    if (!open) return;
    const handlePointer = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    const handleFocus = (e: FocusEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", handlePointer);
    document.addEventListener("focusin", handleFocus);
    return () => {
      document.removeEventListener("mousedown", handlePointer);
      document.removeEventListener("focusin", handleFocus);
    };
  }, [open]);

  // 切换到目标供应商+模型
  const switchTo = useCallback(
    async (provider: ProviderConfig, model: string, optionIndex: number) => {
      if (switching) return;
      setSwitching(true);
      setSwitchError(null);
      try {
        await setActiveProvider(provider.id, model);
        setActive({ providerName: provider.name, model, providerId: provider.id });
        onChange?.(provider.id, model);
        setOpen(false);
        window.requestAnimationFrame(() => triggerRef.current?.focus());
      } catch (error) {
        setSwitchError(describeError(error));
        setOpen(true);
        window.requestAnimationFrame(() => {
          optionRefs.current[optionIndex]?.focus();
        });
      } finally {
        setSwitching(false);
      }
    },
    [switching, onChange],
  );

  const hasProviders = settings && settings.providers.length > 0;
  const choices = settings?.providers.flatMap((provider) =>
    provider.models.map((model) => ({ provider, model })),
  ) ?? [];

  const focusChoice = useCallback((index: number) => {
    if (choices.length === 0) return;
    const next = (index + choices.length) % choices.length;
    setFocusedOption(next);
    window.requestAnimationFrame(() => optionRefs.current[next]?.focus());
  }, [choices.length]);

  const openChoices = useCallback((edge?: "first" | "last") => {
    if (choices.length === 0) {
      setFocusedOption(0);
      setOpen(true);
      return;
    }
    const activeIndex = choices.findIndex(({ provider, model }) =>
      active?.providerId === provider.id && active.model === model,
    );
    const next = edge === "first"
      ? 0
      : edge === "last"
        ? choices.length - 1
        : activeIndex >= 0
          ? activeIndex
          : 0;
    setFocusedOption(next);
    setOpen(true);
    window.requestAnimationFrame(() => optionRefs.current[next]?.focus());
  }, [active, choices]);

  const closeChoices = useCallback((restoreFocus: boolean) => {
    setOpen(false);
    if (restoreFocus) {
      window.requestAnimationFrame(() => triggerRef.current?.focus());
    }
  }, []);

  // 无供应商 → 引导按钮
  if (settings && !hasProviders) {
    return (
      <button
        className={`model-sel model-sel--empty model-sel--${size} ${className}`}
        onClick={onSettingsOpen}
        title="前往配置 AI 供应商"
      >
        <IconProviders size={size === "sm" ? 12 : 14} />
        <span>配置模型</span>
      </button>
    );
  }

  const displayLabel = active
    ? `${active.providerName} · ${active.model}`
    : "选择模型";

  return (
    <div
      ref={containerRef}
      className={`model-sel model-sel--${size} ${className} ${open ? "model-sel--open" : ""}`}
    >
      {/* 触发按钮 */}
      <button
        ref={triggerRef}
        className="model-sel__trigger"
        onClick={() => {
          if (switching) return;
          if (open) closeChoices(false);
          else openChoices();
        }}
        onKeyDown={(event) => {
          if (switching) {
            if (event.key === "Escape" && open) closeChoices(true);
            if (["Escape", "ArrowDown", "ArrowUp"].includes(event.key)) {
              event.preventDefault();
            }
            return;
          }
          if (event.key === "ArrowDown") {
            event.preventDefault();
            openChoices("first");
          } else if (event.key === "ArrowUp") {
            event.preventDefault();
            openChoices("last");
          } else if (event.key === "Escape" && open) {
            event.preventDefault();
            closeChoices(true);
          }
        }}
        title={`当前模型：${displayLabel}`}
        type="button"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? listboxId : undefined}
        aria-busy={switching || undefined}
        aria-disabled={switching || undefined}
        aria-describedby={switchError && open ? errorId : undefined}
      >
        {switching ? (
          <Spinner size={size === "sm" ? 11 : 13} />
        ) : (
          <IconProviders size={size === "sm" ? 11 : 13} />
        )}
        <span className="model-sel__label">{displayLabel}</span>
        <IconChevron
          size={size === "sm" ? 10 : 11}
          className={`model-sel__caret${open ? " model-sel__caret--up" : ""}`}
        />
      </button>

      {/* 浮层下拉列表 */}
      {open && settings && (
        <div
          className={`model-sel__dropdown${switching ? " is-switching" : ""}`}
          aria-busy={switching || undefined}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              event.stopPropagation();
              closeChoices(true);
              return;
            }
            if (event.key === "ArrowDown") {
              event.preventDefault();
              focusChoice(focusedOption + 1);
            } else if (event.key === "ArrowUp") {
              event.preventDefault();
              focusChoice(focusedOption - 1);
            } else if (event.key === "Home") {
              event.preventDefault();
              focusChoice(0);
            } else if (event.key === "End") {
              event.preventDefault();
              focusChoice(choices.length - 1);
            }
          }}
        >
          <div id={listboxId} role="listbox" aria-label="可用模型">
            {settings.providers.map((prov) => {
              if (prov.models.length === 0) return null;
              return (
                <div key={prov.id} className="model-sel__group">
                  <div className="model-sel__group-label">{prov.name}</div>
                  {prov.models.map((m) => {
                    const optionIndex = choices.findIndex(
                      ({ provider, model }) => provider.id === prov.id && model === m,
                    );
                    const isActive =
                      active?.providerId === prov.id && active?.model === m;
                    return (
                      <button
                        key={m}
                        ref={(node) => { optionRefs.current[optionIndex] = node; }}
                        className={`model-sel__option${isActive ? " model-sel__option--active" : ""}`}
                        onClick={() => { void switchTo(prov, m, optionIndex); }}
                        onFocus={() => setFocusedOption(optionIndex)}
                        role="option"
                        aria-selected={isActive}
                        aria-disabled={switching || undefined}
                        tabIndex={focusedOption === optionIndex ? 0 : -1}
                        type="button"
                      >
                        <span className="model-sel__option-dot" />
                        <span className="model-sel__option-name">{m}</span>
                        {isActive && (
                          <span className="model-sel__active-mark" aria-hidden>✓</span>
                        )}
                      </button>
                    );
                  })}
                </div>
              );
            })}
          </div>
          {switchError && (
            <div id={errorId} className="model-sel__error" role="alert">
              切换失败：{switchError}
            </div>
          )}
          {/* 底部跳设置链接 */}
          {onSettingsOpen && (
            <button
              className="model-sel__settings-link"
              onClick={() => {
                if (switching) return;
                setOpen(false);
                onSettingsOpen();
              }}
              onKeyDown={(event) => {
                if (event.key === "ArrowUp" && choices.length > 0) {
                  event.preventDefault();
                  focusChoice(choices.length - 1);
                }
              }}
              type="button"
              aria-disabled={switching || undefined}
            >
              <IconProviders size={11} />
              管理供应商…
            </button>
          )}
        </div>
      )}
    </div>
  );
}
