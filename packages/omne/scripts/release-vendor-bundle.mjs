#!/usr/bin/env node
import crypto from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { writeReleaseIndex } from "./update-release-index.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const packageRoot = path.resolve(__dirname, "..");
const buildScriptPath = path.join(__dirname, "build-vendor-bundle.mjs");
const verifyScriptPath = path.join(__dirname, "verify-vendor-bundle.mjs");

function usage(message = "") {
  if (message) process.stderr.write(`${message}\n\n`);
  process.stderr.write(
    "Usage:\n" +
      "  node ./scripts/release-vendor-bundle.mjs " +
      "--target <triple> --version <semver-or-tag> --omne <bin> --app-server <bin> " +
      "[--path-dir <dir>] [--vendor-out <dir>] [--dist-out <dir>] [--release-out <dir>] [--clean]\n"
  );
}

function parseArgs(argv) {
  const args = {
    target: "",
    version: "",
    omne: "",
    appServer: "",
    pathDir: "",
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
    if (key === "--target") args.target = val;
    else if (key === "--version") args.version = val;
    else if (key === "--omne") args.omne = val;
    else if (key === "--app-server") args.appServer = val;
    else if (key === "--path-dir") args.pathDir = val;
    else if (key === "--vendor-out") args.vendorOut = val;
    else if (key === "--dist-out") args.distOut = val;
    else if (key === "--release-out") args.releaseOut = val;
    else throw new Error(`unknown argument: ${key}`);
    i += 1;
  }
  if (!args.target.trim()) throw new Error("--target is required");
  if (!args.version.trim()) throw new Error("--version is required");
  if (!args.omne.trim()) throw new Error("--omne is required");
  if (!args.appServer.trim()) throw new Error("--app-server is required");
  return args;
}

function runNodeScript(scriptPath, scriptArgs) {
  const result = spawnSync(process.execPath, [scriptPath, ...scriptArgs], {
    encoding: "utf8",
  });
  if (result.status !== 0) {
    throw new Error(
      `${path.basename(scriptPath)} failed (exit=${result.status}): ${result.stderr || result.stdout || ""}`
    );
  }
}

function toPosixPath(filePath) {
  return String(filePath || "").replaceAll(path.sep, "/");
}

async function walkFiles(rootDir) {
  const files = [];
  async function walk(currentDir) {
    const entries = await fs.readdir(currentDir, { withFileTypes: true });
    for (const entry of entries) {
      const abs = path.join(currentDir, entry.name);
      if (entry.isDirectory()) {
        await walk(abs);
      } else if (entry.isFile()) {
        files.push(abs);
      }
    }
  }
  await walk(rootDir);
  files.sort();
  return files;
}

async function sha256File(filePath) {
  const buf = await fs.readFile(filePath);
  return crypto.createHash("sha256").update(buf).digest("hex");
}

async function buildSha256Sums(releaseRoot) {
  const files = await walkFiles(releaseRoot);
  const lines = [];
  for (const absPath of files) {
    const rel = toPosixPath(path.relative(releaseRoot, absPath));
    if (rel === "SHA256SUMS") continue;
    const sum = await sha256File(absPath);
    lines.push(`${sum}  ${rel}`);
  }
  lines.sort();
  return lines.join("\n");
}

async function main() {
  let args;
  try {
    args = parseArgs(process.argv.slice(2));
  } catch (err) {
    usage(String(err?.message || err));
    process.exit(1);
  }

  const target = args.target.trim();
  const version = args.version.trim();
  const vendorOut = path.resolve(args.vendorOut);
  const distOut = path.resolve(args.distOut);
  const releaseOut = path.resolve(args.releaseOut);
  const bundleRoot = path.join(distOut, `vendor-bundle-${target}`);
  const releaseRoot = path.join(releaseOut, `vendor-bundle-${version}-${target}`);

  const buildArgs = [
    "--target",
    target,
    "--omne",
    args.omne,
    "--app-server",
    args.appServer,
    "--vendor-out",
    vendorOut,
    "--dist-out",
    distOut,
  ];
  if (args.pathDir && args.pathDir.trim()) {
    buildArgs.push("--path-dir", args.pathDir);
  }
  if (args.clean) buildArgs.push("--clean");

  runNodeScript(buildScriptPath, buildArgs);
  runNodeScript(verifyScriptPath, ["--bundle", bundleRoot]);

  if (args.clean) {
    await fs.rm(releaseRoot, { recursive: true, force: true });
  }
  await fs.mkdir(path.dirname(releaseRoot), { recursive: true });
  await fs.rm(releaseRoot, { recursive: true, force: true });
  await fs.cp(bundleRoot, releaseRoot, { recursive: true });

  const shaLines = await buildSha256Sums(releaseRoot);
  await fs.writeFile(path.join(releaseRoot, "SHA256SUMS"), `${shaLines}\n`, "utf8");

  const releaseInfo = {
    schema_version: 1,
    version,
    target,
    created_at: new Date().toISOString(),
    source_bundle_dir: bundleRoot,
    release_dir: releaseRoot,
  };
  await fs.writeFile(
    path.join(releaseRoot, "RELEASE.json"),
    `${JSON.stringify(releaseInfo, null, 2)}\n`,
    "utf8"
  );
  const indexPath = await writeReleaseIndex(path.dirname(releaseRoot));
  process.stdout.write(`release index: ${indexPath}\n`);
  process.stdout.write(`released vendor bundle: ${releaseRoot}\n`);
}

main().catch((err) => {
  process.stderr.write(`${String(err?.message || err)}\n`);
  process.exit(1);
});
