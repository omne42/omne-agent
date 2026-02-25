#!/usr/bin/env node
import crypto from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath, pathToFileURL } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const packageRoot = path.resolve(__dirname, "..");

function usage(message = "") {
  if (message) process.stderr.write(`${message}\n\n`);
  process.stderr.write(
    "Usage:\n" +
      "  node ./scripts/package-tag-release-assets.mjs " +
      "--artifacts-root <dir> --tag-version <vX.Y.Z> " +
      "[--out-dir <dir>] [--payload-root <dir>]\n"
  );
}

function parseArgs(argv) {
  const args = {
    artifactsRoot: "",
    tagVersion: "",
    outDir: path.resolve(packageRoot, "dist", "release-bundles"),
    payloadRoot: path.resolve(packageRoot, "dist", "release-payloads"),
  };
  for (let i = 0; i < argv.length; i += 1) {
    const key = argv[i];
    const val = argv[i + 1];
    if (key === "--artifacts-root") {
      if (typeof val !== "string") throw new Error("missing value for --artifacts-root");
      args.artifactsRoot = val;
      i += 1;
      continue;
    }
    if (key === "--tag-version") {
      if (typeof val !== "string") throw new Error("missing value for --tag-version");
      args.tagVersion = val;
      i += 1;
      continue;
    }
    if (key === "--out-dir") {
      if (typeof val !== "string") throw new Error("missing value for --out-dir");
      args.outDir = val;
      i += 1;
      continue;
    }
    if (key === "--payload-root") {
      if (typeof val !== "string") throw new Error("missing value for --payload-root");
      args.payloadRoot = val;
      i += 1;
      continue;
    }
    throw new Error(`unknown argument: ${key}`);
  }
  if (!String(args.artifactsRoot || "").trim()) throw new Error("--artifacts-root is required");
  if (!String(args.tagVersion || "").trim()) throw new Error("--tag-version is required");
  return {
    artifactsRoot: path.resolve(args.artifactsRoot),
    tagVersion: String(args.tagVersion).trim(),
    outDir: path.resolve(args.outDir),
    payloadRoot: path.resolve(args.payloadRoot),
  };
}

function runTarCreate(sourceDir, outTarGzPath) {
  const result = spawnSync("tar", ["-C", sourceDir, "-czf", outTarGzPath, "."], {
    encoding: "utf8",
  });
  if (result.status !== 0) {
    throw new Error(`tar failed for ${sourceDir}: ${result.stderr || result.stdout || ""}`);
  }
}

async function sha256File(filePath) {
  const buf = await fs.readFile(filePath);
  return crypto.createHash("sha256").update(buf).digest("hex");
}

function toPosixPath(filePath) {
  return String(filePath || "").replaceAll(path.sep, "/");
}

async function readJson(filePath) {
  const text = await fs.readFile(filePath, "utf8");
  return JSON.parse(text);
}

function bundleTargetFromName(bundleName, tagVersion) {
  const prefix = `vendor-bundle-${tagVersion}-`;
  if (!bundleName.startsWith(prefix)) return "";
  return bundleName.slice(prefix.length).trim();
}

