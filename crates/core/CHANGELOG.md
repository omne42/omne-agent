# Changelog

## [Unreleased]

### Fixed
- `ThreadStore::resume_thread` no longer synthesizes app-level recovery events in `omne-core`; app-server now owns abandoned turn/approval/tool recovery so the shared foundation layer stays at the storage/runtime boundary.
- `CommandHookRunner` now scrubs common provider API key env vars before spawning hook commands so host model secrets do not leak into hook subprocesses by default.
- Hook env-scrub regression tests now serialize environment mutation with an async-aware mutex so `cargo clippy -D warnings` passes on workspace tests.
- `ThreadStore::read_state` now reads `events.jsonl` directly into derived state instead of rebuilding and replaying a full event vector first.
