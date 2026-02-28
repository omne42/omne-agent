"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { test } = require("node:test");

const packageRoot = path.resolve(__dirname, "..");
const scriptPath = path.join(packageRoot, "scripts", "assemble-vendor.mjs");

function runAssemble(args) {
  return spawnSync(process.execPath, [scriptPath, ...args], {
    encoding: "utf8",
  });
}

function writeFile(filePath, content = "x") {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, content);
}

test("assemble-vendor creates linux vendor layout and copies optional path dir", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-assemble-"));
  const srcRoot = path.join(tmp, "src");
  const outRoot = path.join(tmp, "vendor-out");
  const target = "x86_64-unknown-linux-gnu";

  const omneSrc = path.join(srcRoot, "omne");
  const appServerSrc = path.join(srcRoot, "omne-app-server");
  const pathSrc = path.join(srcRoot, "path-tools");
  writeFile(omneSrc, "omne-binary");
  writeFile(appServerSrc, "app-server-binary");
  writeFile(path.join(pathSrc, "rg"), "rg-binary");
  writeFile(path.join(pathSrc, "git"), "git-binary");
  writeFile(path.join(pathSrc, "gh"), "gh-binary");
  writeFile(path.join(pathSrc, "nested", "helper"), "helper-binary");

  const staleFile = path.join(outRoot, target, "obsolete.txt");
  writeFile(staleFile, "stale");

  const res = runAssemble([
    "--target",
    target,
    "--omne",
    omneSrc,
    "--app-server",
    appServerSrc,
    "--path-dir",
    pathSrc,
    "--out",
    outRoot,
    "--clean",
  ]);

  assert.equal(res.status, 0, `stderr: ${res.stderr}`);
  assert.match(res.stdout, /assembled vendor target:/);
  assert.equal(fs.existsSync(staleFile), false);

  const targetRoot = path.join(outRoot, target);
  assert.equal(
    fs.readFileSync(path.join(targetRoot, "omne", "omne"), "utf8"),
    "omne-binary"
  );
  assert.equal(
    fs.readFileSync(path.join(targetRoot, "omne", "omne-app-server"), "utf8"),
    "app-server-binary"
  );
  assert.equal(
    fs.readFileSync(path.join(targetRoot, "path", "rg"), "utf8"),
    "rg-binary"
  );
  assert.equal(
    fs.readFileSync(path.join(targetRoot, "path", "nested", "helper"), "utf8"),
    "helper-binary"
  );
  const features = JSON.parse(fs.readFileSync(path.join(targetRoot, "features.json"), "utf8"));
  assert.deepEqual(features.features, ["gh-cli", "git-cli"]);
});

test("assemble-vendor uses .exe names for windows targets", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-assemble-"));
  const outRoot = path.join(tmp, "vendor-out");
  const target = "x86_64-pc-windows-msvc";

  const omneSrc = path.join(tmp, "bin", "omne.exe");
  const appServerSrc = path.join(tmp, "bin", "omne-app-server.exe");
  writeFile(omneSrc, "win-omne");
  writeFile(appServerSrc, "win-app-server");

  const res = runAssemble([
    "--target",
    target,
    "--omne",
    omneSrc,
    "--app-server",
    appServerSrc,
    "--out",
    outRoot,
  ]);

  assert.equal(res.status, 0, `stderr: ${res.stderr}`);
  const targetRoot = path.join(outRoot, target, "omne");
  assert.equal(fs.readFileSync(path.join(targetRoot, "omne.exe"), "utf8"), "win-omne");
  assert.equal(
    fs.readFileSync(path.join(targetRoot, "omne-app-server.exe"), "utf8"),
    "win-app-server"
  );
});

test("assemble-vendor fails fast on missing required args", () => {
  const res = runAssemble(["--target", "x86_64-unknown-linux-gnu"]);
  assert.notEqual(res.status, 0);
  assert.match(res.stderr, /--omne is required/);
  assert.match(res.stderr, /Usage:/);
});

test("assemble-vendor can inject explicit git/gh tool binaries", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-assemble-"));
  const outRoot = path.join(tmp, "vendor-out");
  const target = "x86_64-unknown-linux-gnu";

  const omneSrc = path.join(tmp, "bin", "omne");
  const appServerSrc = path.join(tmp, "bin", "omne-app-server");
  const gitSrc = path.join(tmp, "bin", "git-custom");
  const ghSrc = path.join(tmp, "bin", "gh-custom");
  writeFile(omneSrc, "omne");
  writeFile(appServerSrc, "app-server");
  writeFile(gitSrc, "git-custom");
  writeFile(ghSrc, "gh-custom");

  const res = runAssemble([
    "--target",
    target,
    "--omne",
    omneSrc,
    "--app-server",
    appServerSrc,
    "--git-cli",
    gitSrc,
    "--gh-cli",
    ghSrc,
    "--out",
    outRoot,
  ]);
  assert.equal(res.status, 0, `stderr: ${res.stderr}`);
  const targetRoot = path.join(outRoot, target);
  assert.equal(fs.readFileSync(path.join(targetRoot, "path", "git"), "utf8"), "git-custom");
  assert.equal(fs.readFileSync(path.join(targetRoot, "path", "gh"), "utf8"), "gh-custom");
  const features = JSON.parse(fs.readFileSync(path.join(targetRoot, "features.json"), "utf8"));
  assert.deepEqual(features.features, ["gh-cli", "git-cli"]);
});
