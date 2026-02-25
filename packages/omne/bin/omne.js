#!/usr/bin/env node
"use strict";

const { spawn } = require("node:child_process");
const path = require("node:path");
const {
  buildSpawnEnv,
  resolveInvocation,
} = require("../lib/launcher.js");

function usageError(message) {
  process.stderr.write(`${message}\n`);
}

function spawnAndExit(bin, args, logicalBinName) {
  const pkgRoot = path.resolve(__dirname, "..");
  const childEnv = buildSpawnEnv(bin, {
    baseEnv: process.env,
    pkgRoot,
    env: process.env,
  });
  const child = spawn(bin, args, { stdio: "inherit", env: childEnv });

  const forwardSignal = (signal) => {
    if (child.killed) return;
    try {
      child.kill(signal);
    } catch {
      // ignore
    }
  };

  process.on("SIGINT", () => forwardSignal("SIGINT"));
  process.on("SIGTERM", () => forwardSignal("SIGTERM"));
  process.on("SIGHUP", () => forwardSignal("SIGHUP"));

  child.on("error", (err) => {
    if (err && err.code === "ENOENT") {
      usageError(`cannot find executable: ${bin}`);
      usageError("");
      usageError("Fix options:");
      usageError(`  - Install '${bin}' on PATH`);
      if (logicalBinName === "omne-app-server") {
        usageError(
          "  - Or set OMNE_APP_SERVER_BIN to an absolute path (e.g. target/debug/omne-app-server)"
        );
      } else if (logicalBinName === "omne") {
        usageError("  - Or set OMNE_PM_BIN to an absolute path (e.g. target/debug/omne)");
      }
      process.exit(1);
    }

    usageError(String(err));
    process.exit(1);
  });

  child.on("exit", (code, signal) => {
    if (signal) {
      process.exit(1);
    }
    process.exit(code ?? 1);
  });
}

function main() {
  const resolved = resolveInvocation(process.argv.slice(2), { env: process.env });
  spawnAndExit(resolved.bin, resolved.args, resolved.logicalBinName);
}

main();
