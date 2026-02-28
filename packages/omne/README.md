# @omne/omne

Thin Node.js launcher for OmneAgent Rust binaries.

Local smoke (requires `omne` / `omne-app-server` on PATH, or env overrides):

- `node ./packages/omne/bin/omne.js --help`
- `node ./packages/omne/bin/omne.js app-server --help`

Env overrides:

- `OMNE_PM_BIN=/abs/path/to/omne`
- `OMNE_APP_SERVER_BIN=/abs/path/to/omne-app-server`

Vendored layout (optional):

- `vendor/<target-triple>/omne/omne[.exe]`
- `vendor/<target-triple>/omne/omne-app-server[.exe]`
- `vendor/<target-triple>/path/` (optional extra PATH tools, auto-prepended when vendored binary is used)
- `vendor/<target-triple>/features.json` (bundled toolchain feature flags, e.g. `git-cli` / `gh-cli`)

Install-time toolchain bootstrap:

- Binary-first entrypoint: `omne toolchain bootstrap` (works without npm).
- `npm install` triggers `postinstall` script (`scripts/postinstall-toolchain.mjs`) which only forwards to:
  - `omne toolchain bootstrap`
- Bootstrap order:
  - system PATH
  - managed dir (`~/.omne/toolchain/<target-triple>/bin`, override: `OMNE_MANAGED_TOOLCHAIN_DIR`)
  - bundled vendor path (`git-cli` / `gh-cli` feature)
  - public upstream install (official GitHub release metadata/assets)
- Optional public mirror prefixes (public resources only):
  - `OMNE_TOOLCHAIN_MIRROR_PREFIXES` (comma-separated; each prefix is prepended to the canonical URL)
  - `OMNE_TOOLCHAIN_GITHUB_API_BASES` (comma-separated API base list, default `https://api.github.com`)
  - `OMNE_TOOLCHAIN_HTTP_TIMEOUT_SECONDS` (default: `15`)
- Launcher appends this managed directory to child-process PATH.

Assemble vendor tree:

- `node ./packages/omne/scripts/assemble-vendor.mjs --target x86_64-unknown-linux-gnu --omne ./target/debug/omne --app-server ./target/debug/omne-app-server --clean`
- Optional bundled CLI injection:
  - `--git-cli <path-to-git-binary>`
  - `--gh-cli <path-to-gh-binary>`
  - Or place `git`/`gh` directly under `--path-dir` and features will be auto-detected.

Build distributable vendor bundle (with `manifest.json`):

- `node ./packages/omne/scripts/build-vendor-bundle.mjs --target x86_64-unknown-linux-gnu --omne ./target/debug/omne --app-server ./target/debug/omne-app-server --clean`

Verify bundle integrity from `manifest.json`:

- `node ./packages/omne/scripts/verify-vendor-bundle.mjs --bundle ./packages/omne/dist/vendor-bundle-x86_64-unknown-linux-gnu`

Create versioned release output (bundle + `RELEASE.json` + `SHA256SUMS`):

- `node ./packages/omne/scripts/release-vendor-bundle.mjs --target x86_64-unknown-linux-gnu --version v0.3.0-test --omne ./target/debug/omne --app-server ./target/debug/omne-app-server --clean`
- Optional bundled CLI injection is supported through the same flags:
  - `--git-cli <path-to-git-binary>`
  - `--gh-cli <path-to-gh-binary>`

Host-target release driver (auto-resolve binaries from local `target/`):

- `node ./packages/omne/scripts/release-host-vendor-bundle.mjs --target x86_64-unknown-linux-gnu --target-dir ./target --profile debug --clean`
  - Optional `--version`; if omitted, auto-generated as `<package-version>-dev.<UTC timestamp>`.

One-shot local release command (runs host release and prints final artifact paths):

- `node ./packages/omne/scripts/release-local-vendor-bundle.mjs --target x86_64-unknown-linux-gnu --target-dir ./target --profile debug --clean`

Multi-target matrix release (one command for several triples):

- `node ./packages/omne/scripts/release-matrix-vendor-bundle.mjs --version v0.3.0-test --targets x86_64-unknown-linux-gnu,aarch64-unknown-linux-gnu --target-dir ./target --profile debug --clean`
- If `--targets` is omitted, defaults to:
  `x86_64-unknown-linux-gnu,aarch64-unknown-linux-gnu,x86_64-apple-darwin,aarch64-apple-darwin,x86_64-pc-windows-msvc,aarch64-pc-windows-msvc`
- `node ./packages/omne/scripts/release-matrix-vendor-bundle.mjs --version v0.3.0-test --target-dir ./target --profile debug --clean`
  - Writes `dist/releases/last-run.json` summary and updates `dist/releases/index.json`.

Rebuild release index (`dist/releases/index.json`):

- `node ./packages/omne/scripts/update-release-index.mjs --release-out ./packages/omne/dist/releases`

Validate:

- `npm --prefix ./packages/omne run check`
- `npm --prefix ./packages/omne test`

CI (minimal host release rehearsal):

- `.github/workflows/omne-node-vendor.yml`
- Runs Linux/macOS/Windows host matrix: `packages/omne` check/test, host `omne` + `omne-app-server` build, `release-local-vendor-bundle`, then uploads `packages/omne/dist/releases`.
- `workflow_dispatch` inputs: `profile=debug|release`, optional `version`, and `clean=true|false`.
- Pushing a `v*` tag triggers release publishing: uses tag name as release version, validates downloaded artifact structure via `scripts/validate-tag-release-artifacts.mjs` (`index.json` schema, per-target uniqueness, exact target-set match between `index.json` and `vendor-bundle-<tag>-*`, `release_dir`/bundle-name consistency, `RELEASE.json` `version/target` consistency, `SHA256SUMS`), then packages tag-matched payload via `scripts/package-tag-release-assets.mjs` (only `index.json` + `vendor-bundle-<tag>-*`, requiring exact target-set match against index entries) into per-OS tarballs (`omne-vendor-releases-<os>.tar.gz`) with top-level `SHA256SUMS`, and finally runs `scripts/verify-tag-release-tarballs.mjs` to ensure tarballs contain only expected entries and are internally self-consistent (`index.json` schema/version targets, target-set match vs bundle dirs, and per-bundle `RELEASE.json` `version/target` consistency).
- `scripts/package-tag-release-assets.mjs` cleans output directories before packaging to avoid stale tarballs/payload leakage between runs.
