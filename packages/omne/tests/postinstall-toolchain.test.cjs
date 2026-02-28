"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { test } = require("node:test");

const packageRoot = path.resolve(__dirname, "..");
const scriptPath = path.join(packageRoot, "scripts", "postinstall-toolchain.mjs");

function runPostinstall(env) {
  return spawnSync(process.execPath, [scriptPath], {
    env,
    encoding: "utf8",
  });
}

function makeFakeOmneRecorder(tmpDir, argsOutPath) {
  if (process.platform === "win32") {
    const cmdPath = path.join(tmpDir, "fake-omne.cmd");
    fs.writeFileSync(
      cmdPath,
      `@echo off\r\necho %* > "${argsOutPath.replaceAll("\\", "\\\\")}"\r\nexit /b 0\r\n`
    );
    return cmdPath;
  }

  const shPath = path.join(tmpDir, "fake-omne");
  fs.writeFileSync(
    shPath,
    `#!/bin/sh\nprintf '%s' "$*" > "${argsOutPath}"\nexit 0\n`
  );
  fs.chmodSync(shPath, 0o755);
  return shPath;
}

function makeFakeOmneFailure(tmpDir, exitCode) {
  if (process.platform === "win32") {
    const cmdPath = path.join(tmpDir, "fake-omne-fail.cmd");
    fs.writeFileSync(cmdPath, `@echo off\r\nexit /b ${exitCode}\r\n`);
    return cmdPath;
  }

  const shPath = path.join(tmpDir, "fake-omne-fail");
  fs.writeFileSync(shPath, `#!/bin/sh\nexit ${exitCode}\n`);
  fs.chmodSync(shPath, 0o755);
  return shPath;
}

test("postinstall forwards to `omne toolchain bootstrap`", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-postinstall-"));
  const argsOut = path.join(tmp, "args.txt");
  const fakeOmne = makeFakeOmneRecorder(tmp, argsOut);

  const res = runPostinstall({
    ...process.env,
    OMNE_PM_BIN: fakeOmne,
    PATH: process.env.PATH || "",
  });
  assert.equal(res.status, 0, `stderr: ${res.stderr}`);
  const args = fs.readFileSync(argsOut, "utf8");
  assert.match(args, /\btoolchain bootstrap\b/);
});

test("postinstall skips when omne binary is not found", () => {
  const missing = path.join(os.tmpdir(), `missing-omne-${Date.now()}`);
  const res = runPostinstall({
    ...process.env,
    OMNE_PM_BIN: missing,
    PATH: process.env.PATH || "",
  });
  assert.equal(res.status, 0, `stderr: ${res.stderr}`);
  assert.match(res.stdout, /skip: cannot find/);
});

test("postinstall returns non-zero in strict mode when bootstrap fails", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-postinstall-"));
  const fakeFail = makeFakeOmneFailure(tmp, 3);
  const res = runPostinstall({
    ...process.env,
    OMNE_PM_BIN: fakeFail,
    OMNE_TOOLCHAIN_BOOTSTRAP_STRICT: "1",
    PATH: process.env.PATH || "",
  });
  assert.equal(res.status, 3);
  assert.match(res.stdout, /bootstrap failed \(strict\)/);
});
