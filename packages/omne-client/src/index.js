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

function clampU64(value, fallback) {
  if (typeof value === "number" && Number.isFinite(value) && value >= 0) {
    return Math.floor(value);
  }
  if (typeof value === "bigint" && value >= 0n && value <= BigInt(Number.MAX_SAFE_INTEGER)) {
    return Number(value);
  }
  if (typeof value === "string" && value.trim() !== "") {
    const parsed = Number.parseInt(value, 10);
    if (Number.isFinite(parsed) && parsed >= 0) return parsed;
  }
  return fallback;
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, Math.max(0, ms)));
}

function isTransportError(err) {
  const message = String(err?.message ?? err ?? "");
  return (
    message.includes("client is closed") ||
    message.includes("exited with code") ||
    message.includes("exited via signal") ||
    message.includes("EPIPE") ||
    message.includes("ECONNRESET") ||
    message.includes("write after end")
  );
}

function isThreadNotFoundError(err) {
  const message = String(err?.message ?? err ?? "");
  return message.includes("thread not found");
}

export class JsonRpcStdioClient extends EventEmitter {
  #child;
  #pending = new Map();
  #nextId = 1;
  #closed = false;
  #exitPromise;

  constructor(child) {
    super();
    this.#child = child;
    this.#exitPromise = new Promise((resolve) => {
      const rl = createInterface({ input: child.stdout, crlfDelay: Infinity });
      rl.on("line", (line) => this.#onLine(line));

      child.on("exit", (code, signal) => {
        this.#closed = true;
        const err =
          signal != null
            ? new Error(`omne-app-server exited via signal: ${signal}`)
            : new Error(`omne-app-server exited with code: ${code ?? "unknown"}`);
        for (const { reject } of this.#pending.values()) reject(err);
        this.#pending.clear();
        this.emit("exit", { code, signal });
        resolve({ code, signal });
      });
    });

    child.on("error", (err) => {
      this.emit("error", err);
    });
  }

  static spawnPmAppServer({
    bin = process.env.OMNE_APP_SERVER_BIN || "omne-app-server",
    args = [],
    env = {},
    cwd = process.cwd(),
  } = {}) {
    const child = spawn(bin, args, {
      stdio: ["pipe", "pipe", "inherit"],
      env: { ...process.env, ...env },
      cwd,
    });
    return new JsonRpcStdioClient(child);
  }

