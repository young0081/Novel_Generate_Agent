import assert from "node:assert/strict";
import test from "node:test";

import {
  createCloseRequestCoordinator,
  waitForCloseOperation,
} from "../src/lib/windowClose.ts";

test("close operation timeout bounds a hung save", async () => {
  const startedAt = Date.now();
  const result = await waitForCloseOperation(new Promise(() => {}), 20);

  assert.deepEqual(result, { status: "timed-out" });
  assert.ok(Date.now() - startedAt < 500);
});

test("close operation preserves a normal save result", async () => {
  const result = await waitForCloseOperation(Promise.resolve(true), 100);

  assert.deepEqual(result, { status: "completed", value: true });
});

test("repeated close gestures share one preparation and one destroy", async () => {
  let resolvePreparation;
  let prepareCalls = 0;
  let destroyCalls = 0;
  const preparation = new Promise((resolve) => {
    resolvePreparation = resolve;
  });
  const coordinator = createCloseRequestCoordinator(
    async () => {
      prepareCalls += 1;
      return preparation;
    },
    async () => {
      destroyCalls += 1;
    },
  );

  const first = coordinator.requestClose();
  const second = coordinator.requestClose();
  assert.equal(first, second);
  assert.equal(prepareCalls, 1);

  resolvePreparation(true);
  await Promise.all([first, second]);
  await coordinator.requestClose();
  assert.equal(prepareCalls, 1);
  assert.equal(destroyCalls, 1);
});

test("a cancelled close can be retried", async () => {
  let prepareCalls = 0;
  let destroyCalls = 0;
  const coordinator = createCloseRequestCoordinator(
    async () => {
      prepareCalls += 1;
      return prepareCalls > 1;
    },
    async () => {
      destroyCalls += 1;
    },
  );

  await coordinator.requestClose();
  assert.equal(destroyCalls, 0);
  await coordinator.requestClose();
  assert.equal(prepareCalls, 2);
  assert.equal(destroyCalls, 1);
});

test("a failed destroy does not wedge later close attempts", async () => {
  let destroyCalls = 0;
  const errors = [];
  const coordinator = createCloseRequestCoordinator(
    async () => true,
    async () => {
      destroyCalls += 1;
      if (destroyCalls === 1) throw new Error("temporary destroy failure");
    },
    (error) => errors.push(error),
  );

  await coordinator.requestClose();
  await coordinator.requestClose();
  assert.equal(destroyCalls, 2);
  assert.equal(errors.length, 1);
});
