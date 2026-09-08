import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { isDesktop, NotInDesktopError } from "./core";

export interface StyleProfile {
  id: string;
  name: string;
  description: string;
  tone: string;
  narrative_person: string;
  pacing: string;
  sentence_patterns: string[];
  dialogue_rules: string[];
  imagery_rules: string[];
  banned_patterns: string[];
  humanizer_rules: string[];
  sample_excerpts: string[];
  source_article_count: number;
  created_ms: number;
  updated_ms: number;
}

export interface StylePayload {
  profiles: StyleProfile[];
  active_id: string | null;
  active: StyleProfile | null;
  saved?: StyleProfile;
}

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isDesktop()) throw new NotInDesktopError();
  return (await tauriInvoke(cmd, args)) as T;
}

export function emptyStyleProfile(): StyleProfile {
  return {
    id: "",
    name: "",
    description: "",
    tone: "",
    narrative_person: "",
    pacing: "",
    sentence_patterns: [],
    dialogue_rules: [],
    imagery_rules: [],
    banned_patterns: [],
    humanizer_rules: [],
    sample_excerpts: [],
    source_article_count: 0,
    created_ms: 0,
    updated_ms: 0,
  };
}

export async function getStyleProfiles(): Promise<StylePayload> {
  return invoke<StylePayload>("style_profiles_get");
}

export async function analyzeStyle(article: string): Promise<StyleProfile> {
  return invoke<StyleProfile>("style_profile_analyze", { article });
}

export async function saveStyleProfile(profile: StyleProfile): Promise<StylePayload> {
  return invoke<StylePayload>("style_profile_save", { profile });
}

export async function deleteStyleProfile(id: string): Promise<StylePayload> {
  return invoke<StylePayload>("style_profile_delete", { id });
}

export async function setActiveStyle(id: string | null): Promise<StylePayload> {
  return invoke<StylePayload>("style_profile_set_active", { id });
}

export function splitRules(text: string): string[] {
  return text
    .split(/\r?\n|[,，]/)
    .map((item) => item.trim())
    .filter(Boolean);
}

export function joinRules(items: string[]): string {
  return items.join("\n");
}
