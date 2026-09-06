export interface RevisionSaveSnapshot {
  path: string | null;
  content: string;
  savedContent: string;
}

export interface RevisionSaveQueue {
  enqueue: (path: string, content: string) => Promise<boolean>;
  flushLatest: (readSnapshot: () => RevisionSaveSnapshot) => Promise<boolean>;
  drain: () => Promise<boolean>;
}

export interface RevisionReadMetadata {
  bytes: number;
  truncated: boolean;
  was_binary: boolean;
  redactions: number;
}

export function revisionReadOnlyReason(
  metadata: RevisionReadMetadata,
  rawBytes?: number,
): string | null {
  if (metadata.was_binary) return "该文件包含二进制内容，修订页仅供查看，不能覆盖保存。";
  if (metadata.truncated) return "文件内容已被截断显示，为避免覆盖完整原稿，修订页已设为只读。";
  if (metadata.redactions > 0) return "文件内容包含已隐藏信息，为避免用脱敏文本覆盖原稿，修订页已设为只读。";
  if (typeof rawBytes === "number" && rawBytes !== metadata.bytes) {
    return "文件内容在读取时经过了安全处理，为避免覆盖原稿，修订页已设为只读。";
  }
  return null;
}

export type RevisionCreateResult = "created" | "overwritten" | "cancelled";

export async function createRevisionDocumentSafely(options: {
  path: string;
  content: string;
  create: (path: string, content: string, overwrite: boolean) => Promise<void>;
  isConflict: (error: unknown) => boolean;
  confirmOverwrite: () => boolean;
}): Promise<RevisionCreateResult> {
  try {
    await options.create(options.path, options.content, false);
    return "created";
  } catch (error) {
    if (!options.isConflict(error)) throw error;
    if (!options.confirmOverwrite()) return "cancelled";
    await options.create(options.path, options.content, true);
    return "overwritten";
  }
}

/** Serialize revision writes and, during a flush, keep saving until disk catches up. */
export function createRevisionSaveQueue(
  write: (path: string, content: string) => Promise<boolean>,
): RevisionSaveQueue {
  let chain: Promise<boolean> = Promise.resolve(true);
  let latest: { path: string; content: string; promise: Promise<boolean> } | null = null;
  let pendingCount = 0;

  const enqueue = (path: string, content: string): Promise<boolean> => {
    if (latest?.path === path && latest.content === content) return latest.promise;

    const promise = chain.catch(() => false).then(() => write(path, content));
    chain = promise;
    pendingCount += 1;
    latest = { path, content, promise };
    void promise.then(
      () => {
        pendingCount = Math.max(0, pendingCount - 1);
        if (latest?.promise === promise) latest = null;
      },
      () => {
        pendingCount = Math.max(0, pendingCount - 1);
        if (latest?.promise === promise) latest = null;
      },
    );
    return promise;
  };

  const drain = (): Promise<boolean> => chain.catch(() => false);

  const flushLatest = async (
    readSnapshot: () => RevisionSaveSnapshot,
  ): Promise<boolean> => {
    // Capture exactly once while the caller's write barrier is held. If an outer
    // close timeout later releases that barrier, this flush must not ingest text
    // the user types after editing resumes.
    const snapshot = readSnapshot();
    if (!snapshot.path) return drain();
    if (snapshot.content === snapshot.savedContent && pendingCount === 0) return true;
    return enqueue(snapshot.path, snapshot.content);
  };

  return { enqueue, flushLatest, drain };
}
