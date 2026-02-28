#!/usr/bin/env node
import fs from "node:fs/promises";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);
const {
  detectTargetTriple,
  resolveManagedToolchainDir,
  targetBinaryExt,
} = require("../lib/launcher.js");

function logStatus(tool, status, detail = "") {
  const suffix = detail ? ` (${detail})` : "";
  process.stdout.write(`[omne postinstall] ${tool}: ${status}${suffix}\n`);
}

function commandAvailable(command, env) {
  const result = spawnSync(command, ["--version"], {
    env,
    stdio: "ignore",
  });
  if (result.error) {
    if (result.error.code === "ENOENT") return false;
    return false;
  }
  return result.status === 0;
}

async function readFeatureSet(vendorTargetRoot) {
  const featuresPath = path.join(vendorTargetRoot, "features.json");
  try {
    const text = await fs.readFile(featuresPath, "utf8");
    const parsed = JSON.parse(text);
    if (!parsed || !Array.isArray(parsed.features)) return new Set();
    return new Set(
      parsed.features.map((item) => String(item || "").trim()).filter(Boolean)
    );
  } catch {
    return new Set();
  }
}

async function installBundledTool({
  tool,
  featureName,
  featureSet,
  targetTriple,
  vendorTargetRoot,
  managedToolchainDir,
}) {
  if (!featureSet.has(featureName)) {
    logStatus(tool, "missing_without_feature", `${featureName} not bundled`);
    return {
      tool,
      status: "missing_without_feature",
    };
  }

  const ext = targetBinaryExt(targetTriple);
  const sourcePath = path.join(vendorTargetRoot, "path", `${tool}${ext}`);
  try {
    await fs.access(sourcePath);
  } catch {
    logStatus(tool, "feature_mismatch_missing_binary", path.relative(vendorTargetRoot, sourcePath));
    return {
      tool,
      status: "feature_mismatch_missing_binary",
    };
  }

  const destinationPath = path.join(managedToolchainDir, `${tool}${ext}`);
  try {
    await fs.mkdir(managedToolchainDir, { recursive: true });
    await fs.copyFile(sourcePath, destinationPath);
    await fs.chmod(destinationPath, 0o755).catch(() => {});
    logStatus(tool, "installed_bundled", destinationPath);
    return {
      tool,
      status: "installed_bundled",
    };
  } catch (err) {
    logStatus(tool, "install_failed", String(err?.message || err));
    return {
      tool,
      status: "install_failed",
    };
  }
}

async function main() {
  const env = process.env;
  const packageRoot = env.OMNE_PACKAGE_ROOT
    ? path.resolve(env.OMNE_PACKAGE_ROOT)
    : path.resolve(__dirname, "..");
  const targetTriple = detectTargetTriple({ env });
  if (!targetTriple) {
    logStatus("toolchain", "skipped", "unsupported target");
    return;
  }

  const vendorTargetRoot = path.join(packageRoot, "vendor", targetTriple);
  const featureSet = await readFeatureSet(vendorTargetRoot);
  const managedToolchainDir = resolveManagedToolchainDir({
    env,
    targetTriple,
  });
  if (!managedToolchainDir) {
    logStatus("toolchain", "skipped", "cannot resolve managed toolchain dir");
    return;
  }

  const checks = [
    { tool: "git", featureName: "git-cli" },
    { tool: "gh", featureName: "gh-cli" },
  ];
  const results = [];
  for (const check of checks) {
    if (commandAvailable(check.tool, env)) {
      logStatus(check.tool, "present");
      results.push({ tool: check.tool, status: "present" });
      continue;
    }
    const result = await installBundledTool({
      ...check,
      featureSet,
      targetTriple,
      vendorTargetRoot,
      managedToolchainDir,
    });
    results.push(result);
  }

  if (env.OMNE_TOOLCHAIN_BOOTSTRAP_STRICT === "1") {
    const hasFailure = results.some((item) =>
      ["install_failed", "feature_mismatch_missing_binary"].includes(item.status)
    );
    if (hasFailure) {
      process.exitCode = 1;
    }
  }
}

main().catch((err) => {
  process.stderr.write(`[omne postinstall] fatal: ${String(err?.message || err)}\n`);
  if (process.env.OMNE_TOOLCHAIN_BOOTSTRAP_STRICT === "1") {
    process.exit(1);
  }
});
