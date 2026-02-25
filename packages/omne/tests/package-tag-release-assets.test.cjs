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
  "package-tag-release-assets.mjs"
);

function runPack(args) {
  return spawnSync(process.execPath, [scriptPath, ...args], {
    encoding: "utf8",
  });
}

function runTarList(tarPath) {
  return spawnSync("tar", ["-tzf", tarPath], {
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

function makeBundle(artifactRoot, version, target) {
  const bundleName = `vendor-bundle-${version}-${target}`;
  const bundleRoot = path.join(artifactRoot, bundleName);
  writeJson(path.join(bundleRoot, "RELEASE.json"), { version, target });
  writeText(path.join(bundleRoot, "SHA256SUMS"), "abc  file\n");
  writeText(path.join(bundleRoot, "manifest.json"), "{}\n");
}

test("package-tag-release-assets packages only tag-matched bundles", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-package-release-assets-"));
  const artifactsRoot = path.join(tmp, "release-artifacts");
  const outDir = path.join(tmp, "release-bundles");
  const payloadRoot = path.join(tmp, "release-payloads");
  const tag = "v0.3.0-test";

  const linuxRoot = path.join(artifactsRoot, "omne-vendor-releases-ubuntu-latest");
  const winRoot = path.join(artifactsRoot, "omne-vendor-releases-windows-latest");

  writeJson(path.join(linuxRoot, "index.json"), {
    releases: [{ version: tag, target: "x86_64-unknown-linux-gnu" }],
  });
  writeJson(path.join(winRoot, "index.json"), {
    releases: [{ version: tag, target: "x86_64-pc-windows-msvc" }],
  });

  makeBundle(linuxRoot, tag, "x86_64-unknown-linux-gnu");
  makeBundle(linuxRoot, "v0.2.0-other", "x86_64-unknown-linux-gnu");
  makeBundle(winRoot, tag, "x86_64-pc-windows-msvc");

  const res = runPack([
    "--artifacts-root",
    artifactsRoot,
    "--tag-version",
    tag,
    "--out-dir",
    outDir,
    "--payload-root",
    payloadRoot,
  ]);
  assert.equal(res.status, 0, `stderr: ${res.stderr}`);
  assert.match(res.stdout, /"ok": true/);

  const linuxTar = path.join(outDir, "omne-vendor-releases-ubuntu-latest.tar.gz");
  const winTar = path.join(outDir, "omne-vendor-releases-windows-latest.tar.gz");
  assert.equal(fs.existsSync(linuxTar), true);
  assert.equal(fs.existsSync(winTar), true);
  assert.equal(fs.existsSync(path.join(outDir, "SHA256SUMS")), true);

  const linuxList = runTarList(linuxTar);
  assert.equal(linuxList.status, 0, linuxList.stderr);
  assert.match(linuxList.stdout, /index\.json/);
  assert.match(linuxList.stdout, new RegExp(`vendor-bundle-${tag}-x86_64-unknown-linux-gnu`));
  assert.doesNotMatch(linuxList.stdout, /vendor-bundle-v0\.2\.0-other/);
});

test("package-tag-release-assets fails when index misses requested tag version", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-package-release-assets-"));
  const artifactsRoot = path.join(tmp, "release-artifacts");
  const linuxRoot = path.join(artifactsRoot, "omne-vendor-releases-ubuntu-latest");
  writeJson(path.join(linuxRoot, "index.json"), { releases: [] });
  makeBundle(linuxRoot, "v0.2.0-other", "x86_64-unknown-linux-gnu");

  const res = runPack([
    "--artifacts-root",
    artifactsRoot,
    "--tag-version",
    "v0.3.0-test",
    "--out-dir",
    path.join(tmp, "out"),
    "--payload-root",
    path.join(tmp, "payload"),
  ]);
  assert.notEqual(res.status, 0);
  assert.match(res.stderr, /does not contain version=v0.3.0-test/);
});

test("package-tag-release-assets clears stale output and payload directories", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-package-release-assets-"));
  const artifactsRoot = path.join(tmp, "release-artifacts");
  const outDir = path.join(tmp, "release-bundles");
  const payloadRoot = path.join(tmp, "release-payloads");
  const tag = "v0.3.1-test";

  const linuxRoot = path.join(artifactsRoot, "omne-vendor-releases-ubuntu-latest");
  writeJson(path.join(linuxRoot, "index.json"), {
    releases: [{ version: tag, target: "x86_64-unknown-linux-gnu" }],
  });
  makeBundle(linuxRoot, tag, "x86_64-unknown-linux-gnu");

  fs.mkdirSync(outDir, { recursive: true });
  fs.mkdirSync(payloadRoot, { recursive: true });
  fs.writeFileSync(path.join(outDir, "stale-old.tar.gz"), "stale");
  fs.mkdirSync(path.join(payloadRoot, "old"), { recursive: true });
  fs.writeFileSync(path.join(payloadRoot, "old", "stale.txt"), "stale");

  const res = runPack([
    "--artifacts-root",
    artifactsRoot,
    "--tag-version",
    tag,
    "--out-dir",
    outDir,
    "--payload-root",
    payloadRoot,
  ]);

  assert.equal(res.status, 0, `stderr: ${res.stderr}`);
  assert.equal(fs.existsSync(path.join(outDir, "stale-old.tar.gz")), false);
  assert.equal(fs.existsSync(path.join(payloadRoot, "old", "stale.txt")), false);
  assert.equal(
    fs.existsSync(path.join(outDir, "omne-vendor-releases-ubuntu-latest.tar.gz")),
    true
  );
});

