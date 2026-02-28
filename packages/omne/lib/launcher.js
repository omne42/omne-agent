"use strict";

const fs = require("node:fs");
const os = require("node:os");
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

function addPathEntry(env, entry, mode = "prepend") {
  const normalizedEntry = path.resolve(String(entry));
  const pathKey = resolvePathKey(env);
  const currentPath = String(env[pathKey] || "");
  const entries = currentPath
    ? currentPath.split(path.delimiter).filter(Boolean)
    : [];
  const alreadyIncluded = entries.some(
    (item) => path.resolve(item) === normalizedEntry
  );
  if (alreadyIncluded) return env;

  if (!currentPath) {
    env[pathKey] = normalizedEntry;
    return env;
  }
  env[pathKey] =
    mode === "append"
      ? `${currentPath}${path.delimiter}${normalizedEntry}`
      : `${normalizedEntry}${path.delimiter}${currentPath}`;
  return env;
}

function resolveManagedToolchainDir(options = {}) {
  const env = options.env || process.env;
  const directOverride =
    options.managedToolchainDir !== undefined
      ? options.managedToolchainDir
      : env.OMNE_MANAGED_TOOLCHAIN_DIR;
  if (directOverride && String(directOverride).trim() !== "") {
    return path.resolve(String(directOverride).trim());
  }

  const targetTriple = options.targetTriple || detectTargetTriple(options);
  if (!targetTriple) return null;
  const homeDir = options.homeDir || os.homedir();
  if (!homeDir || String(homeDir).trim() === "") return null;
  return path.join(homeDir, ".omne", "toolchain", targetTriple, "bin");
}

function buildSpawnEnv(binPath, options = {}) {
  const env = Object.assign({}, options.baseEnv || process.env);
  const pkgRoot = options.pkgRoot || path.resolve(__dirname, "..");
  const targetTriple = options.targetTriple || detectTargetTriple(options);
  const existsSync = options.existsSync || fs.existsSync;

  const managedToolchainDir = resolveManagedToolchainDir({
    env: options.env || process.env,
    managedToolchainDir: options.managedToolchainDir,
    homeDir: options.homeDir,
    targetTriple,
  });
  if (managedToolchainDir && existsSync(managedToolchainDir)) {
    addPathEntry(env, managedToolchainDir, "append");
  }
  if (!binPath || !targetTriple) return env;

  const vendorBinDir = path.resolve(pkgRoot, "vendor", targetTriple, "omne");
  const resolvedBinPath = path.resolve(String(binPath));
  const underVendorBinDir =
    resolvedBinPath === vendorBinDir ||
    resolvedBinPath.startsWith(`${vendorBinDir}${path.sep}`);
  if (!underVendorBinDir) return env;

  const vendorPathDir = path.resolve(pkgRoot, "vendor", targetTriple, "path");
  if (!existsSync(vendorPathDir)) return env;
  addPathEntry(env, vendorPathDir, "prepend");
  return env;
}

module.exports = {
  addPathEntry,
  buildSpawnEnv,
  detectTargetTriple,
  resolveBinary,
  resolveManagedToolchainDir,
  resolveInvocation,
  resolveVendoredBinary,
  resolvePathKey,
  targetBinaryExt,
};
