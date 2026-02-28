#!/usr/bin/env node
import fs from "node:fs/promises";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { writeReleaseIndex } from "./update-release-index.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const packageRoot = path.resolve(__dirname, "..");
const releaseHostScriptPath = path.join(__dirname, "release-host-vendor-bundle.mjs");

const DEFAULT_TARGETS = [
  "x86_64-unknown-linux-gnu",
  "aarch64-unknown-linux-gnu",
  "x86_64-apple-darwin",
  "aarch64-apple-darwin",
  "x86_64-pc-windows-msvc",
  "aarch64-pc-windows-msvc",
];

function usage(message = "") {
  if (message) process.stderr.write(`${message}\n\n`);
  process.stderr.write(
    "Usage:\n" +
      "  node ./scripts/release-matrix-vendor-bundle.mjs " +
      "[--version <tag>] [--targets <t1,t2,...>] [--workspace-root <repo-root>] " +
      "[--target-dir <dir>] [--profile <debug|release>] [--path-dir <dir>] " +
      "[--git-cli <bin>] [--gh-cli <bin>] " +
      "[--vendor-out <dir>] [--dist-out <dir>] [--release-out <dir>] [--clean]\n"
  );
}

function parseTargets(rawTargets) {
  const text = String(rawTargets || "").trim();
  if (!text) return DEFAULT_TARGETS.slice();
  return text
    .split(",")
    .map((part) => part.trim())
    .filter(Boolean);
}

function parseArgs(argv) {
  const args = {
    version: "",
    targets: "",
    workspaceRoot: path.resolve(packageRoot, "..", ".."),
    targetDir: "",
    profile: "debug",
    pathDir: "",
    gitCli: "",
    ghCli: "",
    vendorOut: path.join(packageRoot, "vendor"),
    distOut: path.join(packageRoot, "dist"),
    releaseOut: path.join(packageRoot, "dist", "releases"),
    clean: false,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const key = argv[i];
    if (key === "--clean") {
      args.clean = true;
      continue;
    }
    const val = argv[i + 1];
    if (typeof val !== "string") throw new Error(`missing value for ${key}`);
    if (key === "--version") args.version = val;
    else if (key === "--targets") args.targets = val;
    else if (key === "--workspace-root") args.workspaceRoot = val;
    else if (key === "--target-dir") args.targetDir = val;
    else if (key === "--profile") args.profile = val;
    else if (key === "--path-dir") args.pathDir = val;
    else if (key === "--git-cli") args.gitCli = val;
    else if (key === "--gh-cli") args.ghCli = val;
    else if (key === "--vendor-out") args.vendorOut = val;
    else if (key === "--dist-out") args.distOut = val;
    else if (key === "--release-out") args.releaseOut = val;
    else throw new Error(`unknown argument: ${key}`);
    i += 1;
  }
  if (args.profile !== "debug" && args.profile !== "release") {
    throw new Error("--profile must be debug or release");
  }
  const targets = parseTargets(args.targets);
  if (targets.length === 0) throw new Error("no targets resolved");
  return { ...args, targets };
}

function buildTargetArgs(baseArgs, target, clean) {
  const args = [
    "--target",
    target,
    "--workspace-root",
    path.resolve(baseArgs.workspaceRoot),
    "--profile",
    baseArgs.profile,
    "--vendor-out",
    path.resolve(baseArgs.vendorOut),
    "--dist-out",
    path.resolve(baseArgs.distOut),
    "--release-out",
    path.resolve(baseArgs.releaseOut),
  ];
  if (baseArgs.version && String(baseArgs.version).trim()) {
    args.push("--version", String(baseArgs.version).trim());
  }
  if (baseArgs.targetDir && String(baseArgs.targetDir).trim()) {
    args.push("--target-dir", path.resolve(baseArgs.targetDir));
  }
  if (baseArgs.pathDir && String(baseArgs.pathDir).trim()) {
    args.push("--path-dir", path.resolve(baseArgs.pathDir));
  }
  if (baseArgs.gitCli && String(baseArgs.gitCli).trim()) {
    args.push("--git-cli", path.resolve(baseArgs.gitCli));
  }
  if (baseArgs.ghCli && String(baseArgs.ghCli).trim()) {
    args.push("--gh-cli", path.resolve(baseArgs.ghCli));
  }
  if (clean) args.push("--clean");
  return args;
}

function parseReleaseHostSummary(output) {
  const text = String(output || "");
  const match = text.match(/release-host: target=(\S+) version=(\S+) /);
  if (!match) return null;
  return { target: match[1], version: match[2] };
}

async function writeRunSummary(releaseOut, payload) {
  const outputPath = path.join(releaseOut, "last-run.json");
  await fs.writeFile(outputPath, `${JSON.stringify(payload, null, 2)}\n`, "utf8");
  return outputPath;
}

async function main() {
  let args;
  try {
    args = parseArgs(process.argv.slice(2));
  } catch (err) {
    usage(String(err?.message || err));
    process.exit(1);
  }

  const releaseOut = path.resolve(args.releaseOut);
  const results = [];
  for (let i = 0; i < args.targets.length; i += 1) {
    const target = args.targets[i];
    const targetArgs = buildTargetArgs(args, target, args.clean && i === 0);
    process.stdout.write(`release-matrix: target=${target}\n`);
    const run = spawnSync(
      process.execPath,
      [releaseHostScriptPath, ...targetArgs],
      { encoding: "utf8" }
    );
    process.stdout.write(run.stdout || "");
    process.stderr.write(run.stderr || "");
    if (run.status !== 0) {
      process.stderr.write(`release-matrix: failed target=${target}\n`);
      process.exit(run.status || 1);
    }
    const summary = parseReleaseHostSummary(run.stdout || "");
    if (!summary) {
      process.stderr.write(`release-matrix: cannot parse release-host output for ${target}\n`);
      process.exit(1);
    }
    results.push(summary);
  }

  const indexPath = await writeReleaseIndex(releaseOut);
  const runSummaryPath = await writeRunSummary(releaseOut, {
    schema_version: 1,
    created_at: new Date().toISOString(),
    targets: args.targets,
    results,
    index_path: indexPath,
  });
  process.stdout.write(`release-matrix: index=${indexPath}\n`);
  process.stdout.write(`release-matrix: run_summary=${runSummaryPath}\n`);
}

main().catch((err) => {
  process.stderr.write(`${String(err?.message || err)}\n`);
  process.exit(1);
});
