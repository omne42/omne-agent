#!/usr/bin/env node
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const packageRoot = path.resolve(__dirname, "..");

function usage(message = "") {
  if (message) process.stderr.write(`${message}\n\n`);
  process.stderr.write(
    "Usage:\n" +
      "  node ./scripts/validate-tag-release-artifacts.mjs " +
      "--artifacts-root <dir> --tag-version <vX.Y.Z>\n"
  );
}

function parseArgs(argv) {
  const args = {
    artifactsRoot: path.join(packageRoot, "dist", "releases"),
    tagVersion: "",
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
    throw new Error(`unknown argument: ${key}`);
  }
  if (!String(args.tagVersion || "").trim()) {
    throw new Error("--tag-version is required");
  }
  return args;
}

async function readJson(filePath) {
  const text = await fs.readFile(filePath, "utf8");
  return JSON.parse(text);
}

function ensureReleaseShape(indexJson, context) {
  if (!indexJson || typeof indexJson !== "object" || !Array.isArray(indexJson.releases)) {
    throw new Error(`invalid index.json shape in ${context}`);
  }
}

function bundleTargetFromName(bundleName, tagVersion) {
  const prefix = `vendor-bundle-${tagVersion}-`;
  if (!bundleName.startsWith(prefix)) return "";
  return bundleName.slice(prefix.length).trim();
}

function normalizeRelPath(relPath) {
  return String(relPath || "").replaceAll("\\", "/").trim();
}

export async function validateTagReleaseArtifacts(artifactsRootInput, tagVersionInput) {
  const artifactsRoot = path.resolve(artifactsRootInput);
  const tagVersion = String(tagVersionInput || "").trim();
  if (!tagVersion) throw new Error("tagVersion is required");

  let entries = [];
  try {
    entries = await fs.readdir(artifactsRoot, { withFileTypes: true });
  } catch (err) {
    if (err && err.code === "ENOENT") {
      throw new Error(`artifacts root not found: ${artifactsRoot}`);
    }
    throw err;
  }

  const artifactDirs = entries.filter(
    (entry) => entry.isDirectory() && entry.name.startsWith("omne-vendor-releases-")
  );
  if (artifactDirs.length === 0) {
    throw new Error(`no omne-vendor-releases-* directories found in ${artifactsRoot}`);
  }

  const summary = [];
  for (const artifactDir of artifactDirs) {
    const artifactRoot = path.join(artifactsRoot, artifactDir.name);
    const indexPath = path.join(artifactRoot, "index.json");
    const indexJson = await readJson(indexPath);
    ensureReleaseShape(indexJson, artifactRoot);

    const releases = indexJson.releases.filter(
      (item) => String(item?.version || "").trim() === tagVersion
    );
    if (releases.length === 0) {
      throw new Error(`index.json in ${artifactRoot} does not contain version=${tagVersion}`);
    }
    const releasesByTarget = new Map();
    for (const item of releases) {
      const target = String(item?.target || "").trim();
      if (!target) {
        throw new Error(`index.json in ${artifactRoot} has empty target for version=${tagVersion}`);
      }
      const list = releasesByTarget.get(target) || [];
      list.push(item);
      releasesByTarget.set(target, list);
    }

    const dirEntries = await fs.readdir(artifactRoot, { withFileTypes: true });
    const bundleDirs = dirEntries.filter((entry) => {
      if (!entry.isDirectory()) return false;
      return entry.name.startsWith(`vendor-bundle-${tagVersion}-`);
    });
    if (bundleDirs.length === 0) {
      throw new Error(`no vendor-bundle-${tagVersion}-* directory found in ${artifactRoot}`);
    }

    const foundTargets = new Set();
    for (const bundleDir of bundleDirs) {
      const bundleName = bundleDir.name;
      const bundleRoot = path.join(artifactRoot, bundleName);
      const targetFromName = bundleTargetFromName(bundleName, tagVersion);
      if (!targetFromName) {
        throw new Error(`cannot infer target from bundle directory name: ${bundleName}`);
      }

      const releaseJsonPath = path.join(bundleRoot, "RELEASE.json");
      const shaSumsPath = path.join(bundleRoot, "SHA256SUMS");
      await fs.access(releaseJsonPath);
      await fs.access(shaSumsPath);

      const releaseJson = await readJson(releaseJsonPath);
      const version = String(releaseJson?.version || "").trim();
      const target = String(releaseJson?.target || "").trim();
      if (version !== tagVersion) {
        throw new Error(
          `RELEASE.json version mismatch in ${bundleRoot}: expected=${tagVersion} actual=${version}`
        );
      }
      if (target !== targetFromName) {
        throw new Error(
          `RELEASE.json target mismatch in ${bundleRoot}: expected=${targetFromName} actual=${target}`
        );
      }
      if (!releasesByTarget.has(target)) {
        throw new Error(
          `index.json in ${artifactRoot} has no release entry for version=${tagVersion} target=${target}`
        );
      }
      const matching = releasesByTarget.get(target) || [];
      if (matching.length !== 1) {
        throw new Error(
          `index.json in ${artifactRoot} has ${matching.length} entries for version=${tagVersion} target=${target}; expected exactly 1`
        );
      }
      const releaseEntry = matching[0] || {};
      const releaseDir = normalizeRelPath(releaseEntry.release_dir);
      if (releaseDir && path.basename(releaseDir) !== bundleName) {
        throw new Error(
          `index.json release_dir mismatch in ${artifactRoot} for target=${target}: expected basename=${bundleName} actual=${releaseDir}`
        );
      }
      foundTargets.add(target);
    }
    const missingBundleTargets = Array.from(releasesByTarget.keys())
      .filter((target) => !foundTargets.has(target))
      .sort();
    if (missingBundleTargets.length > 0) {
      throw new Error(
        `index.json in ${artifactRoot} contains version=${tagVersion} targets with no matching bundle directory: ${missingBundleTargets.join(", ")}`
      );
    }

    summary.push({
      artifact_dir: artifactDir.name,
      tag_version: tagVersion,
      targets: Array.from(foundTargets).sort(),
      bundle_count: bundleDirs.length,
    });
  }

  return summary;
}

async function main() {
  let args;
  try {
    args = parseArgs(process.argv.slice(2));
  } catch (err) {
    usage(String(err?.message || err));
    process.exit(1);
  }

  const summary = await validateTagReleaseArtifacts(args.artifactsRoot, args.tagVersion);
  process.stdout.write(`${JSON.stringify({ ok: true, summary }, null, 2)}\n`);
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((err) => {
    process.stderr.write(`${String(err?.message || err)}\n`);
    process.exit(1);
  });
}
