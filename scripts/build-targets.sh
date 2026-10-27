#!/usr/bin/env bash
# Build both differential targets and print the environment the regression
# tests need.
#
#   mcp-server-rs  github.com/federiconbaez/mcp-server-rs, pinned commit
#   rmcp           targets/rmcp-echo (official Rust SDK, rmcp =3.8.0)
set -euo pipefail

MSR_REPO="${MSR_REPO:-https://github.com/federiconbaez/mcp-server-rs}"
MSR_REV="${MSR_REV:-9da13ddb13a4f33870873ce1fe8d995a3819f1a8}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="$ROOT/work"
mkdir -p "$WORK"

if [ ! -d "$WORK/mcp-server-rs/.git" ]; then
  git clone --quiet "$MSR_REPO" "$WORK/mcp-server-rs"
fi
git -C "$WORK/mcp-server-rs" fetch --quiet origin "$MSR_REV" 2>/dev/null || true
git -C "$WORK/mcp-server-rs" checkout --quiet --detach "$MSR_REV"
cargo build --quiet --release --manifest-path "$WORK/mcp-server-rs/Cargo.toml"
cargo build --quiet --release --manifest-path "$ROOT/Cargo.toml" -p rmcp-echo -p mcp-diffuzz

echo "export DIFFUZZ_MCP_SERVER_RS=$WORK/mcp-server-rs/target/release/mcp-server-rs"
echo "export DIFFUZZ_RMCP=$ROOT/target/release/rmcp-echo"
