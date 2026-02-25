import { mkdtemp } from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import { test } from "node:test";
import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { fileURLToPath } from "node:url";

import { JsonRpcStdioClient, ThreadSubscribeStream } from "../src/index.js";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, "../../..");

function buildSpawnOptions(omneRoot) {
  const envBin = process.env.OMNE_APP_SERVER_BIN;
  if (envBin && envBin.trim() !== "") {
    return { bin: envBin, args: ["--omne-root", omneRoot], cwd: repoRoot };
  }
  return {
    bin: "cargo",
    args: ["run", "-q", "-p", "omne-app-server", "--", "--omne-root", omneRoot],
    cwd: repoRoot,
  };
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, Math.max(0, ms)));
}

async function waitUntil(predicate, timeoutMs, stepMs = 20) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() <= deadline) {
    if (predicate()) return;
    await sleep(stepMs);
  }
  throw new Error("timed out waiting for condition");
}

async function createInitializedClient(spawnOptions) {
  const client = JsonRpcStdioClient.spawnPmAppServer(spawnOptions);
  await client.call("initialize", {});
  await client.call("initialized", {});
  return client;
}

test("ThreadSubscribeStream reconnects and resumes from since_seq", async () => {
  const omneRoot = await mkdtemp(path.join(os.tmpdir(), "omne-node-test-"));
  const spawnOptions = buildSpawnOptions(omneRoot);
  const setupClient = await createInitializedClient(spawnOptions);

  const seenSeq = [];
  const reconnectReasons = [];
  let subscribeStream;
  try {
    const { thread_id: threadId } = await setupClient.call("thread/start", { cwd: repoRoot });
    subscribeStream = new ThreadSubscribeStream({
      threadId,
      sinceSeq: 0,
      waitMs: 250,
      spawnPmAppServer: spawnOptions,
      reconnectInitialDelayMs: 50,
      reconnectMaxDelayMs: 200,
      maxReconnectAttempts: 20,
    });

    subscribeStream.on("event", (event) => {
      if (typeof event?.seq === "number") seenSeq.push(event.seq);
    });
    subscribeStream.on("reconnect_scheduled", (event) => {
      reconnectReasons.push(event?.reason ?? "retry");
    });

    await subscribeStream.start();

    await setupClient.call("thread/pause", { thread_id: threadId });
    await setupClient.call("thread/unpause", { thread_id: threadId });
    await waitUntil(() => seenSeq.length >= 3, 4_000);

    const beforeReconnectSeq = subscribeStream.sinceSeq;
    await subscribeStream.forceReconnect("test");

    await setupClient.call("thread/pause", { thread_id: threadId });
    await setupClient.call("thread/unpause", { thread_id: threadId });
    await waitUntil(() => subscribeStream.sinceSeq > beforeReconnectSeq, 4_000);

    assert.ok(seenSeq.length >= 5, `expected >=5 events, got ${seenSeq.length}`);
    const unique = new Set(seenSeq);
    assert.equal(unique.size, seenSeq.length, "event seq should not duplicate across reconnect");
    for (let i = 1; i < seenSeq.length; i += 1) {
      assert.ok(seenSeq[i] > seenSeq[i - 1], "event seq should be strictly increasing");
    }
    assert.equal(subscribeStream.sinceSeq, seenSeq[seenSeq.length - 1]);
    assert.ok(reconnectReasons.length >= 1, "expected reconnect scheduling to happen");
  } finally {
    if (subscribeStream) {
      await subscribeStream.stop().catch(() => {});
    }
    await setupClient.close();
  }
});

test("ThreadSubscribeStream emits fatal for non-existent thread", async () => {
  const omneRoot = await mkdtemp(path.join(os.tmpdir(), "omne-node-test-"));
  const spawnOptions = buildSpawnOptions(omneRoot);
  const stream = new ThreadSubscribeStream({
    threadId: randomUUID(),
    sinceSeq: 0,
    waitMs: 100,
    spawnPmAppServer: spawnOptions,
    reconnectInitialDelayMs: 20,
    reconnectMaxDelayMs: 50,
    maxReconnectAttempts: 2,
  });

  let fatalPayload = null;
  stream.on("fatal", (payload) => {
    fatalPayload = payload;
  });

  try {
    await stream.start();
    await waitUntil(() => fatalPayload != null, 4_000);
    assert.equal(fatalPayload.reason, "thread_not_found");
  } finally {
    await stream.stop().catch(() => {});
  }
});
