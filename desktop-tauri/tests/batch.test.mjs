import assert from "node:assert/strict";
import test from "node:test";
import { Buffer } from "node:buffer";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";

const bundled = await build({
  entryPoints: [fileURLToPath(new URL("../src/lib/batch.ts", import.meta.url))],
  bundle: true,
  format: "esm",
  platform: "node",
  target: "node22",
  write: false,
});

const moduleUrl = `data:text/javascript;base64,${Buffer.from(bundled.outputFiles[0].text).toString("base64")}`;
const { runBatch } = await import(moduleUrl);

test("runBatch executes sequentially and keeps processing after a failure", async () => {
  const order = [];
  const progress = [];
  const result = await runBatch(
    ["a", "b", "c"],
    async (item) => {
      order.push(item);
      if (item === "b") throw new Error("expected failure");
    },
    (done, total) => progress.push([done, total]),
  );

  assert.deepEqual(order, ["a", "b", "c"]);
  assert.deepEqual(result.completed, ["a", "c"]);
  assert.equal(result.failed.length, 1);
  assert.equal(result.failed[0].item, "b");
  assert.deepEqual(progress, [[1, 3], [2, 3], [3, 3]]);
});
