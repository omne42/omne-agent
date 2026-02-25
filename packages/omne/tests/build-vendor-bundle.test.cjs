"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { test } = require("node:test");

const packageRoot = path.resolve(__dirname, "..");
const scriptPath = path.join(packageRoot, "scripts", "build-vendor-bundle.mjs");

function runBuild(args) {
  return spawnSync(process.execPath, [scriptPath, ...args], {
    encoding: "utf8",
  });
}

function writeFile(filePath, content = "x") {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, content);
}

test("build-vendor-bundle builds bundle directory and manifest", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-bundle-"));
  const srcRoot = path.join(tmp, "src");
  const vendorOut = path.join(tmp, "vendor-out");
  const distOut = path.join(tmp, "dist-out");
  const target = "x86_64-unknown-linux-gnu";

  const omneSrc = path.join(srcRoot, "omne");
  const appServerSrc = path.join(srcRoot, "omne-app-server");
  const pathDir = path.join(srcRoot, "path");
  writeFile(omneSrc, "omne-bin");
  writeFile(appServerSrc, "app-server-bin");
  writeFile(path.join(pathDir, "rg"), "rg-bin");

  const stale = path.join(distOut, `vendor-bundle-${target}`, "stale.txt");
  writeFile(stale, "stale");

  const res = runBuild([
    "--target",
    target,
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
    "--clean",
  ]);

  assert.equal(res.status, 0, `stderr: ${res.stderr}`);
  assert.match(res.stdout, /built vendor bundle:/);
  assert.equal(fs.existsSync(stale), false);

  const bundleRoot = path.join(distOut, `vendor-bundle-${target}`);
  const manifestPath = path.join(bundleRoot, "manifest.json");
  assert.equal(fs.existsSync(manifestPath), true);

  const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
  assert.equal(manifest.schema_version, 1);
  assert.equal(manifest.target, target);
  assert.ok(Array.isArray(manifest.files));

  const listed = new Map(manifest.files.map((item) => [item.path, item]));
  assert.equal(listed.has("vendor/x86_64-unknown-linux-gnu/omne/omne"), true);
  assert.equal(
    listed.has("vendor/x86_64-unknown-linux-gnu/omne/omne-app-server"),
    true
  );
  assert.equal(listed.has("vendor/x86_64-unknown-linux-gnu/path/rg"), true);
  assert.equal(
    fs.readFileSync(
      path.join(bundleRoot, "vendor", target, "omne", "omne"),
      "utf8"
    ),
    "omne-bin"
  );
});

test("build-vendor-bundle fails when required args are missing", () => {
  const res = runBuild(["--target", "x86_64-unknown-linux-gnu"]);
  assert.notEqual(res.status, 0);
  assert.match(res.stderr, /--omne is required/);
  assert.match(res.stderr, /Usage:/);
});
