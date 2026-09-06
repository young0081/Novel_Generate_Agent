export interface BatchFailure<T> {
  item: T;
  error: unknown;
}
export interface BatchResult<T> {
  completed: T[];
  failed: BatchFailure<T>[];
}

/**
 * Run destructive actions one at a time so backend workspace locks and local
 * stores are respected. A failed item does not hide failures from later items.
 */
export async function runBatch<T>(
  items: readonly T[],
  action: (item: T) => Promise<void>,
  onProgress?: (completed: number, total: number) => void,
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
