// Server-side bridge to the Rust core.
//
// Spawns the `na-host` binary once (a singleton, persistent process) and speaks
// line-delimited JSON-RPC 2.0 to it over stdio — exactly the protocol the host
// implements. Requests are matched to responses by id. This module is
// server-only (it uses Node's child_process) and must never be imported from a
// client component.

import { spawn, type ChildProcess } from "node:child_process";
import * as readline from "node:readline";
import * as path from "node:path";
import * as fs from "node:fs";

import type { RpcResponse } from "./types";

interface Pending {
  resolve: (value: RpcResponse) => void;
  reject: (err: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

interface LaunchCommand {
  cmd: string;
  args: string[];
}

interface CoreClientOptions {
  requestTimeoutMs?: number;
  launch?: (workspace: string) => Promise<LaunchCommand>;
  workspace?: () => string;
}

const REQUEST_TIMEOUT_MS = 180_000;
const MAX_BUILD_ERROR_LENGTH = 4_000;

/**
 * Resolve the core workspace directory (where manuscripts / stores live).
 */
function workspaceDir(): string {
  return (
    process.env.NOVEL_WORKSPACE ||
    path.resolve(process.cwd(), "..", "runtime-data", "workspace")
  );
}

/**
 * Resolve the core cargo workspace directory.
 */
function coreDir(): string {
  return process.env.NOVEL_CORE_DIR || path.resolve(process.cwd(), "..", "core");
}

/**
 * Decide how to launch the host: use a prebuilt binary when present, otherwise
 * build the debug binary once. The long-running child is always the host itself
 * so a request timeout can terminate all active and queued work reliably.
 */
async function launchCommand(ws: string): Promise<LaunchCommand> {
  const exe = process.platform === "win32" ? "host.exe" : "host";
  const configured = process.env.NOVEL_HOST_BIN;
  if (configured) {
    if (!fs.existsSync(configured)) {
      throw new Error(`NOVEL_HOST_BIN does not exist: ${configured}`);
    }
    return { cmd: configured, args: [ws] };
  }

  const candidates = [
    path.join(coreDir(), "target", "release", exe),
    path.join(coreDir(), "target", "debug", exe),
  ];

  const existing = newestExistingBinary(candidates);
  if (existing) {
    return { cmd: existing, args: [ws] };
  }

  await new Promise<void>((resolve, reject) => {
    const build = spawn("cargo", ["build", "-q", "-p", "na-host", "--bin", "host"], {
      cwd: coreDir(),
      stdio: ["ignore", "ignore", "pipe"],
      shell: false,
      windowsHide: true,
    });
    let stderr = "";
    build.stderr?.setEncoding("utf8");
    build.stderr?.on("data", (chunk: string) => {
      stderr = (stderr + chunk).slice(-MAX_BUILD_ERROR_LENGTH);
    });
    build.once("error", (error) => reject(new Error(`failed to build core host: ${error.message}`)));
    build.once("close", (code) => {
      if (code === 0) {
        resolve();
      } else {
        reject(
          new Error(
            `failed to build core host (code ${code ?? "?"})${stderr.trim() ? `: ${stderr.trim()}` : ""}`,
          ),
        );
      }
    });
  });

  const debugHost = path.join(coreDir(), "target", "debug", exe);
  if (!fs.existsSync(debugHost)) {
    throw new Error(`cargo completed without producing ${debugHost}`);
  }
  return { cmd: debugHost, args: [ws] };
}

export function newestExistingBinary(candidates: string[]): string | undefined {
  return candidates
    .filter((candidate) => fs.existsSync(candidate))
    .map((candidate) => ({ candidate, modified: fs.statSync(candidate).mtimeMs }))
    .sort((left, right) => right.modified - left.modified)[0]?.candidate;
}

export class CoreClient {
  private child: ChildProcess | null = null;
  private pending = new Map<number, Pending>();
  private nextId = 1;
  private starting: Promise<void> | null = null;
  private stopping: Promise<void> | null = null;
  private operationTail: Promise<void> = Promise.resolve();
  private hostGeneration = 0;

  constructor(private readonly options: CoreClientOptions = {}) {}

  private async ensureStarted(): Promise<void> {
    if (this.stopping) {
      await this.stopping;
    }
    if (this.child && this.child.exitCode === null && !this.child.killed) {
      return;
    }
    if (this.starting) {
      return this.starting;
    }

    const generationAtStart = this.hostGeneration;
    this.starting = (async () => {
      const ws = this.options.workspace?.() ?? workspaceDir();
      fs.mkdirSync(ws, { recursive: true });
      const launch = this.options.launch ?? launchCommand;
      const { cmd, args } = await launch(ws);

      await new Promise<void>((resolve, reject) => {
        const child = spawn(cmd, args, {
          cwd: coreDir(),
          stdio: ["pipe", "pipe", "pipe"],
          shell: false,
          windowsHide: true,
        });
        let started = false;

        child.on("error", (err) => {
          this.handleHostFailure(child, new Error(`core host failed: ${err.message}`));
          if (!started) reject(err);
        });

        child.on("exit", (code) => {
          this.handleHostFailure(child, new Error(`core host exited (code ${code ?? "?"})`));
          if (!started) reject(new Error(`core host exited during startup (code ${code ?? "?"})`));
        });

        // Diagnostics (host banner + cargo build messages) go to stderr.
        if (child.stderr) {
          child.stderr.setEncoding("utf8");
          child.stderr.on("data", (chunk: string) => {
            for (const ln of chunk.split(/\r?\n/)) {
              if (ln.trim()) console.error(`[core] ${ln}`);
            }
          });
        }

        // One JSON-RPC response per stdout line.
        if (child.stdout) {
          const rl = readline.createInterface({ input: child.stdout });
          rl.on("line", (line) => this.onLine(line));
        }

        this.child = child;
        child.once("spawn", () => {
          started = true;
          resolve();
        });
      });
    })().catch((error) => {
      if (this.hostGeneration === generationAtStart) this.hostGeneration += 1;
      throw error;
    }).finally(() => {
      this.starting = null;
    });

    return this.starting;
  }

  private onLine(line: string): void {
    const trimmed = line.trim();
    if (!trimmed) return;
    let msg: RpcResponse;
    try {
      msg = JSON.parse(trimmed) as RpcResponse;
    } catch {
      console.error(`[core] non-JSON stdout: ${trimmed}`);
      return;
    }
    const id = typeof msg.id === "number" ? msg.id : Number(msg.id);
    const pending = this.pending.get(id);
    if (!pending) return;
    clearTimeout(pending.timer);
    this.pending.delete(id);
    pending.resolve(msg);
  }

  private failAll(err: Error): void {
    for (const [, p] of this.pending) {
      clearTimeout(p.timer);
      p.reject(err);
    }
    this.pending.clear();
  }

  private handleHostFailure(child: ChildProcess, err: Error): void {
    if (this.child !== child) return;
    this.child = null;
    this.hostGeneration += 1;
    this.failAll(err);
  }

  private terminateHost(err: Error): void {
    const child = this.child;
    this.child = null;
    this.hostGeneration += 1;
    this.failAll(err);
    if (child && child.exitCode === null) {
      const stopped = new Promise<void>((resolve) => {
        let fallback: ReturnType<typeof setTimeout> | undefined;
        const done = () => {
          if (fallback) clearTimeout(fallback);
          resolve();
        };
        child.once("exit", done);
        child.once("error", done);
        fallback = setTimeout(done, 5_000);
        fallback.unref?.();
      });
      let tracked: Promise<void>;
      tracked = stopped.finally(() => {
        if (this.stopping === tracked) this.stopping = null;
      });
      this.stopping = tracked;
      child.stdin?.destroy();
      child.kill();
    }
  }

  async close(): Promise<void> {
    this.terminateHost(new Error("core host client closed"));
    if (this.stopping) await this.stopping;
  }

  async rpc(method: string, params: unknown): Promise<RpcResponse> {
    if (isControlMethod(method)) {
      return this.rpcNow(method, params);
    }

    const generation = this.hostGeneration;
    const operation = this.operationTail
      .catch(() => undefined)
      .then(() => {
        if (generation !== this.hostGeneration) {
          throw new Error(`queued core RPC "${method}" was cancelled after host restart`);
        }
        return this.rpcNow(method, params);
      });
    this.operationTail = operation.then(
      () => undefined,
      () => undefined,
    );
    return operation;
  }

  private async rpcNow(method: string, params: unknown): Promise<RpcResponse> {
    await this.ensureStarted();
    const child = this.child;
    if (!child || !child.stdin) {
      throw new Error("core host is not available");
    }

    const id = this.nextId++;
    const payload = JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n";

    return new Promise<RpcResponse>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.terminateHost(new Error(`core RPC "${method}" timed out`));
      }, this.options.requestTimeoutMs ?? REQUEST_TIMEOUT_MS);

      this.pending.set(id, { resolve, reject, timer });
      child.stdin!.write(payload, (err) => {
        if (err) {
          this.terminateHost(new Error(`failed to write core RPC "${method}": ${err.message}`));
        }
      });
    });
  }
}

function isControlMethod(method: string): boolean {
  return method === "ping" || method === "cancel" || method === "list_tools";
}

// A process-wide singleton (survives across requests within one server process).
const globalForCore = globalThis as unknown as { __novelCore?: CoreClient };
const client: CoreClient = globalForCore.__novelCore ?? new CoreClient();
if (!globalForCore.__novelCore) {
  globalForCore.__novelCore = client;
}

/**
 * Send one JSON-RPC request to the Rust core and return the full response
 * (with `result` or `error`).
 */
export async function coreRpc(method: string, params: unknown): Promise<RpcResponse> {
  return client.rpc(method, params);
}
