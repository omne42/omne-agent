"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { test } = require("node:test");

const packageRoot = path.resolve(__dirname, "..");
const scriptPath = path.join(packageRoot, "scripts", "update-release-index.mjs");

function runIndex(args) {
  return spawnSync(process.execPath, [scriptPath, ...args], {
    encoding: "utf8",
  });
}

function writeJson(filePath, value) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`);
}

test("update-release-index builds sorted index from release dirs", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "omne-release-index-"));
  const releaseOut = path.join(tmp, "releases");

  const older = path.join(releaseOut, "vendor-bundle-v0.1.0-x86_64-unknown-linux-gnu");
  const newer = path.join(releaseOut, "vendor-bundle-v0.2.0-x86_64-unknown-linux-gnu");
  writeJson(path.join(older, "RELEASE.json"), {
    version: "v0.1.0",
    target: "x86_64-unknown-linux-gnu",
    created_at: "2026-01-01T00:00:00.000Z",
    source_bundle_dir: "/tmp/src-older",
  });
  writeJson(path.join(older, "manifest.json"), {
    files: [{ path: "a", size: 1, sha256: "x" }],
  });
  writeJson(path.join(newer, "RELEASE.json"), {
    version: "v0.2.0",
    target: "x86_64-unknown-linux-gnu",
    created_at: "2026-02-01T00:00:00.000Z",
    source_bundle_dir: "/tmp/src-newer",
  });
  writeJson(path.join(newer, "manifest.json"), {
    files: [{ path: "a" }, { path: "b" }],
  });

  const res = runIndex(["--release-out", releaseOut]);
  assert.equal(res.status, 0, `stderr: ${res.stderr}`);
  assert.match(res.stdout, /release index updated:/);

  const index = JSON.parse(fs.readFileSync(path.join(releaseOut, "index.json"), "utf8"));
  assert.ok(Array.isArray(index.releases));
  assert.equal(index.releases.length, 2);
  assert.equal(index.releases[0].version, "v0.2.0");
  assert.equal(index.releases[0].file_count, 2);
  assert.equal(index.releases[1].version, "v0.1.0");
  assert.equal(index.releases[1].file_count, 1);
});
