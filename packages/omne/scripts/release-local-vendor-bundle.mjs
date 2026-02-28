#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const packageRoot = path.resolve(__dirname, "..");
const releaseHostScriptPath = path.join(__dirname, "release-host-vendor-bundle.mjs");

function usage(message = "") {
  if (message) process.stderr.write(`${message}\n\n`);
  process.stderr.write(
    "Usage:\n" +
      "  node ./scripts/release-local-vendor-bundle.mjs " +
      "[--version <tag>] [--workspace-root <repo-root>] [--profile <debug|release>] " +
      "[--target <triple>] [--target-dir <dir>] [--path-dir <dir>] " +
      "[--git-cli <bin>] [--gh-cli <bin>] " +
      "[--vendor-out <dir>] [--dist-out <dir>] [--release-out <dir>] [--clean]\n"
  );
}

function parseArgs(argv) {
  const args = {
    version: "",
    workspaceRoot: path.resolve(packageRoot, "..", ".."),
    profile: "debug",
    target: "",
    targetDir: "",
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
    else if (key === "--workspace-root") args.workspaceRoot = val;
    else if (key === "--profile") args.profile = val;
    else if (key === "--target") args.target = val;
    else if (key === "--target-dir") args.targetDir = val;
    else if (key === "--path-dir") args.pathDir = val;
    else if (key === "--git-cli") args.gitCli = val;
    else if (key === "--gh-cli") args.ghCli = val;
    else if (key === "--vendor-out") args.vendorOut = val;
    else if (key === "--dist-out") args.distOut = val;
    else if (key === "--release-out") args.releaseOut = val;
    else throw new Error(`unknown argument: ${key}`);
    i += 1;
  }
  return args;
}

function buildReleaseHostArgs(args) {
  const cmd = [
    "--workspace-root",
    path.resolve(args.workspaceRoot),
    "--profile",
    args.profile,
    "--vendor-out",
    path.resolve(args.vendorOut),
    "--dist-out",
    path.resolve(args.distOut),
    "--release-out",
    path.resolve(args.releaseOut),
  ];
  if (args.version && String(args.version).trim()) {
    cmd.push("--version", String(args.version).trim());
  }
  if (args.target && String(args.target).trim()) {
    cmd.push("--target", String(args.target).trim());
  }
  if (args.targetDir && String(args.targetDir).trim()) {
    cmd.push("--target-dir", path.resolve(args.targetDir));
  }
  if (args.pathDir && String(args.pathDir).trim()) {
    cmd.push("--path-dir", path.resolve(args.pathDir));
  }
  if (args.gitCli && String(args.gitCli).trim()) {
    cmd.push("--git-cli", path.resolve(args.gitCli));
  }
  if (args.ghCli && String(args.ghCli).trim()) {
    cmd.push("--gh-cli", path.resolve(args.ghCli));
  }
  if (args.clean) cmd.push("--clean");
  return cmd;
}

function parseReleaseHostSummary(output) {
  const text = String(output || "");
  const match = text.match(/release-host: target=(\S+) version=(\S+) /);
  if (!match) return null;
  return {
    target: match[1],
    version: match[2],
  };
}

function main() {
  let args;
  try {
    args = parseArgs(process.argv.slice(2));
  } catch (err) {
    usage(String(err?.message || err));
    process.exit(1);
  }

  const releaseOut = path.resolve(args.releaseOut);
  const cmdArgs = buildReleaseHostArgs(args);
  const result = spawnSync(process.execPath, [releaseHostScriptPath, ...cmdArgs], {
    encoding: "utf8",
  });
  process.stdout.write(result.stdout || "");
  process.stderr.write(result.stderr || "");
  if (result.status !== 0) {
    process.exit(result.status || 1);
  }

  const summary = parseReleaseHostSummary(result.stdout || "");
  if (!summary) {
    process.stderr.write("release-local: failed to parse release-host summary\n");
    process.exit(1);
  }

  const releaseDir = path.join(
    releaseOut,
    `vendor-bundle-${summary.version}-${summary.target}`
  );
  const releaseJson = path.join(releaseDir, "RELEASE.json");
  const shaSums = path.join(releaseDir, "SHA256SUMS");
  const indexPath = path.join(releaseOut, "index.json");
  if (!fs.existsSync(releaseJson) || !fs.existsSync(shaSums)) {
    process.stderr.write(`release-local: release artifacts missing in ${releaseDir}\n`);
    process.exit(1);
  }
  if (!fs.existsSync(indexPath)) {
    process.stderr.write(`release-local: release index missing at ${indexPath}\n`);
    process.exit(1);
  }

  process.stdout.write(`release-local: release_dir=${releaseDir}\n`);
  process.stdout.write(`release-local: index=${indexPath}\n`);
}

main();
