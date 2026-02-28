#!/usr/bin/env node
import crypto from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const packageRoot = path.resolve(__dirname, "..");

function usage(message = "") {
  if (message) process.stderr.write(`${message}\n\n`);
  process.stderr.write(
    "Usage:\n" +
      "  node ./scripts/verify-vendor-bundle.mjs [--bundle <bundle-dir>]\n"
  );
}

function parseArgs(argv) {
  const args = {
    bundle: "",
  };
  for (let i = 0; i < argv.length; i += 1) {
    const key = argv[i];
    const val = argv[i + 1];
    if (key === "--bundle") {
      if (typeof val !== "string") throw new Error("missing value for --bundle");
      args.bundle = val;
      i += 1;
      continue;
    }
    throw new Error(`unknown argument: ${key}`);
  }
  return args;
}

function targetBinaryExt(targetTriple) {
  return String(targetTriple || "").includes("windows") ? ".exe" : "";
}

async function sha256File(filePath) {
  const buf = await fs.readFile(filePath);
  return crypto.createHash("sha256").update(buf).digest("hex");
}

async function verifyBundle(bundleRoot) {
  const manifestPath = path.join(bundleRoot, "manifest.json");
  const text = await fs.readFile(manifestPath, "utf8");
  const manifest = JSON.parse(text);
  if (!manifest || typeof manifest !== "object") {
    throw new Error("invalid manifest: must be an object");
  }
  if (!Array.isArray(manifest.files)) {
    throw new Error("invalid manifest: files must be an array");
  }
  const features = Array.isArray(manifest.features)
    ? manifest.features.map((item) => String(item || "").trim()).filter(Boolean)
    : [];

  for (const entry of manifest.files) {
    const relPath = String(entry?.path || "").trim();
    const expectedSize = Number(entry?.size);
    const expectedSha = String(entry?.sha256 || "").trim().toLowerCase();
    if (!relPath) throw new Error("invalid manifest entry: missing path");
    if (!Number.isFinite(expectedSize) || expectedSize < 0) {
      throw new Error(`invalid manifest entry size: ${relPath}`);
    }
    if (!/^[0-9a-f]{64}$/.test(expectedSha)) {
      throw new Error(`invalid manifest entry sha256: ${relPath}`);
    }

    const absPath = path.resolve(bundleRoot, relPath);
    const stat = await fs.stat(absPath);
    if (stat.size !== expectedSize) {
      throw new Error(
        `size mismatch for ${relPath}: expected=${expectedSize} actual=${stat.size}`
      );
    }
    const actualSha = await sha256File(absPath);
    if (actualSha !== expectedSha) {
      throw new Error(
        `sha256 mismatch for ${relPath}: expected=${expectedSha} actual=${actualSha}`
      );
    }
  }

  const target = String(manifest.target || "").trim();
  const ext = targetBinaryExt(target);
  if (features.includes("git-cli")) {
    const gitPath = path.join(bundleRoot, "vendor", target, "path", `git${ext}`);
    try {
      await fs.access(gitPath);
    } catch {
      throw new Error(`feature mismatch: git-cli declared but missing ${path.relative(bundleRoot, gitPath)}`);
    }
  }
  if (features.includes("gh-cli")) {
    const ghPath = path.join(bundleRoot, "vendor", target, "path", `gh${ext}`);
    try {
      await fs.access(ghPath);
    } catch {
      throw new Error(`feature mismatch: gh-cli declared but missing ${path.relative(bundleRoot, ghPath)}`);
    }
  }
}

async function findBundleDir(defaultDistRoot) {
  const entries = await fs.readdir(defaultDistRoot, { withFileTypes: true });
  const candidates = entries
    .filter((entry) => entry.isDirectory() && entry.name.startsWith("vendor-bundle-"))
    .map((entry) => path.join(defaultDistRoot, entry.name))
    .sort();
  if (candidates.length === 0) {
    throw new Error(`no bundle found under ${defaultDistRoot}`);
  }
  return candidates[candidates.length - 1];
}

async function main() {
  let args;
  try {
    args = parseArgs(process.argv.slice(2));
  } catch (err) {
    usage(String(err?.message || err));
    process.exit(1);
  }

  const defaultDistRoot = path.join(packageRoot, "dist");
  const bundleRoot = args.bundle
    ? path.resolve(args.bundle)
    : await findBundleDir(defaultDistRoot);
  await verifyBundle(bundleRoot);
  process.stdout.write(`bundle verified: ${bundleRoot}\n`);
}

main().catch((err) => {
  process.stderr.write(`${String(err?.message || err)}\n`);
  process.exit(1);
});
