// Shared types + helpers for the memory-backed screens
// (人物 / 伏笔 / 设定).

export type MemoryKind =
  | "character"
  | "setting"
  | "worldbuilding"
  | "plot"
  | "outline"
  | "foreshadow"
  | "dialogue"
  | "lore"
  | "other";

export interface MemoryHit {
  id: string;
  kind: MemoryKind;
  title: string;
  summary: string;
  tags: string[];
  importance: number;
  /** Relevance score — present only for search (memory_recall), not for listing. */
  score?: number;
}

export interface MemoryRecallData {
  hits: MemoryHit[];
}

export interface MemoryListData {
  entries: MemoryHit[];
  count?: number;
  total?: number;
}

export const KIND_LABEL: Record<MemoryKind, string> = {
  character: "人物",
  setting: "设定",
  worldbuilding: "世界观",
  plot: "情节",
  outline: "大纲",
  foreshadow: "伏笔",
  dialogue: "对白",
  lore: "传说",
  other: "其他",
};

export const ALL_KINDS: MemoryKind[] = [
  "character",
  "setting",
  "worldbuilding",
  "plot",
  "outline",
  "foreshadow",
  "dialogue",
  "lore",
  "other",
];
