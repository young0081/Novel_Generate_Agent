// Coordinates pending editor writes before tabs switch or the desktop window
// closes. Editors register a small flush callback while mounted.

export type EditorFlush = () => Promise<boolean>;
export type EditorWriteLock = (locked: boolean) => void;
export type EditorRunCanceller = () => void;

const flushers = new Set<EditorFlush>();
const writeLocks = new Set<EditorWriteLock>();
const runCancellers = new Set<EditorRunCanceller>();
let writeBarrierDepth = 0;
let workspaceActionActive = false;
const workspaceActionWaiters = new Set<() => void>();

export function registerEditorFlush(
  flush: EditorFlush,
  setWriteLocked?: EditorWriteLock,
): () => void {
  flushers.add(flush);
  if (setWriteLocked) {
    writeLocks.add(setWriteLocked);
    if (writeBarrierDepth > 0) setWriteLocked(true);
  }
  return () => {
    flushers.delete(flush);
    if (setWriteLocked) writeLocks.delete(setWriteLocked);
  };
}

export async function flushPendingEditors(): Promise<boolean> {
  const results = await Promise.all(
    [...flushers].map(async (flush) => {
      try {
        return await flush();
      } catch {
        return false;
      }
    }),
  );
  return results.every(Boolean);
}

/**
 * Freeze every mounted editor before an agent flushes and takes ownership of
 * the workspace. The imperative lock closes the event-to-render window where
 * another keystroke could otherwise queue a stale autosave after the flush.
 */
export function acquireEditorWriteBarrier(): () => void {
  writeBarrierDepth += 1;
  if (writeBarrierDepth === 1) {
    for (const setWriteLocked of [...writeLocks]) setWriteLocked(true);
  }

  let released = false;
  return () => {
    if (released) return;
    released = true;
    writeBarrierDepth = Math.max(0, writeBarrierDepth - 1);
    if (writeBarrierDepth === 0) {
      for (const setWriteLocked of [...writeLocks]) setWriteLocked(false);
    }
  };
}

export function areEditorWritesBlocked(): boolean {
  return writeBarrierDepth > 0;
}

/** Register a synchronous cancellation hook for an IDE agent run. */
export function registerEditorRunCanceller(cancel: EditorRunCanceller): () => void {
  runCancellers.add(cancel);
  return () => runCancellers.delete(cancel);
}

/**
 * Signal cancellation before navigation starts awaiting an editor flush. This
 * prevents the run waiting on that same flush from winning the continuation
 * race and launching just before its component unmounts.
 */
export function cancelPendingEditorRuns(): void {
  for (const cancel of [...runCancellers]) cancel();
}

/**
 * Claim the IDE workspace for one user or agent mutation sequence. Acquisition
 * is synchronous so two event handlers cannot both pass their initial guard.
 */
export function tryAcquireWorkspaceAction(): (() => void) | null {
  if (workspaceActionActive) return null;
  workspaceActionActive = true;
  let released = false;
  return () => {
    if (released) return;
    released = true;
    workspaceActionActive = false;
    for (const resolve of [...workspaceActionWaiters]) resolve();
    workspaceActionWaiters.clear();
  };
}

/** Wait for the current owner, then atomically claim the next action slot. */
export async function acquireWorkspaceAction(): Promise<() => void> {
  for (;;) {
    const release = tryAcquireWorkspaceAction();
    if (release) return release;
    await new Promise<void>((resolve) => workspaceActionWaiters.add(resolve));
  }
}