export async function packageTagReleaseAssets({
  artifactsRoot,
  tagVersion,
  outDir,
  payloadRoot,
}) {
  if (artifactsRoot === outDir || artifactsRoot === payloadRoot || outDir === payloadRoot) {
    throw new Error("artifactsRoot/outDir/payloadRoot must be distinct directories");
  }
  const entries = await fs.readdir(artifactsRoot, { withFileTypes: true });
  const artifactDirs = entries.filter(
    (entry) => entry.isDirectory() && entry.name.startsWith("omne-vendor-releases-")
  );
  if (artifactDirs.length === 0) {
    throw new Error(`no omne-vendor-releases-* directories found in ${artifactsRoot}`);
  }

  // Ensure output directories never carry stale files from previous runs.
  await fs.rm(outDir, { recursive: true, force: true });
  await fs.rm(payloadRoot, { recursive: true, force: true });
  await fs.mkdir(outDir, { recursive: true });
  await fs.mkdir(payloadRoot, { recursive: true });

  const createdTarballs = [];
  for (const artifactDir of artifactDirs) {
    const artifactRoot = path.join(artifactsRoot, artifactDir.name);
    const indexPath = path.join(artifactRoot, "index.json");
    const indexJson = await readJson(indexPath);
    if (!indexJson || typeof indexJson !== "object" || !Array.isArray(indexJson.releases)) {
      throw new Error(`invalid index.json shape in ${artifactRoot}`);
    }
    const releases = indexJson.releases.filter(
      (item) => String(item?.version || "").trim() === tagVersion
    );
    if (releases.length === 0) {
      throw new Error(`index.json in ${artifactRoot} does not contain version=${tagVersion}`);
    }
    const expectedBundleByTarget = new Map();
    for (const item of releases) {
      const target = String(item?.target || "").trim();
      if (!target) {
        throw new Error(`index.json in ${artifactRoot} has empty target for version=${tagVersion}`);
      }
      if (expectedBundleByTarget.has(target)) {
        throw new Error(
          `index.json in ${artifactRoot} has duplicate entries for version=${tagVersion} target=${target}`
        );
      }
      const expectedBundle = `vendor-bundle-${tagVersion}-${target}`;
      const releaseDirBase = path.basename(String(item?.release_dir || "").trim());
      if (releaseDirBase && releaseDirBase !== expectedBundle) {
        throw new Error(
          `index.json release_dir mismatch in ${artifactRoot} for target=${target}: expected=${expectedBundle} actual=${releaseDirBase}`
        );
      }
      expectedBundleByTarget.set(target, expectedBundle);
    }

    const bundleEntries = await fs.readdir(artifactRoot, { withFileTypes: true });
    const bundleTargets = new Map();
    for (const entry of bundleEntries) {
      if (!entry.isDirectory()) continue;
      if (!entry.name.startsWith(`vendor-bundle-${tagVersion}-`)) continue;
      const target = bundleTargetFromName(entry.name, tagVersion);
      if (!target) {
        throw new Error(`cannot infer target from bundle directory name: ${entry.name}`);
      }
      if (bundleTargets.has(target)) {
        throw new Error(
          `found duplicate bundle directories for version=${tagVersion} target=${target} in ${artifactRoot}`
        );
      }
      bundleTargets.set(target, entry.name);
    }
    if (bundleTargets.size === 0) {
      throw new Error(`no vendor-bundle-${tagVersion}-* directory found in ${artifactRoot}`);
    }
    const missingBundleTargets = Array.from(expectedBundleByTarget.keys())
      .filter((target) => !bundleTargets.has(target))
      .sort();
    if (missingBundleTargets.length > 0) {
      throw new Error(
        `index.json in ${artifactRoot} contains version=${tagVersion} targets with no matching bundle directory: ${missingBundleTargets.join(", ")}`
      );
    }
    const unexpectedBundleTargets = Array.from(bundleTargets.keys())
      .filter((target) => !expectedBundleByTarget.has(target))
      .sort();
    if (unexpectedBundleTargets.length > 0) {
      throw new Error(
        `bundle directories in ${artifactRoot} contain version=${tagVersion} targets missing in index.json: ${unexpectedBundleTargets.join(", ")}`
      );
    }

    const payloadDir = path.join(payloadRoot, artifactDir.name);
    await fs.rm(payloadDir, { recursive: true, force: true });
    await fs.mkdir(payloadDir, { recursive: true });
    await fs.cp(indexPath, path.join(payloadDir, "index.json"), { force: true });

    for (const target of Array.from(expectedBundleByTarget.keys()).sort()) {
      const bundleName = bundleTargets.get(target);
      const sourceBundle = path.join(artifactRoot, bundleName);
      const targetBundle = path.join(payloadDir, bundleName);
      await fs.rm(targetBundle, { recursive: true, force: true });
      await fs.cp(sourceBundle, targetBundle, { recursive: true });
    }

    const tarName = `${artifactDir.name}.tar.gz`;
    const tarPath = path.join(outDir, tarName);
    runTarCreate(payloadDir, tarPath);
    createdTarballs.push(tarPath);
  }

  createdTarballs.sort();
  const lines = [];
  for (const tarPath of createdTarballs) {
    const sum = await sha256File(tarPath);
    lines.push(`${sum}  ${toPosixPath(path.relative(outDir, tarPath))}`);
  }
  await fs.writeFile(path.join(outDir, "SHA256SUMS"), `${lines.join("\n")}\n`, "utf8");

  return {
    out_dir: outDir,
    payload_root: payloadRoot,
    tag_version: tagVersion,
    tarballs: createdTarballs.map((item) => toPosixPath(path.relative(outDir, item))),
    checksums_file: "SHA256SUMS",
  };
}

async function main() {
  let args;
  try {
    args = parseArgs(process.argv.slice(2));
  } catch (err) {
    usage(String(err?.message || err));
    process.exit(1);
  }
  const summary = await packageTagReleaseAssets(args);
  process.stdout.write(`${JSON.stringify({ ok: true, summary }, null, 2)}\n`);
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((err) => {
    process.stderr.write(`${String(err?.message || err)}\n`);
    process.exit(1);
  });
}
