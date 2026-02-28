"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { test } = require("node:test");

const packageRoot = path.resolve(__dirname, "..");
const scriptPath = path.join(packageRoot, "scripts", "postinstall-toolchain.mjs");

function writeFile(filePath, content = "x") {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, content);
}

function writeExecutable(filePath, content) {
  writeFile(filePath, content);
  fs.chmodSync(filePath, 0o755);
}

function runPostinstall(env) {
  return spawnSync(process.execPath, [scriptPath], {
    env,
    encoding: "utf8",
  });
}

test("postinstall installs bundled git when missing and feature is enabled", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-postinstall-"));
  const fakePackageRoot = path.join(tmp, "pkg");
  const target = "x86_64-unknown-linux-gnu";

  writeFile(
    path.join(fakePackageRoot, "vendor", target, "features.json"),
    JSON.stringify(
      {
        schema_version: 1,
        target,
        features: ["git-cli"],
      },
      null,
      2
    )
  );
  writeExecutable(path.join(fakePackageRoot, "vendor", target, "path", "git"), "#!/bin/sh\nexit 0\n");

  const homeDir = path.join(tmp, "home");
  const res = runPostinstall({
    ...process.env,
    OMNE_PACKAGE_ROOT: fakePackageRoot,
    OMNE_TARGET_TRIPLE: target,
    HOME: homeDir,
    PATH: "/nonexistent",
  });
  assert.equal(res.status, 0, `stderr: ${res.stderr}`);
  assert.match(res.stdout, /\[omne postinstall\] git: installed_bundled/);
  const installedGit = path.join(homeDir, ".omne", "toolchain", target, "bin", "git");
  assert.equal(fs.existsSync(installedGit), true);
});

test("postinstall keeps existing git and does not require bundled feature", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-postinstall-"));
  const fakePackageRoot = path.join(tmp, "pkg");
  const target = "x86_64-unknown-linux-gnu";
  const fakeBin = path.join(tmp, "bin");

  writeFile(
    path.join(fakePackageRoot, "vendor", target, "features.json"),
    JSON.stringify(
      {
        schema_version: 1,
        target,
        features: [],
      },
      null,
      2
    )
  );
  writeExecutable(path.join(fakeBin, "git"), "#!/bin/sh\nexit 0\n");

  const homeDir = path.join(tmp, "home");
  const res = runPostinstall({
    ...process.env,
    OMNE_PACKAGE_ROOT: fakePackageRoot,
    OMNE_TARGET_TRIPLE: target,
    HOME: homeDir,
    PATH: fakeBin,
  });
  assert.equal(res.status, 0, `stderr: ${res.stderr}`);
  assert.match(res.stdout, /\[omne postinstall\] git: present/);
  const installedGit = path.join(homeDir, ".omne", "toolchain", target, "bin", "git");
  assert.equal(fs.existsSync(installedGit), false);
});
