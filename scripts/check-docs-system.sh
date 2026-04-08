#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"

require_file() {
  local path="$1"
  if [[ ! -f "$path" ]]; then
    echo "missing required file: ${path#$root/}" >&2
    exit 1
  fi
}

require_contains() {
  local path="$1"
  local needle="$2"
  if ! grep -Fq "$needle" "$path"; then
    echo "missing required text in ${path#$root/}: $needle" >&2
    exit 1
  fi
}

require_agents_length() {
  local path="$1"
  local max_lines="$2"
  local line_count
  line_count="$(wc -l < "$path")"
  if (( line_count > max_lines )); then
    echo "AGENTS too long (${line_count} > ${max_lines}): ${path#$root/}" >&2
    exit 1
  fi
}

require_file "$root/README.md"
require_file "$root/AGENTS.md"
require_file "$root/ARCHITECTURE.md"
require_file "$root/docs/README.md"
require_file "$root/docs/docs-system-map.md"
require_file "$root/docs/start.md"
require_file "$root/docs/omne_data.md"
require_file "$root/docs/modes.md"
require_file "$root/docs/approvals.md"
require_file "$root/docs/model_routing.md"
require_file "$root/docs/mcp.md"
require_file "$root/docs/permissions_matrix.md"
require_file "$root/docs/research/README.md"

require_contains "$root/README.md" "AGENTS.md"
require_contains "$root/README.md" "ARCHITECTURE.md"
require_contains "$root/README.md" "docs/README.md"
require_contains "$root/README.md" "docs/docs-system-map.md"
require_contains "$root/AGENTS.md" "README.md"
require_contains "$root/AGENTS.md" "ARCHITECTURE.md"
require_contains "$root/AGENTS.md" "docs/README.md"
require_contains "$root/AGENTS.md" "docs/docs-system-map.md"
require_contains "$root/docs/README.md" "../README.md"
require_contains "$root/docs/README.md" "../AGENTS.md"
require_contains "$root/docs/README.md" "../ARCHITECTURE.md"
require_contains "$root/docs/README.md" "docs/docs-system-map.md"
require_contains "$root/docs/docs-system-map.md" "ARCHITECTURE.md"
require_agents_length "$root/AGENTS.md" 120

echo "docs system check passed"
