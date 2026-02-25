# @omne/omne-client

Minimal Node.js client for `omne-app-server` (JSON-RPC over stdio).

Local smoke (requires `omne-app-server` on PATH, or env override):

- `OMNE_APP_SERVER_BIN=target/debug/omne-app-server node ./packages/omne-client/examples/basic.mjs`
- `OMNE_APP_SERVER_BIN=target/debug/omne-app-server node ./packages/omne-client/examples/subscribe-resume.mjs`
- `OMNE_APP_SERVER_BIN=target/debug/omne-app-server node --test ./packages/omne-client/tests/*.test.mjs`

## Reconnect + Resume

`ThreadSubscribeStream` provides:

- `thread/subscribe` polling loop
- `since_seq` checkpoint tracking
- auto reconnect with backoff
- resume from latest confirmed `since_seq`
