# Changelog

## [Unreleased]

### Fixed
- `CommandHookRunner` now scrubs common provider API key env vars before spawning hook commands so host model secrets do not leak into hook subprocesses by default.
- Hook env-scrub regression tests now serialize environment mutation with an async-aware mutex so `cargo clippy -D warnings` passes on workspace tests.
