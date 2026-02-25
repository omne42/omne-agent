"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { test } = require("node:test");

const packageRoot = path.resolve(__dirname, "..");
const scriptPath = path.join(packageRoot, "scripts", "release-vendor-bundle.mjs");

function runRelease(args) {
  return spawnSync(process.execPath, [scriptPath, ...args], {
    encoding: "utf8",
  });
}

function writeFile(filePath, content = "x") {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, content);
}

test("release-vendor-bundle creates versioned release with checksums", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-release-"));
  const srcRoot = path.join(tmp, "src");
  const vendorOut = path.join(tmp, "vendor-out");
  const distOut = path.join(tmp, "dist-out");
  const releaseOut = path.join(tmp, "release-out");
  const target = "x86_64-unknown-linux-gnu";
  const version = "v0.3.0-test";

  const omneSrc = path.join(srcRoot, "omne");
  const appServerSrc = path.join(srcRoot, "omne-app-server");
  const pathDir = path.join(srcRoot, "path");
  writeFile(omneSrc, "omne-bin");
  writeFile(appServerSrc, "app-server-bin");
  writeFile(path.join(pathDir, "rg"), "rg-bin");

  const res = runRelease([
    "--target",
    target,
    "--version",
    version,
    "--omne",
    omneSrc,
    "--app-server",
    appServerSrc,
    "--path-dir",
    pathDir,
    "--vendor-out",
    vendorOut,
    "--dist-out",
    distOut,
    "--release-out",
    releaseOut,
    "--clean",
  ]);

  assert.equal(res.status, 0, `stderr: ${res.stderr}`);
  assert.match(res.stdout, /released vendor bundle:/);

  const releaseRoot = path.join(releaseOut, `vendor-bundle-${version}-${target}`);
  const releaseJson = JSON.parse(
    fs.readFileSync(path.join(releaseRoot, "RELEASE.json"), "utf8")
  );
  assert.equal(releaseJson.version, version);
  assert.equal(releaseJson.target, target);

  const index = JSON.parse(fs.readFileSync(path.join(releaseOut, "index.json"), "utf8"));
  assert.ok(Array.isArray(index.releases));
  assert.ok(index.releases.some((item) => item.name === `vendor-bundle-${version}-${target}`));

  const shaSums = fs.readFileSync(path.join(releaseRoot, "SHA256SUMS"), "utf8");
  assert.match(shaSums, /manifest\.json/);
  assert.match(shaSums, /vendor\/x86_64-unknown-linux-gnu\/omne\/omne/);
  assert.match(
    fs.readFileSync(path.join(releaseRoot, "vendor", target, "path", "rg"), "utf8"),
    /rg-bin/
  );
});

test("release-vendor-bundle requires --version", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-release-"));
  const omneSrc = path.join(tmp, "bin", "omne");
  const appServerSrc = path.join(tmp, "bin", "omne-app-server");
  writeFile(omneSrc, "omne-bin");
  writeFile(appServerSrc, "app-server-bin");

  const res = runRelease([
    "--target",
    "x86_64-unknown-linux-gnu",
    "--omne",
    omneSrc,
    "--app-server",
    appServerSrc,
  ]);

  assert.notEqual(res.status, 0);
  assert.match(res.stderr, /--version is required/);
  assert.match(res.stderr, /Usage:/);
});
