# @codepm/pm

Thin Node.js launcher for CodePM Rust binaries.

Local smoke (requires `pm` / `pm-app-server` on PATH, or env overrides):

- `node ./packages/pm/bin/pm.js --help`
- `node ./packages/pm/bin/pm.js app-server --help`

Env overrides:

- `CODE_PM_PM_BIN=/abs/path/to/pm`
- `CODE_PM_APP_SERVER_BIN=/abs/path/to/pm-app-server`

