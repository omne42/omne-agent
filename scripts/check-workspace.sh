#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
mode="${1:-local}"

run_docs_system() {
  "$root/scripts/check-docs-system.sh"
}

run_rust_gates() {
  cargo fmt --all -- --check
  cargo check --workspace --all-targets
  cargo test --workspace
  cargo clippy --workspace --all-targets --all-features -- -D warnings
}

cd "$root"

case "$mode" in
  local|ci)
    run_docs_system
    run_rust_gates
    ;;
  docs-system)
    run_docs_system
    ;;
  rust)
    run_rust_gates
    ;;
  *)
    echo "unknown mode: $mode" >&2
    echo "usage: $0 [local|ci|docs-system|rust]" >&2
    exit 1
    ;;
esac
