# Omne-Agent Entropy Reduction Todo (Ordered)

1. Extract test modules from runtime mega-files first (start with `subagents.rs`, then `watch_inbox.rs`).
   Goal: reduce runtime file cognitive load without behavior change.
2. Split `subagents.rs` runtime into bounded modules (`schedule_core`, `approval_proxy`, `artifacts`, `workspace_apply`, `runtime_io`).
   Goal: isolate state machine from IO side-effects.
3. Unify fan-out execution paths in CLI (`run_workflow_fan_out` and `FanOutScheduler`) into one scheduler model.
   Goal: remove duplicate scheduling logic and branching behavior.
4. Replace high-risk `include!()` aggregators with proper `mod` boundaries (start with `agent-cli/main/command`, then `app-server/thread_observe`).
   Goal: improve ownership clarity and reduce hidden cross-file coupling.
5. Consolidate protocol denied/approval response types in `app-server-protocol` via shared base types.
   Goal: reduce repeated structs and generated artifact churn.
6. Move fan-in summary parsing/derivation to shared Rust runtime crate used by both app-server and agent-cli.
   Goal: remove duplicated parsing and summary rules.
7. Standardize app-server test harness usage and delete duplicated local `build_test_server` implementations.
   Goal: simplify test maintenance and fixture evolution.
8. Tighten protocol generation workflow and commit policy for generated files.
   Goal: reduce PR noise while preserving reproducibility.

## Execution Rule
- Complete each item in order; each item must pass `cargo fmt --all`, `cargo check`, and `cargo test` (or scoped equivalent when full test is too slow).
