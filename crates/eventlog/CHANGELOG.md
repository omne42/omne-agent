# Changelog

## [Unreleased]

### Fixed

- Normalize reopened `events.jsonl` files so a complete last record without a trailing newline is repaired before the next append.
- Call `sync_data()` after appending an event so successful writes persist the JSONL record boundary more durably than `flush()` alone.
