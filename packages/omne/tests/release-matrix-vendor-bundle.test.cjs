"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { test } = require("node:test");

const packageRoot = path.resolve(__dirname, "..");
const scriptPath = path.join(packageRoot, "scripts", "release-matrix-vendor-bundle.mjs");
const DEFAULT_TARGETS = [
  "x86_64-unknown-linux-gnu",
  "aarch64-unknown-linux-gnu",
  "x86_64-apple-darwin",
  "aarch64-apple-darwin",
  "x86_64-pc-windows-msvc",
  "aarch64-pc-windows-msvc",
];

function runMatrix(args) {
  return spawnSync(process.execPath, [scriptPath, ...args], {
    encoding: "utf8",
  });
}

function binaryExt(target) {
  return target.includes("windows") ? ".exe" : "";
}

function writeFile(filePath, content = "x") {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, content);
}

test("release-matrix-vendor-bundle releases multiple targets and writes summary", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-release-matrix-"));
  const targetDir = path.join(tmp, "target");
  const vendorOut = path.join(tmp, "vendor-out");
  const distOut = path.join(tmp, "dist-out");
  const releaseOut = path.join(tmp, "release-out");
  const version = "v0.3.0-matrix";
  const targets = ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"];

  for (const target of targets) {
    writeFile(path.join(targetDir, target, "debug", "omne"), `omne-${target}`);
    writeFile(
      path.join(targetDir, target, "debug", "omne-app-server"),
      `app-server-${target}`
    );
  }

  const res = runMatrix([
    "--version",
    version,
    "--targets",
    targets.join(","),
    "--target-dir",
    targetDir,
    "--profile",
    "debug",
    "--vendor-out",
    vendorOut,
    "--dist-out",
    distOut,
    "--release-out",
    releaseOut,
    "--clean",
  ]);

  assert.equal(res.status, 0, `stderr: ${res.stderr}`);
  assert.match(res.stdout, /release-matrix: target=x86_64-unknown-linux-gnu/);
  assert.match(res.stdout, /release-matrix: target=aarch64-unknown-linux-gnu/);
  assert.equal(
    fs.existsSync(
      path.join(releaseOut, `vendor-bundle-${version}-x86_64-unknown-linux-gnu`, "RELEASE.json")
    ),
    true
  );
  assert.equal(
    fs.existsSync(
      path.join(releaseOut, `vendor-bundle-${version}-aarch64-unknown-linux-gnu`, "RELEASE.json")
    ),
    true
  );

  const runSummary = JSON.parse(fs.readFileSync(path.join(releaseOut, "last-run.json"), "utf8"));
  assert.deepEqual(runSummary.targets, targets);
  assert.equal(Array.isArray(runSummary.results), true);
  assert.equal(runSummary.results.length, 2);

  const index = JSON.parse(fs.readFileSync(path.join(releaseOut, "index.json"), "utf8"));
  assert.equal(index.releases.length, 2);
});

test("release-matrix-vendor-bundle uses default target matrix", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-release-matrix-default-"));
  const targetDir = path.join(tmp, "target");
  const vendorOut = path.join(tmp, "vendor-out");
  const distOut = path.join(tmp, "dist-out");
  const releaseOut = path.join(tmp, "release-out");
  const version = "v0.3.0-matrix-default";

  for (const target of DEFAULT_TARGETS) {
    const ext = binaryExt(target);
    writeFile(path.join(targetDir, target, "debug", `omne${ext}`), `omne-${target}`);
    writeFile(
      path.join(targetDir, target, "debug", `omne-app-server${ext}`),
      `app-server-${target}`
    );
  }

  const res = runMatrix([
    "--version",
    version,
    "--target-dir",
    targetDir,
    "--profile",
    "debug",
    "--vendor-out",
    vendorOut,
    "--dist-out",
    distOut,
    "--release-out",
    releaseOut,
    "--clean",
  ]);

  assert.equal(res.status, 0, `stderr: ${res.stderr}`);
  for (const target of DEFAULT_TARGETS) {
    assert.match(res.stdout, new RegExp(`release-matrix: target=${target}`));
    assert.equal(
      fs.existsSync(path.join(releaseOut, `vendor-bundle-${version}-${target}`, "RELEASE.json")),
      true
    );
  }

  const runSummary = JSON.parse(fs.readFileSync(path.join(releaseOut, "last-run.json"), "utf8"));
  assert.deepEqual(runSummary.targets, DEFAULT_TARGETS);
  assert.equal(Array.isArray(runSummary.results), true);
  assert.equal(runSummary.results.length, DEFAULT_TARGETS.length);

  const index = JSON.parse(fs.readFileSync(path.join(releaseOut, "index.json"), "utf8"));
  assert.equal(index.releases.length, DEFAULT_TARGETS.length);
});

test("release-matrix-vendor-bundle fails when one target is missing", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-release-matrix-"));
  const targetDir = path.join(tmp, "target");
  const vendorOut = path.join(tmp, "vendor-out");
  const distOut = path.join(tmp, "dist-out");
  const releaseOut = path.join(tmp, "release-out");
  writeFile(path.join(targetDir, "x86_64-unknown-linux-gnu", "debug", "omne"), "omne");
  writeFile(
    path.join(targetDir, "x86_64-unknown-linux-gnu", "debug", "omne-app-server"),
    "app-server"
  );
  const res = runMatrix([
    "--version",
    "v0.3.0-matrix-fail",
    "--targets",
    "x86_64-unknown-linux-gnu,aarch64-unknown-linux-gnu",
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
  assert.notEqual(res.status, 0);
  assert.match(res.stderr, /release-matrix: failed target=aarch64-unknown-linux-gnu/);
});
