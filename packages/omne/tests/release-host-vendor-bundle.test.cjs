"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { test } = require("node:test");

const packageRoot = path.resolve(__dirname, "..");
const scriptPath = path.join(packageRoot, "scripts", "release-host-vendor-bundle.mjs");

function runHostRelease(args) {
  return spawnSync(process.execPath, [scriptPath, ...args], {
    encoding: "utf8",
  });
}

function writeFile(filePath, content = "x") {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, content);
}

test("release-host-vendor-bundle resolves binaries from target dir", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-release-host-"));
  const targetDir = path.join(tmp, "target");
  const vendorOut = path.join(tmp, "vendor-out");
  const distOut = path.join(tmp, "dist-out");
  const releaseOut = path.join(tmp, "release-out");
  const target = "x86_64-unknown-linux-gnu";
  const version = "v0.3.0-host";
  const profile = "debug";

  const omne = path.join(targetDir, profile, "omne");
  const appServer = path.join(targetDir, profile, "omne-app-server");
  writeFile(omne, "omne");
  writeFile(appServer, "app-server");

  const res = runHostRelease([
    "--version",
    version,
    "--target",
    target,
    "--target-dir",
    targetDir,
    "--profile",
    profile,
    "--vendor-out",
    vendorOut,
    "--dist-out",
    distOut,
    "--release-out",
    releaseOut,
    "--clean",
  ]);

  assert.equal(res.status, 0, `stderr: ${res.stderr}`);
  assert.match(res.stdout, /release-host: target=x86_64-unknown-linux-gnu version=v0.3.0-host/);

  const releaseRoot = path.join(releaseOut, `vendor-bundle-${version}-${target}`);
  assert.equal(fs.existsSync(path.join(releaseRoot, "RELEASE.json")), true);
  assert.equal(
    fs.existsSync(path.join(releaseRoot, "vendor", target, "omne", "omne")),
    true
  );
});

test("release-host-vendor-bundle supports explicit binary overrides", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-release-host-"));
  const vendorOut = path.join(tmp, "vendor-out");
  const distOut = path.join(tmp, "dist-out");
  const releaseOut = path.join(tmp, "release-out");
  const target = "x86_64-unknown-linux-gnu";
  const version = "v0.3.0-host-explicit";

  const omne = path.join(tmp, "bin", "omne");
  const appServer = path.join(tmp, "bin", "omne-app-server");
  writeFile(omne, "omne-explicit");
  writeFile(appServer, "app-server-explicit");

  const res = runHostRelease([
    "--version",
    version,
    "--target",
    target,
    "--omne",
    omne,
    "--app-server",
    appServer,
    "--vendor-out",
    vendorOut,
    "--dist-out",
    distOut,
    "--release-out",
    releaseOut,
    "--clean",
  ]);

  assert.equal(res.status, 0, `stderr: ${res.stderr}`);
  const releaseRoot = path.join(releaseOut, `vendor-bundle-${version}-${target}`);
  assert.match(
    fs.readFileSync(path.join(releaseRoot, "vendor", target, "omne", "omne"), "utf8"),
    /omne-explicit/
  );
});

test("release-host-vendor-bundle auto-generates version when omitted", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-release-host-"));
  const targetDir = path.join(tmp, "target");
  const vendorOut = path.join(tmp, "vendor-out");
  const distOut = path.join(tmp, "dist-out");
  const releaseOut = path.join(tmp, "release-out");
  const target = "x86_64-unknown-linux-gnu";
  const profile = "debug";

  const omne = path.join(targetDir, profile, "omne");
  const appServer = path.join(targetDir, profile, "omne-app-server");
  writeFile(omne, "omne");
  writeFile(appServer, "app-server");

  const res = runHostRelease([
    "--target",
    target,
    "--target-dir",
    targetDir,
    "--profile",
    profile,
    "--vendor-out",
    vendorOut,
    "--dist-out",
    distOut,
    "--release-out",
    releaseOut,
    "--clean",
  ]);
  assert.equal(res.status, 0, `stderr: ${res.stderr}`);
  assert.match(
    res.stdout,
    /release-host: target=x86_64-unknown-linux-gnu version=0\.0\.0-dev\.\d{14}/
  );

  const releaseEntries = fs
    .readdirSync(releaseOut, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .filter((name) => name.startsWith("vendor-bundle-0.0.0-dev."));
  assert.equal(releaseEntries.length, 1);
  assert.match(releaseEntries[0], /-x86_64-unknown-linux-gnu$/);
});

test("release-host-vendor-bundle errors when binaries cannot be resolved", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-release-host-"));
  const res = runHostRelease([
    "--target",
    "x86_64-unknown-linux-gnu",
    "--target-dir",
    path.join(tmp, "missing-target"),
  ]);
  assert.notEqual(res.status, 0);
  assert.match(res.stderr, /cannot resolve omne binary/);
});
