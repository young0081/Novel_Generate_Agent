import assert from "node:assert/strict";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import test from "node:test";

import { CoreClient, newestExistingBinary } from "./coreClient.ts";

const responder = `
const readline = require("node:readline");
const lines = readline.createInterface({ input: process.stdin });
lines.on("line", (line) => {
  const request = JSON.parse(line);
  process.stdout.write(JSON.stringify({ jsonrpc: "2.0", id: request.id, result: "pong" }) + "\\n");
});
`;

const sequentialResponder = `
const fs = require("node:fs");
const readline = require("node:readline");
const marker = process.argv[1];
const queue = [];
let active = false;
function drain() {
  if (active || queue.length === 0) return;
  active = true;
  const request = queue.shift();
  setTimeout(() => {
    process.stdout.write(JSON.stringify({ jsonrpc: "2.0", id: request.id, result: request.id }) + "\\n");
    active = false;
    drain();
  }, 200);
}
const lines = readline.createInterface({ input: process.stdin });
lines.on("line", (line) => {
  fs.appendFileSync(marker, "received\\n");
  queue.push(JSON.parse(line));
  drain();
});
`;

test("selects the newest existing host binary", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "novel-host-selection-"));
  const stale = path.join(root, "release-host");
  const current = path.join(root, "debug-host");
  fs.writeFileSync(stale, "stale");
  fs.writeFileSync(current, "current");
  const now = Date.now() / 1_000;
  fs.utimesSync(stale, now - 60, now - 60);
  fs.utimesSync(current, now, now);

  try {
    assert.equal(newestExistingBinary([stale, current]), current);
    assert.equal(newestExistingBinary([path.join(root, "missing")]), undefined);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("a timed-out mutation is terminated before it can commit", async () => {
  const workspace = fs.mkdtempSync(path.join(os.tmpdir(), "novel-core-client-"));
  const lateWrite = path.join(workspace, "late-write.txt");
  let launches = 0;
  const options = {
    requestTimeoutMs: 60,
    workspace: () => workspace,
    launch: async () => {
      launches += 1;
      if (launches === 1) {
        return {
          cmd: process.execPath,
          args: [
            "-e",
            `
const fs = require("node:fs");
const target = process.argv[1];
process.stdin.once("data", () => {
  setTimeout(() => fs.writeFileSync(target, "late"), 200);
});
process.stdin.resume();
`,
            lateWrite,
          ],
        };
      }
      return { cmd: process.execPath, args: ["-e", responder] };
    },
  };
  const client = new CoreClient(options);

  try {
    await assert.rejects(client.rpc("write_file", { path: "late.txt" }), /timed out/);
    await new Promise((resolve) => setTimeout(resolve, 250));
    assert.equal(fs.existsSync(lateWrite), false);

    options.requestTimeoutMs = 1_000;
    const response = await client.rpc("ping", {});
    assert.equal(response.result, "pong");
    assert.equal(launches, 2);
  } finally {
    await client.close();
    fs.rmSync(workspace, { recursive: true, force: true });
  }
});

test("operation timeout starts after earlier work leaves the client queue", async () => {
  const workspace = fs.mkdtempSync(path.join(os.tmpdir(), "novel-core-queue-"));
  const received = path.join(workspace, "received.txt");
  const client = new CoreClient({
    requestTimeoutMs: 1_000,
    workspace: () => workspace,
    launch: async () => ({
      cmd: process.execPath,
      args: ["-e", sequentialResponder, received],
    }),
  });

  try {
    const firstCall = client.rpc("write_file", { path: "one" });
    const secondCall = client.rpc("write_file", { path: "two" });
    for (let attempt = 0; attempt < 100 && !fs.existsSync(received); attempt += 1) {
      await new Promise((resolve) => setTimeout(resolve, 5));
    }
    assert.equal(fs.readFileSync(received, "utf8"), "received\n");

    const [first, second] = await Promise.all([firstCall, secondCall]);
    assert.equal(first.result, 1);
    assert.equal(second.result, 2);
    assert.equal(fs.readFileSync(received, "utf8"), "received\nreceived\n");
  } finally {
    await client.close();
    fs.rmSync(workspace, { recursive: true, force: true });
  }
});

test("queued mutations from a timed-out host generation are discarded", async () => {
  const workspace = fs.mkdtempSync(path.join(os.tmpdir(), "novel-core-generation-"));
  let launches = 0;
  const options = {
    requestTimeoutMs: 60,
    workspace: () => workspace,
    launch: async () => {
      launches += 1;
      return launches === 1
        ? { cmd: process.execPath, args: ["-e", "process.stdin.resume()"] }
        : { cmd: process.execPath, args: ["-e", responder] };
    },
  };
  const client = new CoreClient(options);

  try {
    const [active, queued] = await Promise.allSettled([
      client.rpc("write_file", { path: "active" }),
      client.rpc("write_file", { path: "queued" }),
    ]);
    assert.equal(active.status, "rejected");
    assert.match(String(active.reason), /timed out/);
    assert.equal(queued.status, "rejected");
    assert.match(String(queued.reason), /cancelled after host restart/);
    assert.equal(launches, 1);

    options.requestTimeoutMs = 1_000;
    const response = await client.rpc("ping", {});
    assert.equal(response.result, "pong");
    assert.equal(launches, 2);
  } finally {
    await client.close();
    fs.rmSync(workspace, { recursive: true, force: true });
  }
});
