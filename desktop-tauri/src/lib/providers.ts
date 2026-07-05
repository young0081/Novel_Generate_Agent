// Typed client for the model-provider configuration commands.
//
// Like `core.ts`, every call is guarded so that running outside the desktop
// shell (a plain browser `vite preview`) yields a friendly Chinese message
// instead of an opaque "invoke is not defined" crash. Tauri v2 maps the
// camelCase keys in the args object to snake_case Rust params, so the wrapper
// passes camelCase here while the wire types themselves stay serde snake_case.

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { isDesktop, NotInDesktopError } from "./core";

/** Wire protocol identifier. Note: OpenAI-compatible is "open_ai" (not "openai"). */
export type ProviderProtocol = "open_ai" | "anthropic" | "gemini";

/** How agent tasks ask the model to use tools. */
export type ProviderToolMode = "auto" | "native" | "text";

/** Sampling parameters for model generation. */
export interface SamplingParams {
  /** Temperature (0.0 = deterministic, 2.0 = very random). Default: 1.0. */
  temperature: number;
  /** Top-p (nucleus sampling, 0.0-1.0). Default: 1.0. */
  top_p: number;
  /** Top-k (0 = disabled). Default: 0. */
  top_k: number;
  /** Presence penalty (-2.0 to 2.0). Default: 0.0. */
  presence_penalty: number;
  /** Frequency penalty (-2.0 to 2.0). Default: 0.0. */
  frequency_penalty: number;
}

/** A single configured provider (one endpoint + credentials + its models). */
export interface ProviderConfig {
  /** Stable id; generated for new providers. */
  id: string;
  /** Display name. */
  name: string;
  protocol: ProviderProtocol;
  /** Agent tool-call compatibility mode. Missing values are treated as "auto". */
  tool_mode?: ProviderToolMode;
  /**
   * Endpoint base URL.
   * OpenAI-compatible: e.g. https://api.openai.com/v1 (DeepSeek/Kimi/… alike).
   * Anthropic: https://api.anthropic.com
   * Gemini: https://generativelanguage.googleapis.com
   */
  base_url: string;
  api_key: string;
  /** User-managed list of model names for this provider. */
  models: string[];
  default_model?: string | null;
  /** Optional cap; Anthropic falls back to 4096 server-side when null. */
  max_tokens?: number | null;
  /** Default sampling parameters for this provider. */
  sampling?: SamplingParams;
}

/** The whole providers document persisted by the core. */
export interface ProviderSettings {
  providers: ProviderConfig[];
  /** Active provider id. */
  active_provider?: string | null;
  /** Active model name (must belong to the active provider). */
  active_model?: string | null;
}

async function invoke<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (!isDesktop()) {
    throw new NotInDesktopError();
  }
  return (await tauriInvoke(cmd, args)) as T;
}

/** Load the persisted provider settings. */
export async function getProviders(): Promise<ProviderSettings> {
  return invoke<ProviderSettings>("providers_get");
}

/** Add or update a provider (matched by id). Returns the new settings. */
export async function saveProvider(
  config: ProviderConfig,
): Promise<ProviderSettings> {
  return invoke<ProviderSettings>("providers_save", { config });
}

/** Delete a provider by id. Returns the new settings. */
export async function deleteProvider(id: string): Promise<ProviderSettings> {
  return invoke<ProviderSettings>("providers_delete", { id });
}

/** Set the active provider + model. Returns the new settings. */
export async function setActiveProvider(
  providerId: string,
  model: string,
): Promise<ProviderSettings> {
  return invoke<ProviderSettings>("providers_set_active", {
    providerId,
    model,
  });
}

/**
 * Send a tiny real request to verify a provider/model works. This performs a
 * NETWORK call: callers must show a spinner. Throws a string on failure;
 * resolves with the model's reply text on success.
 */
export async function testProvider(
  config: ProviderConfig,
  model: string,
): Promise<string> {
  return invoke<string>("provider_test", { config, model });
}

/** Human-readable label for a protocol. */
export const PROTOCOL_LABEL: Record<ProviderProtocol, string> = {
  open_ai: "OpenAI 兼容",
  anthropic: "Anthropic",
  gemini: "Gemini",
};

export const TOOL_MODE_LABEL: Record<ProviderToolMode, string> = {
  auto: "自动兼容",
  native: "原生工具",
  text: "文本工具",
};

