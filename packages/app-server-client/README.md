# omne-agent-app-server (Node client)

Minimal Node.js client for `omne-agent-app-server` (JSON-RPC over stdio).

Local smoke (requires `omne-agent-app-server` on PATH, or env override):

- `OMNE_AGENT_APP_SERVER_BIN=target/debug/omne-agent-app-server node ./packages/app-server-client/examples/basic.mjs`
