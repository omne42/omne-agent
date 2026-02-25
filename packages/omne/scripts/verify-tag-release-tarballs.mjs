#!/usr/bin/env node
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
      "  node ./scripts/verify-tag-release-tarballs.mjs " +
      "--out-dir <dir> --tag-version <vX.Y.Z>\n"
  );
}

function parseArgs(argv) {
  const args = {
    outDir: path.resolve(packageRoot, "dist", "release-bundles"),
    tagVersion: "",
  };
  for (let i = 0; i < argv.length; i += 1) {
    const key = argv[i];
    const val = argv[i + 1];
    if (key === "--out-dir") {
      if (typeof val !== "string") throw new Error("missing value for --out-dir");
      args.outDir = path.resolve(val);
      i += 1;
      continue;
    }
    if (key === "--tag-version") {
      if (typeof val !== "string") throw new Error("missing value for --tag-version");
      args.tagVersion = String(val).trim();
      i += 1;
      continue;
    }
    throw new Error(`unknown argument: ${key}`);
  }
  if (!args.tagVersion) throw new Error("--tag-version is required");
  return args;
}

function parseSha256Sums(text) {
  const entries = [];
  const lines = String(text || "")
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
  for (const line of lines) {
    const match = line.match(/^([a-fA-F0-9]{64})\s{2}(.+)$/);
    if (!match) throw new Error(`invalid SHA256SUMS line: ${line}`);
    entries.push({
      sum: match[1].toLowerCase(),
      file: String(match[2] || "").trim().replaceAll("\\", "/"),
    });
  }
  return entries;
}

function tarListEntries(tarPath) {
  const result = spawnSync("tar", ["-tzf", tarPath], { encoding: "utf8" });
  if (result.status !== 0) {
    throw new Error(`tar list failed for ${tarPath}: ${result.stderr || result.stdout || ""}`);
  }
  return String(result.stdout || "")
    .split(/\r?\n/)
    .map((item) => item.trim())
    .filter(Boolean)
    .map((item) => item.replace(/^\.\/+/, "").replace(/\/+$/, ""))
    .filter(Boolean)
    .filter((item) => item !== ".");
}

function tarReadEntryText(tarPath, relPath) {
  const candidates = [relPath, `./${relPath}`];
  for (const candidate of candidates) {
    const result = spawnSync("tar", ["-xOf", tarPath, candidate], { encoding: "utf8" });
    if (result.status === 0) {
      return String(result.stdout || "");
    }
  }
  throw new Error(`failed to read tar entry ${relPath} from ${tarPath}`);
}

function readJsonFromTar(tarPath, relPath, context) {
  const text = tarReadEntryText(tarPath, relPath);
  try {
    return JSON.parse(text);
  } catch (err) {
    throw new Error(`invalid JSON in ${context}: ${String(err?.message || err)}`);
  }
}

function isUnsafePath(relPath) {
  const normalized = String(relPath || "");
  if (!normalized || path.isAbsolute(normalized)) return true;
  const parts = normalized.split("/").filter(Boolean);
  return parts.some((part) => part === "." || part === "..");
}

function normalizeRelPath(relPath) {
  return String(relPath || "").replaceAll("\\", "/").trim();
}

function bundleTargetFromName(bundleName, tagVersion) {
  const prefix = `vendor-bundle-${tagVersion}-`;
  if (!bundleName.startsWith(prefix)) return "";
  return bundleName.slice(prefix.length).trim();
}

