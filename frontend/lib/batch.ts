/** Result of a best-effort sequential batch operation. */
export interface BatchFailure<T> {
  item: T;
  error: unknown;
}

export interface BatchResult<T> {
  completed: T[];
  failed: BatchFailure<T>[];
}

/**
 * Run mutations one at a time. The core owns workspace/store locks, so
 * serial execution avoids races while still allowing later items to proceed
 * when one item fails.
 */
export async function runBatch<T>(
  items: readonly T[],
  action: (item: T) => Promise<void>,
  onProgress?: (processed: number, total: number) => void,
): Promise<BatchResult<T>> {
  const completed: T[] = [];
  const failed: BatchFailure<T>[] = [];
  for (const item of items) {
    try {
      await action(item);
      completed.push(item);
    } catch (error) {
      failed.push({ item, error });
    }
    onProgress?.(completed.length + failed.length, items.length);
  }
  return { completed, failed };
}
