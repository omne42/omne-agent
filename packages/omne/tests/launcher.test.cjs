"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { test } = require("node:test");

const {
  buildSpawnEnv,
  detectTargetTriple,
  resolveBinary,
  resolveInvocation,
  resolveVendoredBinary,
} = require("../lib/launcher.js");

test("detectTargetTriple maps known platform/arch and respects override", () => {
  assert.equal(
    detectTargetTriple({ platform: "linux", arch: "x64", env: {} }),
    "x86_64-unknown-linux-gnu"
  );
  assert.equal(
    detectTargetTriple({ platform: "darwin", arch: "arm64", env: {} }),
    "aarch64-apple-darwin"
  );
  assert.equal(
    detectTargetTriple({
      platform: "linux",
      arch: "x64",
      env: { OMNE_TARGET_TRIPLE: "custom-triple" },
    }),
    "custom-triple"
  );
  assert.equal(detectTargetTriple({ platform: "freebsd", arch: "x64", env: {} }), null);
});

test("resolveVendoredBinary returns candidate only when it exists", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-launcher-"));
  const target = "x86_64-unknown-linux-gnu";
  const existing = path.join(tmp, "vendor", target, "omne", "omne");
  fs.mkdirSync(path.dirname(existing), { recursive: true });
  fs.writeFileSync(existing, "bin");

  assert.equal(
    resolveVendoredBinary("omne", {
      pkgRoot: tmp,
      targetTriple: target,
      existsSync: fs.existsSync,
    }),
    existing
  );
  assert.equal(
    resolveVendoredBinary("omne-app-server", {
      pkgRoot: tmp,
      targetTriple: target,
      existsSync: fs.existsSync,
    }),
    null
  );
});

test("resolveBinary priority: env override > vendored > fallback", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-launcher-"));
  const target = "x86_64-unknown-linux-gnu";
  const vendored = path.join(tmp, "vendor", target, "omne", "omne");
  fs.mkdirSync(path.dirname(vendored), { recursive: true });
  fs.writeFileSync(vendored, "bin");

  assert.equal(
    resolveBinary("omne", "OMNE_PM_BIN", {
      env: { OMNE_PM_BIN: "/abs/custom/omne" },
      pkgRoot: tmp,
      targetTriple: target,
      existsSync: fs.existsSync,
    }),
    "/abs/custom/omne"
  );
  assert.equal(
    resolveBinary("omne", "OMNE_PM_BIN", {
      env: {},
      pkgRoot: tmp,
      targetTriple: target,
      existsSync: fs.existsSync,
    }),
    vendored
  );
  assert.equal(
    resolveBinary("omne-app-server", "OMNE_APP_SERVER_BIN", {
      env: {},
      pkgRoot: tmp,
      targetTriple: target,
      existsSync: fs.existsSync,
    }),
    "omne-app-server"
  );
});

test("resolveInvocation switches app-server subcommand to app-server binary", () => {
  const normal = resolveInvocation(["--help"], { env: { OMNE_PM_BIN: "/p/omne" } });
  assert.equal(normal.logicalBinName, "omne");
  assert.equal(normal.bin, "/p/omne");
  assert.deepEqual(normal.args, ["--help"]);

  const appServer = resolveInvocation(["app-server", "--help"], {
    env: { OMNE_APP_SERVER_BIN: "/p/omne-app-server" },
  });
  assert.equal(appServer.logicalBinName, "omne-app-server");
  assert.equal(appServer.bin, "/p/omne-app-server");
  assert.deepEqual(appServer.args, ["--help"]);
});

test("buildSpawnEnv prepends vendor path only for vendored binary", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-launcher-"));
  const target = "x86_64-unknown-linux-gnu";
  const vendorBin = path.join(tmp, "vendor", target, "omne", "omne");
  const vendorPath = path.join(tmp, "vendor", target, "path");
  fs.mkdirSync(path.dirname(vendorBin), { recursive: true });
  fs.mkdirSync(vendorPath, { recursive: true });
  fs.writeFileSync(vendorBin, "bin");

  const env = buildSpawnEnv(vendorBin, {
    baseEnv: { PATH: "/usr/bin" },
    pkgRoot: tmp,
    targetTriple: target,
    existsSync: fs.existsSync,
  });
  assert.equal(env.PATH, `${vendorPath}${path.delimiter}/usr/bin`);

  const noChange = buildSpawnEnv("/abs/non-vendor/omne", {
    baseEnv: { PATH: "/usr/bin" },
    pkgRoot: tmp,
    targetTriple: target,
    existsSync: fs.existsSync,
  });
  assert.equal(noChange.PATH, "/usr/bin");
});
