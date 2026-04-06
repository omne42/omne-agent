# Changelog

## [Unreleased]

### Fixed
- `CommandHookRunner` now scrubs common provider API key env vars before spawning hook commands so host model secrets do not leak into hook subprocesses by default.
