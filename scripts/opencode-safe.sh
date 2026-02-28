#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
data_root="${OPENCODE_DATA_BASE:-$repo_root/.omne_data}"

export XDG_DATA_HOME="${XDG_DATA_HOME:-$data_root/data}"
export XDG_STATE_HOME="${XDG_STATE_HOME:-$data_root/state}"
export XDG_CACHE_HOME="${XDG_CACHE_HOME:-$data_root/cache}"
export XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-$data_root/config}"

mkdir -p "$XDG_DATA_HOME" "$XDG_STATE_HOME" "$XDG_CACHE_HOME" "$XDG_CONFIG_HOME"

exec opencode "$@"
