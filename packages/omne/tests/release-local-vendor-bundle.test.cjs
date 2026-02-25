"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { test } = require("node:test");

const packageRoot = path.resolve(__dirname, "..");
const scriptPath = path.join(packageRoot, "scripts", "release-local-vendor-bundle.mjs");

function runLocalRelease(args) {
  return spawnSync(process.execPath, [scriptPath, ...args], {
    encoding: "utf8",
  });
}

function writeFile(filePath, content = "x") {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, content);
}

test("release-local-vendor-bundle performs one-shot local release", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-release-local-"));
  const targetDir = path.join(tmp, "target");
  const vendorOut = path.join(tmp, "vendor-out");
  const distOut = path.join(tmp, "dist-out");
  const releaseOut = path.join(tmp, "release-out");
  const target = "x86_64-unknown-linux-gnu";
  const version = "v0.3.0-local";

  writeFile(path.join(targetDir, "debug", "omne"), "omne");
  writeFile(path.join(targetDir, "debug", "omne-app-server"), "app-server");

  const res = runLocalRelease([
    "--version",
    version,
    "--target",
    target,
    "--target-dir",
    targetDir,
    "--vendor-out",
    vendorOut,
    "--dist-out",
    distOut,
    "--release-out",
    releaseOut,
    "--clean",
  ]);

  assert.equal(res.status, 0, `stderr: ${res.stderr}`);
  assert.match(res.stdout, /release-host: target=x86_64-unknown-linux-gnu version=v0.3.0-local/);
  assert.match(
    res.stdout,
    new RegExp(`release-local: release_dir=.*vendor-bundle-${version}-${target}`)
  );
  assert.match(res.stdout, /release-local: index=/);

  const releaseRoot = path.join(releaseOut, `vendor-bundle-${version}-${target}`);
  assert.equal(fs.existsSync(path.join(releaseRoot, "RELEASE.json")), true);
  assert.equal(fs.existsSync(path.join(releaseRoot, "SHA256SUMS")), true);
  assert.equal(fs.existsSync(path.join(releaseOut, "index.json")), true);
});

test("release-local-vendor-bundle propagates errors from release-host", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-release-local-"));
  const res = runLocalRelease([
    "--target",
    "x86_64-unknown-linux-gnu",
    "--target-dir",
    path.join(tmp, "missing-target"),
  ]);
  assert.notEqual(res.status, 0);
  assert.match(res.stderr, /cannot resolve omne binary/);
});
