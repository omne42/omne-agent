#!/usr/bin/env node
import crypto from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const packageRoot = path.resolve(__dirname, "..");
const assembleScriptPath = path.join(__dirname, "assemble-vendor.mjs");

function usage(message = "") {
  if (message) process.stderr.write(`${message}\n\n`);
  process.stderr.write(
    "Usage:\n" +
      "  node ./scripts/build-vendor-bundle.mjs " +
      "--target <triple> --omne <bin> --app-server <bin> " +
      "[--path-dir <dir>] [--git-cli <bin>] [--gh-cli <bin>] " +
      "[--vendor-out <dir>] [--dist-out <dir>] [--clean]\n"
  );
}

function parseArgs(argv) {
  const args = {
    target: "",
    omne: "",
    appServer: "",
    pathDir: "",
    gitCli: "",
    ghCli: "",
    vendorOut: path.join(packageRoot, "vendor"),
    distOut: path.join(packageRoot, "dist"),
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
    else if (key === "--omne") args.omne = val;
    else if (key === "--app-server") args.appServer = val;
    else if (key === "--path-dir") args.pathDir = val;
    else if (key === "--git-cli") args.gitCli = val;
    else if (key === "--gh-cli") args.ghCli = val;
    else if (key === "--vendor-out") args.vendorOut = val;
    else if (key === "--dist-out") args.distOut = val;
    else throw new Error(`unknown argument: ${key}`);
    i += 1;
  }

  if (!args.target.trim()) throw new Error("--target is required");
  if (!args.omne.trim()) throw new Error("--omne is required");
  if (!args.appServer.trim()) throw new Error("--app-server is required");
  return args;
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

async function readFeatures(vendorTargetRoot) {
  const featuresPath = path.join(vendorTargetRoot, "features.json");
  try {
    const text = await fs.readFile(featuresPath, "utf8");
    const parsed = JSON.parse(text);
    if (!parsed || !Array.isArray(parsed.features)) return [];
    return parsed.features
      .map((item) => String(item || "").trim())
      .filter(Boolean)
      .sort();
  } catch {
    return [];
  }
}

async function buildManifest(bundleRoot, target, features) {
  const files = await walkFiles(bundleRoot);
  const entries = [];
  for (const abs of files) {
    const rel = toPosixPath(path.relative(bundleRoot, abs));
    const stat = await fs.stat(abs);
    entries.push({
      path: rel,
      size: stat.size,
      sha256: await sha256File(abs),
    });
  }
  return {
    schema_version: 1,
    target,
    features,
    generated_at: new Date().toISOString(),
    files: entries,
  };
}

function runAssembleVendor(args) {
  const cmdArgs = [
    assembleScriptPath,
    "--target",
    args.target,
    "--omne",
    args.omne,
    "--app-server",
    args.appServer,
    "--out",
    args.vendorOut,
  ];
  if (args.pathDir && String(args.pathDir).trim()) {
    cmdArgs.push("--path-dir", args.pathDir);
  }
  if (args.gitCli && String(args.gitCli).trim()) {
    cmdArgs.push("--git-cli", args.gitCli);
  }
  if (args.ghCli && String(args.ghCli).trim()) {
    cmdArgs.push("--gh-cli", args.ghCli);
  }
  if (args.clean) cmdArgs.push("--clean");

  const result = spawnSync(process.execPath, cmdArgs, { encoding: "utf8" });
  if (result.status !== 0) {
    throw new Error(
      `assemble-vendor failed (exit=${result.status}): ${result.stderr || result.stdout || ""}`
    );
  }
}

async function main() {
  let args;
  try {
    args = parseArgs(process.argv.slice(2));
  } catch (err) {
    usage(String(err?.message || err));
    process.exit(1);
  }

  const target = String(args.target || "").trim();
  const vendorOut = path.resolve(args.vendorOut);
  const distOut = path.resolve(args.distOut);
  const vendorTargetRoot = path.join(vendorOut, target);
  const bundleRoot = path.join(distOut, `vendor-bundle-${target}`);

  runAssembleVendor(args);
  await fs.access(vendorTargetRoot);

  if (args.clean) {
    await fs.rm(bundleRoot, { recursive: true, force: true });
  }
  await fs.mkdir(bundleRoot, { recursive: true });

  const vendorDst = path.join(bundleRoot, "vendor", target);
  await fs.rm(vendorDst, { recursive: true, force: true });
  await fs.mkdir(path.dirname(vendorDst), { recursive: true });
  await fs.cp(vendorTargetRoot, vendorDst, { recursive: true });

  const features = await readFeatures(vendorTargetRoot);
  const manifest = await buildManifest(bundleRoot, target, features);
  const manifestPath = path.join(bundleRoot, "manifest.json");
  await fs.writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
  process.stdout.write(`built vendor bundle: ${bundleRoot}\n`);
}

main().catch((err) => {
  process.stderr.write(`${String(err?.message || err)}\n`);
  process.exit(1);
});