export async function verifyTagReleaseTarballs({ outDir, tagVersion }) {
  const dirEntries = await fs.readdir(outDir, { withFileTypes: true });
  const tarFiles = dirEntries
    .filter((entry) => entry.isFile() && entry.name.endsWith(".tar.gz"))
    .map((entry) => entry.name)
    .sort();
  if (tarFiles.length === 0) {
    throw new Error(`no *.tar.gz release bundles found in ${outDir}`);
  }

  const shaPath = path.join(outDir, "SHA256SUMS");
  const shaEntries = parseSha256Sums(await fs.readFile(shaPath, "utf8"));
  const sumsByFile = new Map();
  for (const entry of shaEntries) {
    if (sumsByFile.has(entry.file)) {
      throw new Error(`SHA256SUMS has duplicate entry for file: ${entry.file}`);
    }
    sumsByFile.set(entry.file, entry.sum);
  }
  const missingInSums = tarFiles.filter((file) => !sumsByFile.has(file));
  if (missingInSums.length > 0) {
    throw new Error(`SHA256SUMS missing entries for: ${missingInSums.join(", ")}`);
  }
  const unexpectedInSums = Array.from(sumsByFile.keys()).filter((file) => !tarFiles.includes(file));
  if (unexpectedInSums.length > 0) {
    throw new Error(`SHA256SUMS contains unknown files: ${unexpectedInSums.join(", ")}`);
  }

  const tarSummaries = [];
  for (const tarFile of tarFiles) {
    const tarPath = path.join(outDir, tarFile);
    const entries = tarListEntries(tarPath);
    const entrySet = new Set(entries);
    if (!entrySet.has("index.json")) {
      throw new Error(`${tarFile} is missing required entry: index.json`);
    }
    const indexJson = readJsonFromTar(tarPath, "index.json", `${tarFile}:index.json`);
    if (!indexJson || typeof indexJson !== "object" || !Array.isArray(indexJson.releases)) {
      throw new Error(`${tarFile} has invalid index.json shape`);
    }
    const indexReleases = indexJson.releases.filter(
      (item) => String(item?.version || "").trim() === tagVersion
    );
    if (indexReleases.length === 0) {
      throw new Error(`${tarFile} index.json does not contain version=${tagVersion}`);
    }
    const indexByTarget = new Map();
    for (const item of indexReleases) {
      const target = String(item?.target || "").trim();
      if (!target) {
        throw new Error(`${tarFile} index.json has empty target for version=${tagVersion}`);
      }
      if (indexByTarget.has(target)) {
        throw new Error(
          `${tarFile} index.json has duplicate entries for version=${tagVersion} target=${target}`
        );
      }
      indexByTarget.set(target, item);
    }

    const bundleDirs = new Set();
    for (const relPath of entries) {
      if (isUnsafePath(relPath)) {
        throw new Error(`${tarFile} contains unsafe entry path: ${relPath}`);
      }
      if (relPath === "index.json") continue;
      const topLevel = relPath.split("/")[0];
      if (!String(topLevel).startsWith(`vendor-bundle-${tagVersion}-`)) {
        throw new Error(`${tarFile} contains unexpected entry: ${relPath}`);
      }
      bundleDirs.add(topLevel);
    }
    if (bundleDirs.size === 0) {
      throw new Error(`${tarFile} has no vendor-bundle-${tagVersion}-* entries`);
    }
    const foundTargets = new Set();
    const targets = [];
    for (const bundleDir of Array.from(bundleDirs).sort()) {
      const target = bundleTargetFromName(bundleDir, tagVersion);
      if (!target) {
        throw new Error(`${tarFile} has invalid bundle directory name: ${bundleDir}`);
      }
      if (!entrySet.has(`${bundleDir}/RELEASE.json`)) {
        throw new Error(`${tarFile} is missing required entry: ${bundleDir}/RELEASE.json`);
      }
      if (!entrySet.has(`${bundleDir}/SHA256SUMS`)) {
        throw new Error(`${tarFile} is missing required entry: ${bundleDir}/SHA256SUMS`);
      }
      const releaseJson = readJsonFromTar(
        tarPath,
        `${bundleDir}/RELEASE.json`,
        `${tarFile}:${bundleDir}/RELEASE.json`
      );
      const releaseVersion = String(releaseJson?.version || "").trim();
      const releaseTarget = String(releaseJson?.target || "").trim();
      if (releaseVersion !== tagVersion) {
        throw new Error(
          `${tarFile} ${bundleDir}/RELEASE.json version mismatch: expected=${tagVersion} actual=${releaseVersion}`
        );
      }
      if (releaseTarget !== target) {
        throw new Error(
          `${tarFile} ${bundleDir}/RELEASE.json target mismatch: expected=${target} actual=${releaseTarget}`
        );
      }
      const indexEntry = indexByTarget.get(target);
      if (!indexEntry) {
        throw new Error(
          `${tarFile} contains bundle target=${target} missing in index.json for version=${tagVersion}`
        );
      }
      const releaseDir = normalizeRelPath(indexEntry.release_dir);
      if (releaseDir && path.basename(releaseDir) !== bundleDir) {
        throw new Error(
          `${tarFile} index.json release_dir mismatch for target=${target}: expected basename=${bundleDir} actual=${releaseDir}`
        );
      }
      foundTargets.add(target);
      targets.push(target);
    }
    const missingBundleTargets = Array.from(indexByTarget.keys())
      .filter((target) => !foundTargets.has(target))
      .sort();
    if (missingBundleTargets.length > 0) {
      throw new Error(
        `${tarFile} index.json contains version=${tagVersion} targets with no matching bundle directory: ${missingBundleTargets.join(", ")}`
      );
    }
    tarSummaries.push({
      tarball: tarFile,
      target_count: targets.length,
      targets,
    });
  }

  return {
    out_dir: outDir,
    tag_version: tagVersion,
    tarballs: tarSummaries,
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

  const summary = await verifyTagReleaseTarballs(args);
  process.stdout.write(`${JSON.stringify({ ok: true, summary }, null, 2)}\n`);
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((err) => {
    process.stderr.write(`${String(err?.message || err)}\n`);
    process.exit(1);
  });
}
