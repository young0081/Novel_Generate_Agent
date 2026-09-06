export type CloseOperationResult<T> =
  | { status: "completed"; value: T }
  | { status: "failed"; error: unknown }
  | { status: "timed-out" };

/** Bound an operation while keeping its eventual rejection handled. */
export function waitForCloseOperation<T>(
  operation: Promise<T>,
  timeoutMs: number,
): Promise<CloseOperationResult<T>> {
  return new Promise((resolve) => {
    let settled = false;
    const finish = (result: CloseOperationResult<T>) => {
      if (settled) return;
      settled = true;
      globalThis.clearTimeout(timeoutId);
      resolve(result);
    };
    const timeoutId = globalThis.setTimeout(
      () => finish({ status: "timed-out" }),
      timeoutMs,
    );

    operation.then(
      (value) => finish({ status: "completed", value }),
      (error: unknown) => finish({ status: "failed", error }),
    );
  });
}

export interface CloseRequestCoordinator {
  requestClose: () => Promise<void>;
}

/** Coalesce repeated close gestures and destroy the window at most once. */
export function createCloseRequestCoordinator(
  prepare: () => Promise<boolean>,
  destroy: () => Promise<void>,
  reportError: (error: unknown) => void = (error) => {
    console.error("Failed to close window", error);
  },
): CloseRequestCoordinator {
  let inFlight: Promise<void> | null = null;
  let destroyed = false;

  return {
    requestClose() {
      if (destroyed) return Promise.resolve();
      if (inFlight) return inFlight;

      const request = (async () => {
        if (!(await prepare())) return;
        await destroy();
        destroyed = true;
      })()
        .catch((error: unknown) => {
          try {
            reportError(error);
          } catch {
            // Reporting must never leave the close coordinator wedged.
          }
        })
        .finally(() => {
          if (inFlight === request) inFlight = null;
        });
      inFlight = request;
      return request;
    },
  };
}
