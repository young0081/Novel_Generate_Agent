// ModelSelector — 快速模型/供应商切换下拉组件
// 设计：紧凑内联按钮，点击展开浮层列表；支持 size="sm"/"md" 两种尺寸。
// 外部通过 onChange 回调感知切换，也可直接用 onSettingsOpen 跳转到供应商设置。

import { useCallback, useEffect, useRef, useState } from "react";
import {
  getProviders,
  setActiveProvider,
  type ProviderConfig,
  type ProviderSettings,
} from "../lib/providers";
import { IconProviders, IconChevron } from "./icons";
import { Spinner } from "./Spinner";

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
  const containerRef = useRef<HTMLDivElement>(null);

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
      // 回退：取第一个有模型的供应商的第一个模型
      for (const prov of s.providers) {
        if (prov.models.length > 0) {
          setActive({
            providerName: prov.name,
            model: prov.default_model ?? prov.models[0],
            providerId: prov.id,
          });
          return;
        }
      }
      setActive(null);
    } catch {
      setActive(null);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  // 点击外部关闭浮层
  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  // 切换到目标供应商+模型
  const switchTo = useCallback(
    async (provider: ProviderConfig, model: string) => {
      if (switching) return;
      setSwitching(true);
      setOpen(false);
      try {
        await setActiveProvider(provider.id, model);
        setActive({ providerName: provider.name, model, providerId: provider.id });
        onChange?.(provider.id, model);
      } catch {
        // 忽略：UI 静默降级
      } finally {
        setSwitching(false);
      }
    },
    [switching, onChange],
  );

  const hasProviders = settings && settings.providers.length > 0;

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
        className="model-sel__trigger"
        onClick={() => setOpen((v) => !v)}
        title={`当前模型：${displayLabel}`}
        disabled={switching}
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
        <div className="model-sel__dropdown" role="listbox">
          {settings.providers.map((prov) => {
            if (prov.models.length === 0) return null;
            return (
              <div key={prov.id} className="model-sel__group">
                <div className="model-sel__group-label">{prov.name}</div>
                {prov.models.map((m) => {
                  const isActive =
                    active?.providerId === prov.id && active?.model === m;
                  return (
                    <button
                      key={m}
                      className={`model-sel__option${isActive ? " model-sel__option--active" : ""}`}
                      onClick={() => switchTo(prov, m)}
                      role="option"
                      aria-selected={isActive}
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
          {/* 底部跳设置链接 */}
          {onSettingsOpen && (
            <button
              className="model-sel__settings-link"
              onClick={() => { setOpen(false); onSettingsOpen(); }}
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
