#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const packageRoot = path.resolve(__dirname, "..");
const releaseScriptPath = path.join(__dirname, "release-vendor-bundle.mjs");

function usage(message = "") {
  if (message) process.stderr.write(`${message}\n\n`);
  process.stderr.write(
    "Usage:\n" +
      "  node ./scripts/release-host-vendor-bundle.mjs [--version <tag>] " +
      "[--workspace-root <repo-root>] [--profile <debug|release>] [--target <triple>] " +
      "[--target-dir <dir>] [--omne <bin>] [--app-server <bin>] [--path-dir <dir>] " +
      "[--git-cli <bin>] [--gh-cli <bin>] " +
      "[--vendor-out <dir>] [--dist-out <dir>] [--release-out <dir>] [--clean]\n"
  );
}

function detectTargetTriple(platform, arch) {
  if (platform === "darwin" && arch === "arm64") return "aarch64-apple-darwin";
  if (platform === "darwin" && arch === "x64") return "x86_64-apple-darwin";
  if (platform === "linux" && arch === "arm64") return "aarch64-unknown-linux-gnu";
  if (platform === "linux" && arch === "x64") return "x86_64-unknown-linux-gnu";
  if (platform === "win32" && arch === "arm64") return "aarch64-pc-windows-msvc";
  if (platform === "win32" && arch === "x64") return "x86_64-pc-windows-msvc";
  return "";
}

function binaryExt(targetTriple) {
  return String(targetTriple || "").includes("windows") ? ".exe" : "";
}

function parseArgs(argv) {
  const args = {
    version: "",
    workspaceRoot: path.resolve(packageRoot, "..", ".."),
    profile: "debug",
    target: "",
    targetDir: "",
    omne: "",
    appServer: "",
    pathDir: "",
    gitCli: "",
    ghCli: "",
    vendorOut: "",
    distOut: "",
    releaseOut: "",
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
    else if (key === "--omne") args.omne = val;
    else if (key === "--app-server") args.appServer = val;
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
  return args;
}

function firstExisting(paths) {
  for (const candidate of paths) {
    if (fs.existsSync(candidate)) return candidate;
  }
  return "";
}

function resolveBinaryFromWorkspace({
  workspaceRoot,
  targetDirOverride,
  target,
  profile,
  binName,
}) {
  const ext = binaryExt(target);
  const targetDir = targetDirOverride
    ? path.resolve(targetDirOverride)
    : path.join(workspaceRoot, "target");
  const candidates = [
    path.join(targetDir, profile, `${binName}${ext}`),
    path.join(targetDir, target, profile, `${binName}${ext}`),
  ];
  return firstExisting(candidates);
}

function utcTimestampCompact(date = new Date()) {
  const y = date.getUTCFullYear();
  const m = String(date.getUTCMonth() + 1).padStart(2, "0");
  const d = String(date.getUTCDate()).padStart(2, "0");
  const hh = String(date.getUTCHours()).padStart(2, "0");
  const mm = String(date.getUTCMinutes()).padStart(2, "0");
  const ss = String(date.getUTCSeconds()).padStart(2, "0");
  return `${y}${m}${d}${hh}${mm}${ss}`;
}

function readPackageVersion() {
  try {
    const packageJsonPath = path.join(packageRoot, "package.json");
    const parsed = JSON.parse(fs.readFileSync(packageJsonPath, "utf8"));
    const version = String(parsed?.version || "").trim();
    return version || "0.0.0";
  } catch {
    return "0.0.0";
  }
}

function resolveReleaseVersion(rawVersion) {
  const given = String(rawVersion || "").trim();
  if (given) return given;
  return `${readPackageVersion()}-dev.${utcTimestampCompact()}`;
}

function buildReleaseArgs(rawArgs) {
  const workspaceRoot = path.resolve(rawArgs.workspaceRoot);
  const target =
    String(rawArgs.target || "").trim() ||
    detectTargetTriple(process.platform, process.arch);
  if (!target) {
    throw new Error(
      `cannot detect target triple for platform=${process.platform} arch=${process.arch}; use --target`
    );
  }

  const omne =
    String(rawArgs.omne || "").trim() ||
    resolveBinaryFromWorkspace({
      workspaceRoot,
      targetDirOverride: rawArgs.targetDir,
      target,
      profile: rawArgs.profile,
      binName: "omne",
    });
  const appServer =
    String(rawArgs.appServer || "").trim() ||
    resolveBinaryFromWorkspace({
      workspaceRoot,
      targetDirOverride: rawArgs.targetDir,
      target,
      profile: rawArgs.profile,
      binName: "omne-app-server",
    });
  if (!omne) {
    throw new Error("cannot resolve omne binary; pass --omne or --target-dir/--workspace-root");
  }
  if (!appServer) {
    throw new Error(
      "cannot resolve omne-app-server binary; pass --app-server or --target-dir/--workspace-root"
    );
  }

  const version = resolveReleaseVersion(rawArgs.version);
  const args = [
    "--target",
    target,
    "--version",
    version,
    "--omne",
    path.resolve(omne),
    "--app-server",
    path.resolve(appServer),
  ];
  if (rawArgs.pathDir && String(rawArgs.pathDir).trim()) {
    args.push("--path-dir", path.resolve(rawArgs.pathDir));
  }
  if (rawArgs.gitCli && String(rawArgs.gitCli).trim()) {
    args.push("--git-cli", path.resolve(rawArgs.gitCli));
  }
  if (rawArgs.ghCli && String(rawArgs.ghCli).trim()) {
    args.push("--gh-cli", path.resolve(rawArgs.ghCli));
  }
  if (rawArgs.vendorOut && String(rawArgs.vendorOut).trim()) {
    args.push("--vendor-out", path.resolve(rawArgs.vendorOut));
  }
  if (rawArgs.distOut && String(rawArgs.distOut).trim()) {
    args.push("--dist-out", path.resolve(rawArgs.distOut));
  }
  if (rawArgs.releaseOut && String(rawArgs.releaseOut).trim()) {
    args.push("--release-out", path.resolve(rawArgs.releaseOut));
  }
  if (rawArgs.clean) args.push("--clean");
  return {
    args,
    target,
    version,
    omne: path.resolve(omne),
    appServer: path.resolve(appServer),
  };
}

function runRelease(args) {
  const result = spawnSync(process.execPath, [releaseScriptPath, ...args], {
    stdio: "inherit",
  });
  if (result.status !== 0) {
    process.exit(result.status || 1);
  }
}

function main() {
  let rawArgs;
  try {
    rawArgs = parseArgs(process.argv.slice(2));
  } catch (err) {
    usage(String(err?.message || err));
    process.exit(1);
  }

  let resolved;
  try {
    resolved = buildReleaseArgs(rawArgs);
  } catch (err) {
    process.stderr.write(`${String(err?.message || err)}\n`);
    process.exit(1);
  }

  process.stdout.write(
    `release-host: target=${resolved.target} version=${resolved.version} omne=${resolved.omne} app_server=${resolved.appServer}\n`
  );
  runRelease(resolved.args);
}

main();
