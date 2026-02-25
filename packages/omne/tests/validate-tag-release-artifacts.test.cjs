"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { test } = require("node:test");

const packageRoot = path.resolve(__dirname, "..");
const scriptPath = path.join(
  packageRoot,
  "scripts",
  "validate-tag-release-artifacts.mjs"
);

function runValidate(args) {
  return spawnSync(process.execPath, [scriptPath, ...args], {
    encoding: "utf8",
  });
}

function writeJson(filePath, value) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`);
}

function writeText(filePath, text = "x\n") {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, text);
}

function makeReleaseTree(root, { osName, version, target }) {
  const artifactRoot = path.join(root, `omne-vendor-releases-${osName}`);
  const bundleName = `vendor-bundle-${version}-${target}`;
  const bundleRoot = path.join(artifactRoot, bundleName);
  writeJson(path.join(artifactRoot, "index.json"), {
    schema_version: 1,
    releases: [
      {
        version,
        target,
        release_dir: bundleName,
      },
    ],
  });
  writeJson(path.join(bundleRoot, "RELEASE.json"), {
    version,
    target,
  });
  writeText(path.join(bundleRoot, "SHA256SUMS"), "abc  file\n");
  return { artifactRoot, bundleRoot };
}

test("validate-tag-release-artifacts passes on matching version/target bundles", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-validate-artifacts-"));
  makeReleaseTree(tmp, {
    osName: "ubuntu-latest",
    version: "v0.3.0-test",
    target: "x86_64-unknown-linux-gnu",
  });
  makeReleaseTree(tmp, {
    osName: "windows-latest",
    version: "v0.3.0-test",
    target: "x86_64-pc-windows-msvc",
  });
  const res = runValidate([
    "--artifacts-root",
    tmp,
    "--tag-version",
    "v0.3.0-test",
  ]);
  assert.equal(res.status, 0, `stderr: ${res.stderr}`);
  assert.match(res.stdout, /"ok": true/);
  assert.match(res.stdout, /x86_64-unknown-linux-gnu/);
  assert.match(res.stdout, /x86_64-pc-windows-msvc/);
});

test("validate-tag-release-artifacts fails when index.json misses requested version", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-validate-artifacts-"));
  makeReleaseTree(tmp, {
    osName: "ubuntu-latest",
    version: "v0.3.0-other",
    target: "x86_64-unknown-linux-gnu",
  });
  const res = runValidate([
    "--artifacts-root",
    tmp,
    "--tag-version",
    "v0.3.0-test",
  ]);
  assert.notEqual(res.status, 0);
  assert.match(res.stderr, /does not contain version=v0.3.0-test/);
});

test("validate-tag-release-artifacts fails when RELEASE target mismatches bundle dir", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-validate-artifacts-"));
  const { bundleRoot } = makeReleaseTree(tmp, {
    osName: "ubuntu-latest",
    version: "v0.3.0-test",
    target: "x86_64-unknown-linux-gnu",
  });
  writeJson(path.join(bundleRoot, "RELEASE.json"), {
    version: "v0.3.0-test",
    target: "aarch64-unknown-linux-gnu",
  });
  const res = runValidate([
    "--artifacts-root",
    tmp,
    "--tag-version",
    "v0.3.0-test",
  ]);
  assert.notEqual(res.status, 0);
  assert.match(res.stderr, /RELEASE.json target mismatch/);
});

test("validate-tag-release-artifacts fails on duplicate index entries for same version/target", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-validate-artifacts-"));
  const artifactRoot = path.join(tmp, "omne-vendor-releases-ubuntu-latest");
  const version = "v0.3.0-test";
  const target = "x86_64-unknown-linux-gnu";
  const bundleName = `vendor-bundle-${version}-${target}`;
  const bundleRoot = path.join(artifactRoot, bundleName);

  writeJson(path.join(artifactRoot, "index.json"), {
    releases: [
      { version, target, release_dir: bundleName },
      { version, target, release_dir: `${bundleName}-dup` },
    ],
  });
  writeJson(path.join(bundleRoot, "RELEASE.json"), { version, target });
  writeText(path.join(bundleRoot, "SHA256SUMS"), "abc  file\n");

  const res = runValidate([
    "--artifacts-root",
    tmp,
    "--tag-version",
    version,
  ]);
  assert.notEqual(res.status, 0);
  assert.match(res.stderr, /expected exactly 1/);
});

test("validate-tag-release-artifacts fails when index release_dir mismatches bundle dir", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-validate-artifacts-"));
  const artifactRoot = path.join(tmp, "omne-vendor-releases-ubuntu-latest");
  const version = "v0.3.0-test";
  const target = "x86_64-unknown-linux-gnu";
  const bundleName = `vendor-bundle-${version}-${target}`;
  const bundleRoot = path.join(artifactRoot, bundleName);

  writeJson(path.join(artifactRoot, "index.json"), {
    releases: [
      {
        version,
        target,
        release_dir: `vendor-bundle-${version}-aarch64-unknown-linux-gnu`,
      },
    ],
  });
  writeJson(path.join(bundleRoot, "RELEASE.json"), { version, target });
  writeText(path.join(bundleRoot, "SHA256SUMS"), "abc  file\n");

  const res = runValidate([
    "--artifacts-root",
    tmp,
    "--tag-version",
    version,
  ]);
  assert.notEqual(res.status, 0);
  assert.match(res.stderr, /release_dir mismatch/);
});

test("validate-tag-release-artifacts fails when index target has no matching bundle dir", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-validate-artifacts-"));
  const artifactRoot = path.join(tmp, "omne-vendor-releases-ubuntu-latest");
  const version = "v0.3.0-test";
  const target = "x86_64-unknown-linux-gnu";
  const missingTarget = "aarch64-unknown-linux-gnu";
  const bundleName = `vendor-bundle-${version}-${target}`;
  const bundleRoot = path.join(artifactRoot, bundleName);

  writeJson(path.join(artifactRoot, "index.json"), {
    releases: [
      { version, target, release_dir: bundleName },
      {
        version,
        target: missingTarget,
        release_dir: `vendor-bundle-${version}-${missingTarget}`,
      },
    ],
  });
  writeJson(path.join(bundleRoot, "RELEASE.json"), { version, target });
  writeText(path.join(bundleRoot, "SHA256SUMS"), "abc  file\n");

  const res = runValidate([
    "--artifacts-root",
    tmp,
    "--tag-version",
    version,
  ]);
  assert.notEqual(res.status, 0);
  assert.match(res.stderr, /targets with no matching bundle directory/);
  assert.match(res.stderr, /aarch64-unknown-linux-gnu/);
});
