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

const REQUEST_TIMEOUT_MS = 180_000; // generous: first call may trigger a cargo build

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
 * Decide how to launch the host: a prebuilt binary if present (instant start),
 * otherwise `cargo run` (builds on first use). Returns [command, args].
 */
function launchCommand(ws: string): { cmd: string; args: string[]; useShell: boolean } {
  const exe = process.platform === "win32" ? "host.exe" : "host";
  const candidates = [
    process.env.NOVEL_HOST_BIN,
    path.join(coreDir(), "target", "release", exe),
    path.join(coreDir(), "target", "debug", exe),
  ].filter((p): p is string => Boolean(p));

  for (const bin of candidates) {
    if (fs.existsSync(bin)) {
      return { cmd: bin, args: [ws], useShell: false };
    }
  }
  // Fallback: build & run via cargo. `shell:true` on Windows lets PATH resolve cargo.
  return {
    cmd: "cargo",
    args: ["run", "-q", "-p", "na-host", "--bin", "host", "--", ws],
    useShell: process.platform === "win32",
  };
}

class CoreClient {
  private child: ChildProcess | null = null;
  private pending = new Map<number, Pending>();
  private nextId = 1;
  private starting: Promise<void> | null = null;

  private async ensureStarted(): Promise<void> {
    if (this.child && this.child.exitCode === null && !this.child.killed) {
      return;
    }
    if (this.starting) {
      return this.starting;
    }

    this.starting = new Promise<void>((resolve, reject) => {
      try {
        const ws = workspaceDir();
        fs.mkdirSync(ws, { recursive: true });

        const { cmd, args, useShell } = launchCommand(ws);
        const child = spawn(cmd, args, {
          cwd: coreDir(),
          stdio: ["pipe", "pipe", "pipe"],
          shell: useShell,
        });

        child.on("error", (err) => {
          this.failAll(new Error(`core host failed to start: ${err.message}`));
          this.child = null;
        });

        child.on("exit", (code) => {
          this.failAll(new Error(`core host exited (code ${code ?? "?"})`));
          this.child = null;
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
        resolve();
      } catch (err) {
        reject(err as Error);
      }
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

  async rpc(method: string, params: unknown): Promise<RpcResponse> {
    await this.ensureStarted();
    const child = this.child;
    if (!child || !child.stdin) {
      throw new Error("core host is not available");
    }

    const id = this.nextId++;
    const payload = JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n";

    return new Promise<RpcResponse>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`core RPC "${method}" timed out`));
      }, REQUEST_TIMEOUT_MS);

      this.pending.set(id, { resolve, reject, timer });
      child.stdin!.write(payload, (err) => {
        if (err) {
          clearTimeout(timer);
          this.pending.delete(id);
          reject(err);
        }
      });
    });
  }
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
