#!/usr/bin/env node
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const packageRoot = path.resolve(__dirname, "..");

function usage(message = "") {
  if (message) process.stderr.write(`${message}\n\n`);
  process.stderr.write(
    "Usage:\n" +
      "  node ./scripts/assemble-vendor.mjs " +
      "--target <triple> --omne <bin> --app-server <bin> [--path-dir <dir>] [--out <vendor-root>] [--clean]\n"
  );
}

function parseArgs(argv) {
  const args = {
    target: "",
    omne: "",
    appServer: "",
    pathDir: "",
    out: path.join(packageRoot, "vendor"),
    clean: false,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const key = argv[i];
    if (key === "--clean") {
      args.clean = true;
      continue;
    }
    const val = argv[i + 1];
    if (typeof val !== "string") {
      throw new Error(`missing value for ${key}`);
    }
    if (key === "--target") args.target = val;
    else if (key === "--omne") args.omne = val;
    else if (key === "--app-server") args.appServer = val;
    else if (key === "--path-dir") args.pathDir = val;
    else if (key === "--out") args.out = val;
    else throw new Error(`unknown argument: ${key}`);
    i += 1;
  }
  if (!args.target.trim()) throw new Error("--target is required");
  if (!args.omne.trim()) throw new Error("--omne is required");
  if (!args.appServer.trim()) throw new Error("--app-server is required");
  return args;
}

function binaryExtFromTarget(targetTriple) {
  return String(targetTriple || "").includes("windows") ? ".exe" : "";
}

async function copyExecutable(src, dst) {
  await fs.mkdir(path.dirname(dst), { recursive: true });
  await fs.copyFile(src, dst);
  await fs.chmod(dst, 0o755).catch(() => {});
}

async function main() {
  let args;
  try {
    args = parseArgs(process.argv.slice(2));
  } catch (err) {
    usage(String(err.message || err));
    process.exit(1);
  }

  const target = args.target.trim();
  const outRoot = path.resolve(args.out);
  const targetRoot = path.join(outRoot, target);
  const binaryRoot = path.join(targetRoot, "omne");
  const ext = binaryExtFromTarget(target);

  if (args.clean) {
    await fs.rm(targetRoot, { recursive: true, force: true });
  }
  await fs.mkdir(binaryRoot, { recursive: true });

  const omneSrc = path.resolve(args.omne);
  const appServerSrc = path.resolve(args.appServer);
  await copyExecutable(omneSrc, path.join(binaryRoot, `omne${ext}`));
  await copyExecutable(appServerSrc, path.join(binaryRoot, `omne-app-server${ext}`));

  if (args.pathDir && args.pathDir.trim()) {
    const pathSrc = path.resolve(args.pathDir);
    const pathDst = path.join(targetRoot, "path");
    await fs.rm(pathDst, { recursive: true, force: true });
    await fs.cp(pathSrc, pathDst, { recursive: true });
  }

  process.stdout.write(`assembled vendor target: ${targetRoot}\n`);
}

main().catch((err) => {
  process.stderr.write(`${String(err?.message || err)}\n`);
  process.exit(1);
});
