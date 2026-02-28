#!/usr/bin/env node
import path from "node:path";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const packageRoot = path.resolve(__dirname, "..");
const require = createRequire(import.meta.url);
const { buildSpawnEnv, resolveBinary } = require("../lib/launcher.js");

function logLine(message) {
  process.stdout.write(`[omne postinstall] ${message}\n`);
}

function runBootstrap() {
  const env = { ...process.env, OMNE_PACKAGE_ROOT: packageRoot };
  const omneBin = resolveBinary("omne", "OMNE_PM_BIN", { env, pkgRoot: packageRoot });
  const childEnv = buildSpawnEnv(omneBin, {
    baseEnv: env,
    env,
    pkgRoot: packageRoot,
  });
  const args = ["toolchain", "bootstrap"];
  if (process.env.OMNE_TOOLCHAIN_BOOTSTRAP_JSON === "1") {
    args.push("--json");
  }
  if (process.env.OMNE_TOOLCHAIN_BOOTSTRAP_STRICT === "1") {
    args.push("--strict");
  }

  const run = spawnSync(omneBin, args, {
    env: childEnv,
    encoding: "utf8",
  });
  if (run.error && run.error.code === "ENOENT") {
    logLine(`skip: cannot find '${omneBin}' (set OMNE_PM_BIN if needed)`);
    return 0;
  }
  if (run.stdout) process.stdout.write(run.stdout);
  if (run.stderr) process.stderr.write(run.stderr);
  if (run.status !== 0) {
    const strict = process.env.OMNE_TOOLCHAIN_BOOTSTRAP_STRICT === "1";
    if (strict) {
      logLine(`bootstrap failed (strict): exit=${run.status ?? 1}`);
      return run.status ?? 1;
    }
    logLine(`bootstrap warning: exit=${run.status ?? 1} (continue)`);
  }
  return 0;
}

const code = runBootstrap();
if (code !== 0) {
  process.exit(code);
}
