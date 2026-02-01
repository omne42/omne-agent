# omne-agent (Node launcher)

Thin Node.js launcher for Omne Agent Rust binaries.

Local smoke (requires `omne-agent` / `omne-agent-app-server` on PATH, or env overrides):

- `node ./packages/omne-agent/bin/omne-agent.js --help`
- `node ./packages/omne-agent/bin/omne-agent.js app-server --help`

Env overrides:

- `OMNE_AGENT_BIN=/abs/path/to/omne-agent`
- `OMNE_AGENT_APP_SERVER_BIN=/abs/path/to/omne-agent-app-server`
