"use strict";

const fs = require("node:fs");
const path = require("node:path");

function detectTargetTriple(options = {}) {
  const env = options.env || process.env;
  const override =
    options.override !== undefined ? options.override : env.OMNE_TARGET_TRIPLE;
  if (override && String(override).trim() !== "") {
    return String(override).trim();
  }

  const platform = options.platform || process.platform;
  const arch = options.arch || process.arch;
  if (platform === "darwin" && arch === "arm64") return "aarch64-apple-darwin";
  if (platform === "darwin" && arch === "x64") return "x86_64-apple-darwin";
  if (platform === "linux" && arch === "arm64") return "aarch64-unknown-linux-gnu";
  if (platform === "linux" && arch === "x64") return "x86_64-unknown-linux-gnu";
  if (platform === "win32" && arch === "arm64") return "aarch64-pc-windows-msvc";
  if (platform === "win32" && arch === "x64") return "x86_64-pc-windows-msvc";
  return null;
}

function targetBinaryExt(targetTriple) {
  return String(targetTriple || "").includes("windows") ? ".exe" : "";
}

function resolveVendoredBinary(binName, options = {}) {
  const targetTriple =
    options.targetTriple || detectTargetTriple(options);
  if (!targetTriple) return null;
  const pkgRoot = options.pkgRoot || path.resolve(__dirname, "..");
  const candidate = path.join(
    pkgRoot,
    "vendor",
    targetTriple,
    "omne",
    `${binName}${targetBinaryExt(targetTriple)}`
  );
  const existsSync = options.existsSync || fs.existsSync;
  return existsSync(candidate) ? candidate : null;
}

function resolveBinary(binName, envOverrideKey, options = {}) {
  const env = options.env || process.env;
  const override = env[envOverrideKey];
  if (override && String(override).trim() !== "") {
    return String(override).trim();
  }
  return resolveVendoredBinary(binName, options) || binName;
}

function resolveInvocation(argv, options = {}) {
  const args = Array.isArray(argv) ? argv.slice() : [];
  if (args[0] === "app-server") {
    return {
      logicalBinName: "omne-app-server",
      bin: resolveBinary("omne-app-server", "OMNE_APP_SERVER_BIN", options),
      args: args.slice(1),
    };
  }
  return {
    logicalBinName: "omne",
    bin: resolveBinary("omne", "OMNE_PM_BIN", options),
    args,
  };
}

function resolvePathKey(env) {
  if (Object.prototype.hasOwnProperty.call(env, "PATH")) return "PATH";
  if (Object.prototype.hasOwnProperty.call(env, "Path")) return "Path";
  return "PATH";
}

function buildSpawnEnv(binPath, options = {}) {
  const env = Object.assign({}, options.baseEnv || process.env);
  const pkgRoot = options.pkgRoot || path.resolve(__dirname, "..");
  const targetTriple = options.targetTriple || detectTargetTriple(options);
  if (!binPath || !targetTriple) return env;

  const vendorBinDir = path.resolve(pkgRoot, "vendor", targetTriple, "omne");
  const resolvedBinPath = path.resolve(String(binPath));
  const underVendorBinDir =
    resolvedBinPath === vendorBinDir ||
    resolvedBinPath.startsWith(`${vendorBinDir}${path.sep}`);
  if (!underVendorBinDir) return env;

  const vendorPathDir = path.resolve(pkgRoot, "vendor", targetTriple, "path");
  const existsSync = options.existsSync || fs.existsSync;
  if (!existsSync(vendorPathDir)) return env;

  const pathKey = resolvePathKey(env);
  const currentPath = String(env[pathKey] || "");
  const entries = currentPath
    ? currentPath.split(path.delimiter).filter(Boolean)
    : [];
  const alreadyIncluded = entries.some(
    (entry) => path.resolve(entry) === vendorPathDir
  );
  if (alreadyIncluded) return env;
  env[pathKey] = currentPath
    ? `${vendorPathDir}${path.delimiter}${currentPath}`
    : vendorPathDir;
  return env;
}

module.exports = {
  buildSpawnEnv,
  detectTargetTriple,
  resolveBinary,
  resolveInvocation,
  resolveVendoredBinary,
  targetBinaryExt,
};
