import assert from "node:assert/strict";
import test from "node:test";

import { runBatch } from "./batch.ts";

test("runBatch processes items in order and continues after failures", async () => {
  const order: number[] = [];
  const progress: Array<[number, number]> = [];

  const result = await runBatch(
    [1, 2, 3],
    async (item) => {
      order.push(item);
      if (item === 2) throw new Error("expected failure");
    },
    (done, total) => progress.push([done, total]),
  );

  assert.deepEqual(order, [1, 2, 3]);
  assert.deepEqual(result.completed, [1, 3]);
  assert.equal(result.failed.length, 1);
  assert.equal(result.failed[0]?.item, 2);
  assert.deepEqual(progress, [[1, 3], [2, 3], [3, 3]]);
});

test("runBatch handles an empty selection without invoking the action", async () => {
  let calls = 0;
  const result = await runBatch([], async () => {
    calls += 1;
  });

  assert.equal(calls, 0);
  assert.deepEqual(result, { completed: [], failed: [] });
});
