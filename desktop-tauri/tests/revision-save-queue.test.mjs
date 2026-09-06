import assert from "node:assert/strict";
import test from "node:test";

import {
  acquireEditorWriteBarrier,
  areEditorWritesBlocked,
  flushPendingEditors,
  registerEditorFlush,
} from "../src/lib/editorPersistence.ts";
import {
  createRevisionDocumentSafely,
  createRevisionSaveQueue,
  revisionReadOnlyReason,
} from "../src/lib/revisionSaveQueue.ts";

test("processed or incomplete reads are never considered writable", () => {
  const safe = { bytes: 12, truncated: false, was_binary: false, redactions: 0 };
  assert.equal(revisionReadOnlyReason(safe, 12), null);
  assert.match(revisionReadOnlyReason({ ...safe, truncated: true }, 12), /截断/);
  assert.match(revisionReadOnlyReason({ ...safe, was_binary: true }, 12), /二进制/);
  assert.match(revisionReadOnlyReason({ ...safe, redactions: 1 }, 12), /隐藏/);
  assert.match(revisionReadOnlyReason(safe, 18), /安全处理/);
});

test("an existing chapter is not overwritten without explicit confirmation", async () => {
  const conflict = new Error("WORKSPACE_TARGET_EXISTS");
  const calls = [];
  const result = await createRevisionDocumentSafely({
    path: "book/ch1.md",
    content: "new",
    create: async (path, content, overwrite) => {
      calls.push({ path, content, overwrite });
      throw conflict;
    },
    isConflict: (error) => error === conflict,
    confirmOverwrite: () => false,
  });

  assert.equal(result, "cancelled");
  assert.deepEqual(calls, [{ path: "book/ch1.md", content: "new", overwrite: false }]);
});

test("confirmed chapter overwrite uses the atomic overwrite command", async () => {
  const conflict = new Error("WORKSPACE_TARGET_EXISTS");
  const overwriteFlags = [];
  const result = await createRevisionDocumentSafely({
    path: "book/ch1.md",
    content: "new",
    create: async (_path, _content, overwrite) => {
      overwriteFlags.push(overwrite);
      if (!overwrite) throw conflict;
    },
    isConflict: (error) => error === conflict,
    confirmOverwrite: () => true,
  });

  assert.equal(result, "overwritten");
  assert.deepEqual(overwriteFlags, [false, true]);
});

test("revision writes are serialized in request order", async () => {
  const started = [];
  let finishFirst;
  let markFirstStarted;
  const firstStarted = new Promise((resolve) => { markFirstStarted = resolve; });
  const queue = createRevisionSaveQueue(async (_path, content) => {
    started.push(content);
    if (content === "first") {
      markFirstStarted();
      await new Promise((resolve) => { finishFirst = resolve; });
    }
    return true;
  });

  const first = queue.enqueue("book/ch1.md", "first");
  const second = queue.enqueue("book/ch1.md", "second");
  await firstStarted;
  assert.deepEqual(started, ["first"]);

  finishFirst();
  assert.equal(await first, true);
  assert.equal(await second, true);
  assert.deepEqual(started, ["first", "second"]);
});

test("flush saves an edit that lands while an older write is in flight", async () => {
  const snapshot = {
    path: "book/ch1.md",
    content: "first",
    savedContent: "base",
  };
  const writes = [];
  let finishFirst;
  let markFirstStarted;
  const firstStarted = new Promise((resolve) => { markFirstStarted = resolve; });
  const queue = createRevisionSaveQueue(async (_path, content) => {
    writes.push(content);
    if (content === "first") {
      markFirstStarted();
      await new Promise((resolve) => { finishFirst = resolve; });
    }
    snapshot.savedContent = content;
    return true;
  });

  const first = queue.enqueue(snapshot.path, snapshot.content);
  snapshot.content = "second";
  const flushed = queue.flushLatest(() => ({ ...snapshot }));
  await firstStarted;
  finishFirst();

  assert.equal(await first, true);
  assert.equal(await flushed, true);
  assert.deepEqual(writes, ["first", "second"]);
  assert.equal(snapshot.savedContent, "second");
});

test("a failed flush blocks navigation and can be retried", async () => {
  const snapshot = {
    path: "book/ch1.md",
    content: "unsaved",
    savedContent: "saved",
  };
  let attempts = 0;
  const queue = createRevisionSaveQueue(async (_path, content) => {
    attempts += 1;
    if (attempts === 1) return false;
    snapshot.savedContent = content;
    return true;
  });

  assert.equal(await queue.flushLatest(() => ({ ...snapshot })), false);
  assert.equal(snapshot.savedContent, "saved");
  assert.equal(await queue.flushLatest(() => ({ ...snapshot })), true);
  assert.equal(snapshot.savedContent, "unsaved");
  assert.equal(attempts, 2);
});

test("flush freezes its snapshot before a close timeout can release the barrier", async () => {
  const snapshot = {
    path: "book/ch1.md",
    content: "captured before timeout",
    savedContent: "saved",
  };
  const writes = [];
  let finishWrite;
  let markWriteStarted;
  const writeStarted = new Promise((resolve) => { markWriteStarted = resolve; });
  const queue = createRevisionSaveQueue(async (_path, content) => {
    writes.push(content);
    markWriteStarted();
    await new Promise((resolve) => { finishWrite = resolve; });
    snapshot.savedContent = content;
    return true;
  });

  const flushed = queue.flushLatest(() => ({ ...snapshot }));
  await writeStarted;
  snapshot.content = "typed after timeout";
  finishWrite();

  assert.equal(await flushed, true);
  assert.deepEqual(writes, ["captured before timeout"]);
  assert.equal(snapshot.savedContent, "captured before timeout");
  assert.equal(snapshot.content, "typed after timeout");
});

test("flush restores a clean-looking draft after an older save completes", async () => {
  const snapshot = {
    path: "book/ch1.md",
    content: "intermediate",
    savedContent: "original",
  };
  const writes = [];
  let finishFirst;
  let markFirstStarted;
  const firstStarted = new Promise((resolve) => { markFirstStarted = resolve; });
  const queue = createRevisionSaveQueue(async (_path, content) => {
    writes.push(content);
    if (content === "intermediate") {
      markFirstStarted();
      await new Promise((resolve) => { finishFirst = resolve; });
    }
    snapshot.savedContent = content;
    return true;
  });

  const first = queue.enqueue(snapshot.path, snapshot.content);
  await firstStarted;
  snapshot.content = "original";
  const flushed = queue.flushLatest(() => ({ ...snapshot }));
  finishFirst();

  assert.equal(await first, true);
  assert.equal(await flushed, true);
  assert.deepEqual(writes, ["intermediate", "original"]);
  assert.equal(snapshot.savedContent, "original");
});

test("registered revision flush participates in the global write barrier", async () => {
  const locks = [];
  let flushCalls = 0;
  const unregister = registerEditorFlush(
    async () => {
      flushCalls += 1;
      return true;
    },
    (locked) => locks.push(locked),
  );
  const release = acquireEditorWriteBarrier();

  try {
    assert.equal(areEditorWritesBlocked(), true);
    assert.deepEqual(locks, [true]);
    assert.equal(await flushPendingEditors(), true);
    assert.equal(flushCalls, 1);
  } finally {
    unregister();
    release();
  }

  assert.equal(areEditorWritesBlocked(), false);
  assert.equal(await flushPendingEditors(), true);
  assert.equal(flushCalls, 1);
});
