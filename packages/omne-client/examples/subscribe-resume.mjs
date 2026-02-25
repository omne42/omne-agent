import { mkdtemp } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { JsonRpcStdioClient, ThreadSubscribeStream } from "../src/index.js";

async function sleep(ms) {
  await new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitUntil(fn, timeoutMs, stepMs = 20) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() <= deadline) {
    if (fn()) return;
    await sleep(stepMs);
  }
  throw new Error("timed out waiting for condition");
}

async function main() {
  const pmRoot = await mkdtemp(path.join(os.tmpdir(), "omne-node-subscribe-"));
  const spawnArgs = ["--omne-root", pmRoot];

  const setupClient = JsonRpcStdioClient.spawnPmAppServer({ args: spawnArgs });
  await setupClient.call("initialize", {});
  await setupClient.call("initialized", {});
  const { thread_id } = await setupClient.call("thread/start", { cwd: process.cwd() });

  const stream = new ThreadSubscribeStream({
    threadId: thread_id,
    sinceSeq: 0,
    waitMs: 250,
    spawnPmAppServer: { args: spawnArgs },
    reconnectInitialDelayMs: 50,
    reconnectMaxDelayMs: 200,
    maxReconnectAttempts: 20,
  });

  const seenSeq = [];
  let reconnects = 0;
  stream.on("event", (event) => {
    if (typeof event?.seq === "number") {
      seenSeq.push(event.seq);
    }
  });
  stream.on("reconnect_scheduled", () => {
    reconnects += 1;
  });

  await stream.start();

  await setupClient.call("thread/pause", { thread_id });
  await setupClient.call("thread/unpause", { thread_id });
  await waitUntil(() => seenSeq.length >= 3, 3_000);

  // Force one reconnect, then continue consuming new events from the stored since_seq.
  await stream.forceReconnect("demo");
  await setupClient.call("thread/pause", { thread_id });
  await setupClient.call("thread/unpause", { thread_id });
  await waitUntil(() => seenSeq.length >= 5, 3_000);

  console.log(
    JSON.stringify(
      {
        thread_id,
        seen_events: seenSeq.length,
        first_seq: seenSeq[0] ?? null,
        last_seq: seenSeq[seenSeq.length - 1] ?? null,
        since_seq: stream.sinceSeq,
        reconnects,
      },
      null,
      2
    )
  );

  await stream.stop();
  await setupClient.close();
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
