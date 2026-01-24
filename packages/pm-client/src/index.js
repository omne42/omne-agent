import { spawn } from "node:child_process";
import { EventEmitter } from "node:events";
import { createInterface } from "node:readline";

function jsonRpcErrorToMessage(error) {
  if (!error || typeof error !== "object") return "unknown JSON-RPC error";

  const code = "code" in error ? error.code : undefined;
  const message = "message" in error ? error.message : undefined;
  if (typeof code === "number" && typeof message === "string") {
    return `${message} (code=${code})`;
  }
  if (typeof message === "string") return message;

  try {
    return JSON.stringify(error);
  } catch {
    return "unknown JSON-RPC error";
  }
}

export class JsonRpcStdioClient extends EventEmitter {
  #child;
  #pending = new Map();
  #nextId = 1;
  #closed = false;

  constructor(child) {
    super();
    this.#child = child;

    const rl = createInterface({ input: child.stdout, crlfDelay: Infinity });
    rl.on("line", (line) => this.#onLine(line));

    child.on("exit", (code, signal) => {
      this.#closed = true;
      const err =
        signal != null
          ? new Error(`pm-app-server exited via signal: ${signal}`)
          : new Error(`pm-app-server exited with code: ${code ?? "unknown"}`);
      for (const { reject } of this.#pending.values()) reject(err);
      this.#pending.clear();
      this.emit("exit", { code, signal });
    });

    child.on("error", (err) => {
      this.emit("error", err);
    });
  }

  static spawnPmAppServer({
    bin = process.env.CODE_PM_APP_SERVER_BIN || "pm-app-server",
    args = [],
    env = {},
  } = {}) {
    const child = spawn(bin, args, {
      stdio: ["pipe", "pipe", "inherit"],
      env: { ...process.env, ...env },
    });
    return new JsonRpcStdioClient(child);
  }

  call(method, params) {
    if (this.#closed) return Promise.reject(new Error("client is closed"));

    const id = this.#nextId++;
    const request = {
      jsonrpc: "2.0",
      id,
      method,
      params: params ?? null,
    };

    return new Promise((resolve, reject) => {
      this.#pending.set(id, { resolve, reject, method });
      this.#child.stdin.write(`${JSON.stringify(request)}\n`, (err) => {
        if (!err) return;
        this.#pending.delete(id);
        reject(err);
      });
    });
  }

  async close() {
    this.#closed = true;
    if (!this.#child.killed) {
      try {
        this.#child.kill("SIGTERM");
      } catch {
        // ignore
      }
    }
  }

  #onLine(line) {
    const trimmed = line.trim();
    if (trimmed === "") return;

    let msg;
    try {
      msg = JSON.parse(trimmed);
    } catch (err) {
      this.emit("invalid_json", { line: trimmed, error: err });
      return;
    }

    if (msg && typeof msg === "object" && "id" in msg) {
      const pending = this.#pending.get(msg.id);
      if (!pending) return;
      this.#pending.delete(msg.id);

      if ("error" in msg) {
        const err = new Error(jsonRpcErrorToMessage(msg.error));
        err.data = msg.error;
        pending.reject(err);
        return;
      }

      pending.resolve(msg.result);
      return;
    }

    if (msg && typeof msg === "object" && "method" in msg) {
      this.emit("notification", {
        method: msg.method,
        params: msg.params ?? null,
      });
      return;
    }

    this.emit("unknown_message", msg);
  }
}