export const TOOL_MODE_HINT: Record<ProviderToolMode, string> = {
  auto: "OpenAI 兼容接口默认走文本工具模式，Anthropic/Gemini 默认走原生工具调用。",
  native: "把工具 schema 直接发给模型。适合确认支持 function/tool calling 的模型。",
  text: "不发送 tools 字段，改用 ReAct 文本格式。适合 DeepSeek、Ollama、本地兼容接口。",
};

/** A sensible base_url placeholder per protocol. */
export const PROTOCOL_PLACEHOLDER: Record<ProviderProtocol, string> = {
  open_ai: "https://api.openai.com/v1",
  anthropic: "https://api.anthropic.com",
  gemini: "https://generativelanguage.googleapis.com",
};

/**
 * A one-click quick-fill template for a well-known provider. It only seeds the
 * endpoint (protocol + base_url) and a couple of suggested model names — the
 * user still supplies their own API key. These never touch the backend
 * contract; they're purely a UI convenience.
 */
export interface ProviderPreset {
  /** Display name, also used as the suggested provider name. */
  name: string;
  protocol: ProviderProtocol;
  tool_mode?: ProviderToolMode;
  base_url: string;
  /** Optional starter models to seed the list (user can edit freely). */
  models?: string[];
}

/** Common providers, mirroring the empty-state copy. */
export const PROVIDER_PRESETS: ProviderPreset[] = [
  {
    name: "OpenAI",
    protocol: "open_ai",
    tool_mode: "native",
    base_url: "https://api.openai.com/v1",
    models: ["gpt-4o-mini", "gpt-4o"],
  },
  {
    name: "DeepSeek",
    protocol: "open_ai",
    tool_mode: "text",
    base_url: "https://api.deepseek.com/v1",
    models: ["deepseek-chat", "deepseek-reasoner"],
  },
  {
    name: "Kimi",
    protocol: "open_ai",
    tool_mode: "text",
    base_url: "https://api.moonshot.cn/v1",
    models: ["moonshot-v1-8k", "moonshot-v1-32k"],
  },
  {
    name: "智谱 GLM",
    protocol: "open_ai",
    tool_mode: "text",
    base_url: "https://open.bigmodel.cn/api/paas/v4",
    models: ["glm-4-plus", "glm-4-flash"],
  },
  {
    name: "OpenRouter",
    protocol: "open_ai",
    tool_mode: "text",
    base_url: "https://openrouter.ai/api/v1",
  },
  {
    name: "Ollama (本地)",
    protocol: "open_ai",
    tool_mode: "text",
    base_url: "http://localhost:11434/v1",
    models: ["llama3.1"],
  },
  {
    name: "Anthropic",
    protocol: "anthropic",
    tool_mode: "native",
    base_url: "https://api.anthropic.com",
    models: ["claude-3-5-sonnet-latest", "claude-3-5-haiku-latest"],
  },
  {
    name: "Gemini",
    protocol: "gemini",
    tool_mode: "native",
    base_url: "https://generativelanguage.googleapis.com",
    models: ["gemini-2.5-flash", "gemini-2.5-pro"],
  },
];

/** Generate a stable id for a new provider (UUID when available, else time-based). */
export function newProviderId(): string {
  const c = (globalThis as { crypto?: { randomUUID?: () => string } }).crypto;
  if (c && typeof c.randomUUID === "function") {
    return c.randomUUID();
  }
  return `prov-${Date.now().toString(36)}-${Math.random()
    .toString(36)
    .slice(2, 8)}`;
}

/**
 * Mask a base URL for display: keeps the scheme + host (and a hint of the path)
 * so users recognise the endpoint without exposing a full, copyable string.
 */
export function maskBaseUrl(url: string): string {
  const raw = url.trim();
  if (!raw) return "（未填写）";
  try {
    const u = new URL(raw);
    const tail = u.pathname && u.pathname !== "/" ? u.pathname : "";
    return `${u.protocol}//${u.host}${tail}`;
  } catch {
    // not a parseable URL — show as-is, it's the user's own input
    return raw;
  }
}

/** Mask an API key, revealing only a short head/tail. */
export function maskApiKey(key: string): string {
  const k = key.trim();
  if (!k) return "（未设置）";
  if (k.length <= 8) return "••••••";
  return `${k.slice(0, 4)}••••${k.slice(-4)}`;
}
