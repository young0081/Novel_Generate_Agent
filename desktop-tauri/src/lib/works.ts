// Typed client for the 书库 (multi-work) commands.
//
// Each "work" is a fully isolated novel project: its own manuscript workspace,
// memory, checkpoints, sessions, story-state, and knowledge bases. Switching the
// active work rebuilds the core engine on the backend, so there's zero
// cross-contamination between books.

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { isDesktop, NotInDesktopError } from "./core";

/** Full metadata for one work (novel project). */
export interface WorkMeta {
  id: string;
  title: string;
  blurb: string;
  genre: string;
  source_material: string;
  created_ms: number;
  updated_ms: number;
  workspace_dir: string;
  sessions_dir: string;
  knowledge_dir: string;
}

/** A lightweight library-list summary, with the active flag. */
export interface WorkSummary {
  id: string;
  title: string;
  blurb: string;
  genre: string;
  source_material: string;
  created_ms: number;
  updated_ms: number;
  active: boolean;
}

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isDesktop()) throw new NotInDesktopError();
  return (await tauriInvoke(cmd, args)) as T;
}

/** Every work, newest-updated first, with the active one flagged. */
export async function listWorks(): Promise<WorkSummary[]> {
  return invoke<WorkSummary[]>("works_list");
}

/** The active work's metadata (or null if none). */
export async function currentWork(): Promise<WorkMeta | null> {
  return invoke<WorkMeta | null>("works_current");
}

/** Create a new work and switch to it. Returns the created work. */
export async function createWork(input: {
  title: string;
  blurb?: string;
  genre?: string;
  source_material?: string;
}): Promise<WorkMeta> {
  return invoke<WorkMeta>("works_create", {
    title: input.title,
    blurb: input.blurb ?? "",
    genre: input.genre ?? "",
    sourceMaterial: input.source_material ?? "",
  });
}

/** Switch the active work; returns the refreshed list. */
export async function openWork(id: string): Promise<WorkSummary[]> {
  return invoke<WorkSummary[]>("works_open", { id });
}

/** Rename / re-blurb / re-tag a work. */
export async function updateWork(
  id: string,
  patch: { title?: string; blurb?: string; genre?: string; source_material?: string },
): Promise<WorkMeta> {
  return invoke<WorkMeta>("works_update", {
    id,
    title: patch.title ?? null,
    blurb: patch.blurb ?? null,
    genre: patch.genre ?? null,
    sourceMaterial: patch.source_material ?? null,
  });
}

/** Delete a work (purges its files by default); returns the refreshed list. */
export async function deleteWork(id: string, purgeFiles = true): Promise<WorkSummary[]> {
  return invoke<WorkSummary[]>("works_delete", { id, purgeFiles });
}
