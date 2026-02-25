"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { test } = require("node:test");

const packageRoot = path.resolve(__dirname, "..");
const buildScript = path.join(packageRoot, "scripts", "build-vendor-bundle.mjs");
const verifyScript = path.join(packageRoot, "scripts", "verify-vendor-bundle.mjs");

function runScript(scriptPath, args) {
  return spawnSync(process.execPath, [scriptPath, ...args], { encoding: "utf8" });
}

function writeFile(filePath, content = "x") {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, content);
}

function makeBundleFixture() {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-verify-"));
  const srcRoot = path.join(tmp, "src");
  const vendorOut = path.join(tmp, "vendor-out");
  const distOut = path.join(tmp, "dist-out");
  const target = "x86_64-unknown-linux-gnu";

  const omneSrc = path.join(srcRoot, "omne");
  const appServerSrc = path.join(srcRoot, "omne-app-server");
  writeFile(omneSrc, "omne-bin");
  writeFile(appServerSrc, "app-server-bin");

  const build = runScript(buildScript, [
    "--target",
    target,
    "--omne",
    omneSrc,
    "--app-server",
    appServerSrc,
    "--vendor-out",
    vendorOut,
    "--dist-out",
    distOut,
    "--clean",
  ]);
  assert.equal(build.status, 0, `stderr: ${build.stderr}`);
  return {
    bundleRoot: path.join(distOut, `vendor-bundle-${target}`),
    target,
  };
}

test("verify-vendor-bundle validates a correct bundle", () => {
  const fixture = makeBundleFixture();
  const res = runScript(verifyScript, ["--bundle", fixture.bundleRoot]);
  assert.equal(res.status, 0, `stderr: ${res.stderr}`);
  assert.match(res.stdout, /bundle verified:/);
});

test("verify-vendor-bundle fails on tampered file", () => {
  const fixture = makeBundleFixture();
  const payloadPath = path.join(
    fixture.bundleRoot,
    "vendor",
    fixture.target,
    "omne",
    "omne"
  );
  fs.writeFileSync(payloadPath, "tampered");
  const res = runScript(verifyScript, ["--bundle", fixture.bundleRoot]);
  assert.notEqual(res.status, 0);
  assert.match(res.stderr, /mismatch/);
});
