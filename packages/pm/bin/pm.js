#!/usr/bin/env node
"use strict";

const { spawn } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

function detectTargetTriple() {
  const override = process.env.CODE_PM_TARGET_TRIPLE;
  if (override && override.trim() !== "") return override.trim();

  const { platform, arch } = process;
  if (platform === "darwin" && arch === "arm64") return "aarch64-apple-darwin";
  if (platform === "darwin" && arch === "x64") return "x86_64-apple-darwin";
  if (platform === "linux" && arch === "arm64") return "aarch64-unknown-linux-gnu";
  if (platform === "linux" && arch === "x64") return "x86_64-unknown-linux-gnu";
  if (platform === "win32" && arch === "arm64") return "aarch64-pc-windows-msvc";
  if (platform === "win32" && arch === "x64") return "x86_64-pc-windows-msvc";

  return null;
}

function resolveVendoredBinary(binName) {
  const targetTriple = detectTargetTriple();
  if (!targetTriple) return null;

  const pkgRoot = path.resolve(__dirname, "..");
  const ext = process.platform === "win32" ? ".exe" : "";
  const candidate = path.join(pkgRoot, "vendor", targetTriple, "pm", `${binName}${ext}`);

  try {
    fs.accessSync(candidate, fs.constants.F_OK);
    return candidate;
  } catch {
    return null;
  }
}

function resolveBinary(binName, envOverrideKey) {
  const override = process.env[envOverrideKey];
  if (override && override.trim() !== "") return override.trim();

  const vendored = resolveVendoredBinary(binName);
  if (vendored) return vendored;

  return binName;
}

function usageError(message) {
  process.stderr.write(`${message}\n`);
}

function spawnAndExit(bin, args) {
  const child = spawn(bin, args, { stdio: "inherit", env: process.env });

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
      if (bin === "pm-app-server") {
        usageError(
          "  - Or set CODE_PM_APP_SERVER_BIN to an absolute path (e.g. target/debug/pm-app-server)"
        );
      } else if (bin === "pm") {
        usageError("  - Or set CODE_PM_PM_BIN to an absolute path (e.g. target/debug/pm)");
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
  const argv = process.argv.slice(2);

  const isAppServer = argv[0] === "app-server";
  if (isAppServer) {
    const bin = resolveBinary("pm-app-server", "CODE_PM_APP_SERVER_BIN");
    spawnAndExit(bin, argv.slice(1));
    return;
  }

  const bin = resolveBinary("pm", "CODE_PM_PM_BIN");
  spawnAndExit(bin, argv);
}

main();