  get closed() {
    return this.#closed;
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

  async waitForExit(timeoutMs = 0) {
    if (timeoutMs <= 0) return this.#exitPromise;
    return Promise.race([
      this.#exitPromise,
      new Promise((_, reject) => {
        setTimeout(() => reject(new Error("waitForExit timeout")), timeoutMs);
      }),
    ]);
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
    try {
      await this.waitForExit(2_000);
    } catch {
      // best-effort close
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

export class ThreadSubscribeStream extends EventEmitter {
  #threadId;
  #sinceSeq;
  #waitMs;
  #maxEvents;
  #spawnOptions;
  #running = false;
  #stopping = false;
  #client = null;
  #loopPromise = null;
  #drainFast = false;
  #consecutiveFailures = 0;
  #reconnectInitialDelayMs;
  #reconnectMaxDelayMs;
  #reconnectMultiplier;
  #maxReconnectAttempts;

  constructor({
    threadId,
    sinceSeq = 0,
    waitMs = 30_000,
    maxEvents = undefined,
    spawnPmAppServer = {},
    reconnectInitialDelayMs = 250,
    reconnectMaxDelayMs = 5_000,
    reconnectMultiplier = 2,
    maxReconnectAttempts = Infinity,
  }) {
    super();
    if (typeof threadId !== "string" || threadId.trim() === "") {
      throw new Error("threadId is required");
    }

    this.#threadId = threadId;
    this.#sinceSeq = clampU64(sinceSeq, 0);
    this.#waitMs = clampU64(waitMs, 30_000);
    this.#maxEvents = maxEvents == null ? undefined : clampU64(maxEvents, 1);
    this.#spawnOptions = spawnPmAppServer;
    this.#reconnectInitialDelayMs = clampU64(reconnectInitialDelayMs, 250);
    this.#reconnectMaxDelayMs = clampU64(reconnectMaxDelayMs, 5_000);
    this.#reconnectMultiplier = Math.max(1, Number(reconnectMultiplier) || 2);
    this.#maxReconnectAttempts =
      maxReconnectAttempts === Infinity
        ? Infinity
        : Math.max(0, clampU64(maxReconnectAttempts, 0));
  }

  get sinceSeq() {
    return this.#sinceSeq;
  }

  async start() {
    if (this.#running) return this;
    this.#running = true;
    this.#stopping = false;
    this.#loopPromise = this.#runLoop();
    return this;
  }

  async stop() {
    this.#stopping = true;
    await this.#dropClient();
    if (this.#loopPromise) {
      try {
        await this.#loopPromise;
      } catch {
        // stop is best-effort
      }
    }
  }

  async forceReconnect(reason = "manual") {
    this.emit("reconnect_scheduled", {
      attempt: 0,
      delay_ms: 0,
      reason,
      forced: true,
    });
    await this.#dropClient();
  }

  async #connect() {
    const client = JsonRpcStdioClient.spawnPmAppServer(this.#spawnOptions);
    await client.call("initialize", {});
    await client.call("initialized", {});
    this.#client = client;
    this.#consecutiveFailures = 0;
    this.emit("connected", { since_seq: this.#sinceSeq });
  }

  async #dropClient() {
    const client = this.#client;
    this.#client = null;
    if (!client) return;
    try {
      await client.close();
    } catch {
      // ignore
    }
  }

  #subscribeParams() {
    return {
      thread_id: this.#threadId,
      since_seq: this.#sinceSeq,
      wait_ms: this.#drainFast ? 0 : this.#waitMs,
      max_events: this.#maxEvents,
    };
  }

  #nextBackoffMs() {
    const base = this.#reconnectInitialDelayMs;
    const factor = this.#reconnectMultiplier ** Math.max(0, this.#consecutiveFailures - 1);
    return Math.min(this.#reconnectMaxDelayMs, Math.floor(base * factor));
  }

  async #waitReconnectDelay(err) {
    if (this.#maxReconnectAttempts !== Infinity) {
      if (this.#consecutiveFailures > this.#maxReconnectAttempts) {
        this.emit("fatal", {
          reason: "max_reconnect_attempts_exceeded",
          error: err,
          attempts: this.#consecutiveFailures,
        });
        this.#stopping = true;
        return;
      }
    }

    const delay = this.#nextBackoffMs();
    this.emit("reconnect_scheduled", {
      attempt: this.#consecutiveFailures,
      delay_ms: delay,
      error: err,
      forced: false,
    });
    await sleep(delay);
  }

  async #runLoop() {
    while (!this.#stopping) {
      if (!this.#client) {
        try {
          await this.#connect();
        } catch (err) {
          this.#consecutiveFailures += 1;
          await this.#waitReconnectDelay(err);
          continue;
        }
      }

      try {
        const result = await this.#client.call("thread/subscribe", this.#subscribeParams());
        this.#consecutiveFailures = 0;

        const events = Array.isArray(result?.events) ? result.events : [];
        let seenLast = this.#sinceSeq;
        for (const event of events) {
          const seq = clampU64(event?.seq, seenLast);
          if (seq > seenLast) seenLast = seq;
          this.emit("event", event);
        }

        const lastSeq = clampU64(result?.last_seq, seenLast);
        if (lastSeq > this.#sinceSeq) {
          this.#sinceSeq = lastSeq;
        }

        this.#drainFast = Boolean(result?.has_more);
        this.emit("batch", {
          events,
          last_seq: clampU64(result?.last_seq, this.#sinceSeq),
          thread_last_seq: clampU64(result?.thread_last_seq, this.#sinceSeq),
          has_more: Boolean(result?.has_more),
          timed_out: Boolean(result?.timed_out),
          since_seq: this.#sinceSeq,
        });
      } catch (err) {
        if (this.#stopping) break;
        if (isThreadNotFoundError(err)) {
          this.emit("fatal", { reason: "thread_not_found", error: err });
          this.#stopping = true;
          break;
        }

        this.emit("disconnect", {
          error: err,
          retryable: isTransportError(err),
          since_seq: this.#sinceSeq,
        });
        await this.#dropClient();
        this.#consecutiveFailures += 1;
        await this.#waitReconnectDelay(err);
      }
    }

    this.#running = false;
    this.emit("stopped", { since_seq: this.#sinceSeq });
  }
}
