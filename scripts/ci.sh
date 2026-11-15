#!/usr/bin/env bash
# What CI runs, and what to run before a pull request: scripts/ci.sh
# Scrub check, clippy, tests. Offline once the crates are fetched (--locked). The tests use a
# stand-in model: no model server, no sound.
set -euo pipefail
cd "$(dirname "$0")/.."
scripts/scrub-check
# A throwaway home: nothing here may read a real config or play sound through a real setup.
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
T=$(mktemp -d "${TMPDIR:-/tmp}/lci.XXXXXX")
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/home" "$T/config"
export HOME="$T/home" XDG_CONFIG_HOME="$T/config"
for v in $(env | grep -oE '^LUMBRIDGE_SPEAK_[A-Za-z0-9_]*' || true); do unset "$v"; done
rustc --version
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
