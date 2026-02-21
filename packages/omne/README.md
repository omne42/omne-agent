# @omne/omne

Thin Node.js launcher for OmneAgent Rust binaries.

Local smoke (requires `omne` / `omne-app-server` on PATH, or env overrides):

- `node ./packages/omne/bin/omne.js --help`
- `node ./packages/omne/bin/omne.js app-server --help`

Env overrides:

- `OMNE_PM_BIN=/abs/path/to/omne`
- `OMNE_APP_SERVER_BIN=/abs/path/to/omne-app-server`

