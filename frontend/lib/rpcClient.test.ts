import assert from "node:assert/strict";
import test from "node:test";

import { invokeTool, rpc } from "./rpcClient.ts";
import { isTrustedLocalRpcRequest } from "./requestSecurity.ts";

const originalFetch = globalThis.fetch;

test.afterEach(() => {
  globalThis.fetch = originalFetch;
});

test("invokeTool uses the stable invoke_tool RPC envelope", async () => {
  globalThis.fetch = async (_input, init) => {
    assert.deepEqual(JSON.parse(String(init?.body)), {
      method: "invoke_tool",
      params: { name: "read_file", args: { path: "book/ch1.md" } },
    });
    return Response.json({
      jsonrpc: "2.0",
      id: 1,
      result: { ok: true, content: "chapter", data: null, metadata: {} },
    });
  };

  const result = await invokeTool("read_file", { path: "book/ch1.md" });
  assert.equal(result.content, "chapter");
});

test("rpc preserves a JSON-RPC error returned with a non-2xx status", async () => {
  globalThis.fetch = async () =>
    Response.json(
      { jsonrpc: "2.0", id: null, error: { code: -32000, message: "host unavailable" } },
      { status: 502 },
    );

  await assert.rejects(rpc("ping"), /\[-32000\] host unavailable/);
});

test("rpc reports non-JSON HTTP failures with a bounded body excerpt", async () => {
  globalThis.fetch = async () =>
    new Response("upstream returned HTML", { status: 502, statusText: "Bad Gateway" });

  await assert.rejects(
    rpc("ping"),
    /RPC HTTP error \(502 Bad Gateway\): upstream returned HTML/,
  );
});

test("rpc rejects successful responses that are not JSON-RPC envelopes", async () => {
  globalThis.fetch = async () => Response.json({ status: "ok" });

  await assert.rejects(rpc("ping"), /neither result nor error/);
});

test("local RPC security rejects cross-origin and non-loopback hosts", () => {
  const local = new Request("http://127.0.0.1:3000/api/rpc", {
    headers: { host: "127.0.0.1:3000", origin: "http://127.0.0.1:3000" },
  });
  const crossOrigin = new Request("http://127.0.0.1:3000/api/rpc", {
    headers: { host: "127.0.0.1:3000", origin: "https://attacker.example" },
  });
  const rebound = new Request("http://attacker.example/api/rpc", {
    headers: { host: "attacker.example", origin: "http://attacker.example" },
  });
  const crossScheme = new Request("http://localhost:3000/api/rpc", {
    headers: { host: "localhost:3000", origin: "https://localhost:3000" },
  });
  const defaultPort = new Request("http://localhost/api/rpc", {
    headers: { host: "localhost", origin: "http://localhost:80" },
  });
  const ipv6 = new Request("http://[::1]:3000/api/rpc", {
    headers: { host: "[::1]:3000", origin: "http://[::1]:3000" },
  });
  const localClient = new Request("http://127.0.0.1:3000/api/rpc", {
    headers: { host: "127.0.0.1:3000" },
  });

  assert.equal(isTrustedLocalRpcRequest(local), true);
  assert.equal(isTrustedLocalRpcRequest(crossOrigin), false);
  assert.equal(isTrustedLocalRpcRequest(rebound), false);
  assert.equal(isTrustedLocalRpcRequest(crossScheme), false);
  assert.equal(isTrustedLocalRpcRequest(defaultPort), true);
  assert.equal(isTrustedLocalRpcRequest(ipv6), true);
  assert.equal(isTrustedLocalRpcRequest(localClient), true);
});
