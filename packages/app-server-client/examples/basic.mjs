import { mkdtemp } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { JsonRpcStdioClient } from "../src/index.js";

async function main() {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "omne-agent-node-"));
  const client = JsonRpcStdioClient.spawnOmneAgentAppServer({ args: ["--root", rootDir] });

  await client.call("initialize", {});
  await client.call("initialized", {});

  const { thread_id } = await client.call("thread/start", { cwd: process.cwd() });
  const sub = await client.call("thread/subscribe", {
    thread_id,
    since_seq: 0,
    wait_ms: 0,
  });

  // Keep output stable and keyless (no LLM call).
  console.log(
    JSON.stringify(
      {
        thread_id,
        events: Array.isArray(sub.events) ? sub.events.length : 0,
        last_seq: sub.last_seq,
        thread_last_seq: sub.thread_last_seq,
        timed_out: sub.timed_out,
      },
      null,
      2
    )
  );

  await client.close();
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
