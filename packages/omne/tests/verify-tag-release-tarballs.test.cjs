"use strict";

const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { test } = require("node:test");

const packageRoot = path.resolve(__dirname, "..");
const packageScriptPath = path.join(
  packageRoot,
  "scripts",
  "package-tag-release-assets.mjs"
);
const verifyScriptPath = path.join(
  packageRoot,
  "scripts",
  "verify-tag-release-tarballs.mjs"
);

function runNode(scriptPath, args) {
  return spawnSync(process.execPath, [scriptPath, ...args], {
    encoding: "utf8",
  });
}

function runTarCreate(sourceDir, outTarGzPath) {
  return spawnSync("tar", ["-C", sourceDir, "-czf", outTarGzPath, "."], {
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

test("verify-tag-release-tarballs validates package outputs", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-verify-release-tarballs-"));
  const artifactsRoot = path.join(tmp, "release-artifacts");
  const outDir = path.join(tmp, "release-bundles");
  const payloadRoot = path.join(tmp, "release-payloads");
  const tag = "v0.3.4-test";
  const linuxRoot = path.join(artifactsRoot, "omne-vendor-releases-ubuntu-latest");

  writeJson(path.join(linuxRoot, "index.json"), {
    releases: [{ version: tag, target: "x86_64-unknown-linux-gnu" }],
  });
  makeBundle(linuxRoot, tag, "x86_64-unknown-linux-gnu");

  const pack = runNode(packageScriptPath, [
    "--artifacts-root",
    artifactsRoot,
    "--tag-version",
    tag,
    "--out-dir",
    outDir,
    "--payload-root",
    payloadRoot,
  ]);
  assert.equal(pack.status, 0, `stderr: ${pack.stderr}`);

  const verify = runNode(verifyScriptPath, [
    "--out-dir",
    outDir,
    "--tag-version",
    tag,
  ]);
  assert.equal(verify.status, 0, `stderr: ${verify.stderr}`);
  assert.match(verify.stdout, /"ok": true/);
  assert.match(verify.stdout, /x86_64-unknown-linux-gnu/);
});

test("verify-tag-release-tarballs fails on unexpected tar entry", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-verify-release-tarballs-"));
  const outDir = path.join(tmp, "release-bundles");
  const payloadDir = path.join(tmp, "payload");
  const tag = "v0.3.5-test";
  const tarName = "omne-vendor-releases-ubuntu-latest.tar.gz";
  const tarPath = path.join(outDir, tarName);

  writeJson(path.join(payloadDir, "index.json"), {
    releases: [{ version: tag, target: "x86_64-unknown-linux-gnu" }],
  });
  writeJson(
    path.join(payloadDir, `vendor-bundle-${tag}-x86_64-unknown-linux-gnu`, "RELEASE.json"),
    { version: tag, target: "x86_64-unknown-linux-gnu" }
  );
  writeText(
    path.join(payloadDir, `vendor-bundle-${tag}-x86_64-unknown-linux-gnu`, "SHA256SUMS"),
    "abc  file\n"
  );
  writeText(path.join(payloadDir, "leak.txt"), "should-not-be-here\n");

  fs.mkdirSync(outDir, { recursive: true });
  const tar = runTarCreate(payloadDir, tarPath);
  assert.equal(tar.status, 0, tar.stderr);

  const sum = crypto.createHash("sha256").update(fs.readFileSync(tarPath)).digest("hex");
  writeText(path.join(outDir, "SHA256SUMS"), `${sum}  ${tarName}\n`);

  const verify = runNode(verifyScriptPath, [
    "--out-dir",
    outDir,
    "--tag-version",
    tag,
  ]);
  assert.notEqual(verify.status, 0);
  assert.match(verify.stderr, /contains unexpected entry/);
  assert.match(verify.stderr, /leak\.txt/);
});

test("verify-tag-release-tarballs fails when index target has no matching bundle", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-verify-release-tarballs-"));
  const outDir = path.join(tmp, "release-bundles");
  const payloadDir = path.join(tmp, "payload");
  const tag = "v0.3.6-test";
  const tarName = "omne-vendor-releases-ubuntu-latest.tar.gz";
  const tarPath = path.join(outDir, tarName);

  writeJson(path.join(payloadDir, "index.json"), {
    releases: [
      { version: tag, target: "x86_64-unknown-linux-gnu" },
      { version: tag, target: "aarch64-unknown-linux-gnu" },
    ],
  });
  writeJson(
    path.join(payloadDir, `vendor-bundle-${tag}-x86_64-unknown-linux-gnu`, "RELEASE.json"),
    { version: tag, target: "x86_64-unknown-linux-gnu" }
  );
  writeText(
    path.join(payloadDir, `vendor-bundle-${tag}-x86_64-unknown-linux-gnu`, "SHA256SUMS"),
    "abc  file\n"
  );

  fs.mkdirSync(outDir, { recursive: true });
  const tar = runTarCreate(payloadDir, tarPath);
  assert.equal(tar.status, 0, tar.stderr);

  const sum = crypto.createHash("sha256").update(fs.readFileSync(tarPath)).digest("hex");
  writeText(path.join(outDir, "SHA256SUMS"), `${sum}  ${tarName}\n`);

  const verify = runNode(verifyScriptPath, [
    "--out-dir",
    outDir,
    "--tag-version",
    tag,
  ]);
  assert.notEqual(verify.status, 0);
  assert.match(verify.stderr, /targets with no matching bundle directory/);
  assert.match(verify.stderr, /aarch64-unknown-linux-gnu/);
});

test("verify-tag-release-tarballs fails when RELEASE target mismatches bundle dir", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-verify-release-tarballs-"));
  const outDir = path.join(tmp, "release-bundles");
  const payloadDir = path.join(tmp, "payload");
  const tag = "v0.3.7-test";
  const tarName = "omne-vendor-releases-ubuntu-latest.tar.gz";
  const tarPath = path.join(outDir, tarName);

  writeJson(path.join(payloadDir, "index.json"), {
    releases: [{ version: tag, target: "x86_64-unknown-linux-gnu" }],
  });
  writeJson(
    path.join(payloadDir, `vendor-bundle-${tag}-x86_64-unknown-linux-gnu`, "RELEASE.json"),
    { version: tag, target: "aarch64-unknown-linux-gnu" }
  );
  writeText(
    path.join(payloadDir, `vendor-bundle-${tag}-x86_64-unknown-linux-gnu`, "SHA256SUMS"),
    "abc  file\n"
  );

  fs.mkdirSync(outDir, { recursive: true });
  const tar = runTarCreate(payloadDir, tarPath);
  assert.equal(tar.status, 0, tar.stderr);

  const sum = crypto.createHash("sha256").update(fs.readFileSync(tarPath)).digest("hex");
  writeText(path.join(outDir, "SHA256SUMS"), `${sum}  ${tarName}\n`);

  const verify = runNode(verifyScriptPath, [
    "--out-dir",
    outDir,
    "--tag-version",
    tag,
  ]);
  assert.notEqual(verify.status, 0);
  assert.match(verify.stderr, /RELEASE\.json target mismatch/);
});
