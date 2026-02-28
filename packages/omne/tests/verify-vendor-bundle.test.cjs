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

function makeBundleFixture(options = {}) {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-verify-"));
  const srcRoot = path.join(tmp, "src");
  const vendorOut = path.join(tmp, "vendor-out");
  const distOut = path.join(tmp, "dist-out");
  const target = "x86_64-unknown-linux-gnu";

  const omneSrc = path.join(srcRoot, "omne");
  const appServerSrc = path.join(srcRoot, "omne-app-server");
  writeFile(omneSrc, "omne-bin");
  writeFile(appServerSrc, "app-server-bin");
  const pathDir = path.join(srcRoot, "path");
  if (options.includeCliFeatures) {
    writeFile(path.join(pathDir, "git"), "git-bin");
    writeFile(path.join(pathDir, "gh"), "gh-bin");
  }

  const buildArgs = [
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
  ];
  if (options.includeCliFeatures) {
    buildArgs.push("--path-dir", pathDir);
  }
  const build = runScript(buildScript, buildArgs);
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

test("verify-vendor-bundle fails on declared cli feature without binary", () => {
  const fixture = makeBundleFixture();
  const manifestPath = path.join(fixture.bundleRoot, "manifest.json");
  const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
  manifest.features = ["git-cli"];
  fs.writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
  const res = runScript(verifyScript, ["--bundle", fixture.bundleRoot]);
  assert.notEqual(res.status, 0);
  assert.match(res.stderr, /feature mismatch: git-cli declared but missing/);
});