test("package-tag-release-assets fails when index target has no matching bundle dir", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-package-release-assets-"));
  const artifactsRoot = path.join(tmp, "release-artifacts");
  const linuxRoot = path.join(artifactsRoot, "omne-vendor-releases-ubuntu-latest");
  const tag = "v0.3.2-test";

  writeJson(path.join(linuxRoot, "index.json"), {
    releases: [
      { version: tag, target: "x86_64-unknown-linux-gnu" },
      { version: tag, target: "aarch64-unknown-linux-gnu" },
    ],
  });
  makeBundle(linuxRoot, tag, "x86_64-unknown-linux-gnu");

  const res = runPack([
    "--artifacts-root",
    artifactsRoot,
    "--tag-version",
    tag,
    "--out-dir",
    path.join(tmp, "out"),
    "--payload-root",
    path.join(tmp, "payload"),
  ]);
  assert.notEqual(res.status, 0);
  assert.match(res.stderr, /targets with no matching bundle directory/);
  assert.match(res.stderr, /aarch64-unknown-linux-gnu/);
});

test("package-tag-release-assets fails when bundle target missing in index", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-package-release-assets-"));
  const artifactsRoot = path.join(tmp, "release-artifacts");
  const linuxRoot = path.join(artifactsRoot, "omne-vendor-releases-ubuntu-latest");
  const tag = "v0.3.3-test";

  writeJson(path.join(linuxRoot, "index.json"), {
    releases: [{ version: tag, target: "x86_64-unknown-linux-gnu" }],
  });
  makeBundle(linuxRoot, tag, "x86_64-unknown-linux-gnu");
  makeBundle(linuxRoot, tag, "aarch64-unknown-linux-gnu");

  const res = runPack([
    "--artifacts-root",
    artifactsRoot,
    "--tag-version",
    tag,
    "--out-dir",
    path.join(tmp, "out"),
    "--payload-root",
    path.join(tmp, "payload"),
  ]);
  assert.notEqual(res.status, 0);
  assert.match(res.stderr, /targets missing in index\.json/);
  assert.match(res.stderr, /aarch64-unknown-linux-gnu/);
});
