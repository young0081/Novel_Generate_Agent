// Typed client for the 知识库 (per-work knowledge base) commands.
//
// A work can own several knowledge bases — RAG corpora that keep creation
// faithful to canon. Each base holds entries indexed by CJK-aware BM25 on the
// backend; the set of *active* bases is what creation-time RAG injection
// searches. Bases can be hand-curated or auto-filled from source material.

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { isDesktop, NotInDesktopError } from "./core";

/** What a knowledge entry is about. */
export type KnowledgeKind =
  | "character"
  | "location"
  | "worldbuilding"
  | "event"
  | "item"
  | "term"
  | "lore"
  | "other";

/** Display labels for each kind. */
export const KIND_LABELS: Record<KnowledgeKind, string> = {
  character: "人物",
  location: "地点",
  worldbuilding: "设定",
  event: "事件",
  item: "器物",
  term: "术语",
  lore: "传说",
  other: "其他",
};

export interface KnowledgeBaseMeta {
  id: string;
  name: string;
  description: string;
  active: boolean;
  created_ms: number;
  updated_ms: number;
  entry_count: number;
}

export interface KnowledgeEntry {
  id: string;
  kind: KnowledgeKind;
  title: string;
  content: string;
  source: string;
  tags: string[];
  created_ms: number;
}

export interface KnowledgeHit {
  entry: KnowledgeEntry;
  kb_id: string;
  kb_name: string;
  score: number;
}

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isDesktop()) throw new NotInDesktopError();
  return (await tauriInvoke(cmd, args)) as T;
}

/** List the active work's knowledge bases. */
export async function listBases(): Promise<KnowledgeBaseMeta[]> {
  return invoke<KnowledgeBaseMeta[]>("knowledge_list_bases");
}

/** Create a new knowledge base in the active work. */
export async function createBase(
  name: string,
  description = "",
): Promise<KnowledgeBaseMeta> {
  return invoke<KnowledgeBaseMeta>("knowledge_create_base", { name, description });
}

/** Delete a knowledge base. */
export async function deleteBase(kbId: string): Promise<void> {
  return invoke<void>("knowledge_delete_base", { kbId });
}

/** Toggle whether a base participates in RAG retrieval. */
export async function setBaseActive(
  kbId: string,
  active: boolean,
): Promise<KnowledgeBaseMeta> {
  return invoke<KnowledgeBaseMeta>("knowledge_set_active", { kbId, active });
}

/** Rename / re-describe a base. */
export async function updateBase(
  kbId: string,
  patch: { name?: string; description?: string },
): Promise<KnowledgeBaseMeta> {
  return invoke<KnowledgeBaseMeta>("knowledge_update_base", {
    kbId,
    name: patch.name ?? null,
    description: patch.description ?? null,
  });
}

/** List a base's entries (full content, newest first). */
export async function listEntries(kbId: string): Promise<KnowledgeEntry[]> {
  return invoke<KnowledgeEntry[]>("knowledge_list_entries", { kbId });
}

/** Add an entry to a base; returns the new entry id. */
export async function addEntry(input: {
  kbId: string;
  kind: KnowledgeKind;
  title: string;
  content: string;
  source?: string;
  tags?: string[];
}): Promise<string> {
  return invoke<string>("knowledge_add_entry", {
    kbId: input.kbId,
    kind: input.kind,
    title: input.title,
    content: input.content,
    source: input.source ?? "user",
    tags: input.tags ?? [],
  });
}

/** Remove an entry from a base. */
export async function deleteEntry(kbId: string, entryId: string): Promise<void> {
  return invoke<void>("knowledge_delete_entry", { kbId, entryId });
}

/** Search across all active bases (RAG preview). */
export async function searchKnowledge(
  query: string,
  k = 8,
): Promise<KnowledgeHit[]> {
  return invoke<KnowledgeHit[]>("knowledge_search", { query, k });
}

/**
 * Auto-fill a base from a topic / source material using the active model.
 * Streams progress on the `knowledge-delta` event. Returns the number added.
 */
export async function fillFromTopic(
  kbId: string,
  topic: string,
): Promise<{ added: number; raw: string }> {
  return invoke<{ added: number; raw: string }>("knowledge_fill_web", {
    kbId,
    topic,
  });
}
