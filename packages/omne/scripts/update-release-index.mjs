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
      "  node ./scripts/update-release-index.mjs [--release-out <dir>]\n"
  );
}

function parseArgs(argv) {
  const args = {
    releaseOut: path.join(packageRoot, "dist", "releases"),
  };
  for (let i = 0; i < argv.length; i += 1) {
    const key = argv[i];
    const val = argv[i + 1];
    if (key === "--release-out") {
      if (typeof val !== "string") throw new Error("missing value for --release-out");
      args.releaseOut = val;
      i += 1;
      continue;
    }
    throw new Error(`unknown argument: ${key}`);
  }
  return args;
}

function toPosixPath(filePath) {
  return String(filePath || "").replaceAll(path.sep, "/");
}

async function readJsonIfExists(filePath) {
  try {
    const text = await fs.readFile(filePath, "utf8");
    return JSON.parse(text);
  } catch {
    return null;
  }
}

export async function buildReleaseIndex(releaseOutRoot) {
  const releaseOut = path.resolve(releaseOutRoot);
  let entries = [];
  try {
    entries = await fs.readdir(releaseOut, { withFileTypes: true });
  } catch (err) {
    if (err && err.code === "ENOENT") {
      return {
        schema_version: 1,
        generated_at: new Date().toISOString(),
        release_out: releaseOut,
        releases: [],
      };
    }
    throw err;
  }

  const releases = [];
  for (const entry of entries) {
    if (!entry.isDirectory()) continue;
    if (!entry.name.startsWith("vendor-bundle-")) continue;
    const releaseRoot = path.join(releaseOut, entry.name);
    const releaseJson = await readJsonIfExists(path.join(releaseRoot, "RELEASE.json"));
    if (!releaseJson || typeof releaseJson !== "object") continue;
    const manifest = await readJsonIfExists(path.join(releaseRoot, "manifest.json"));
    releases.push({
      name: entry.name,
      version: String(releaseJson.version || "").trim(),
      target: String(releaseJson.target || "").trim(),
      created_at: String(releaseJson.created_at || "").trim(),
      release_dir: toPosixPath(path.relative(releaseOut, releaseRoot)),
      source_bundle_dir: String(releaseJson.source_bundle_dir || "").trim(),
      file_count: Array.isArray(manifest?.files) ? manifest.files.length : null,
    });
  }

  releases.sort((a, b) => {
    const ta = Date.parse(a.created_at || "");
    const tb = Date.parse(b.created_at || "");
    if (Number.isFinite(tb) && Number.isFinite(ta) && tb !== ta) return tb - ta;
    return b.name.localeCompare(a.name);
  });

  return {
    schema_version: 1,
    generated_at: new Date().toISOString(),
    release_out: releaseOut,
    releases,
  };
}

export async function writeReleaseIndex(releaseOutRoot) {
  const releaseOut = path.resolve(releaseOutRoot);
  await fs.mkdir(releaseOut, { recursive: true });
  const index = await buildReleaseIndex(releaseOut);
  const indexPath = path.join(releaseOut, "index.json");
  await fs.writeFile(indexPath, `${JSON.stringify(index, null, 2)}\n`, "utf8");
  return indexPath;
}

async function main() {
  let args;
  try {
    args = parseArgs(process.argv.slice(2));
  } catch (err) {
    usage(String(err?.message || err));
    process.exit(1);
  }
  const indexPath = await writeReleaseIndex(args.releaseOut);
  process.stdout.write(`release index updated: ${indexPath}\n`);
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((err) => {
    process.stderr.write(`${String(err?.message || err)}\n`);
    process.exit(1);
  });
}
