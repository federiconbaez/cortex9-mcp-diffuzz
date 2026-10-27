#!/usr/bin/env bash
# Line and region coverage of mcp-server-rs under three workloads:
#   conformance  official suite (@modelcontextprotocol/conformance, HTTP)
#   diffuzz      mcp-diffuzz against mcp-server-rs + rmcp-echo (stdio)
#   union        both together
#
# Needs: Node >= 22 (npx), rustup component llvm-tools, network for the
# conformance package and the mcp-server-rs clone.
#
# Usage: scripts/coverage.sh [DIFFUZZ_CASES] [SEED]
set -euo pipefail

CASES="${1:-300}"
SEED="${2:-1}"
CONFORMANCE_PKG="${CONFORMANCE_PKG:-@modelcontextprotocol/conformance@0.1.16}"
MSR_REPO="${MSR_REPO:-https://github.com/federiconbaez/mcp-server-rs}"
MSR_REV="${MSR_REV:-9da13ddb13a4f33870873ce1fe8d995a3819f1a8}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
COV="$ROOT/work/cov"
SRC="$COV/mcp-server-rs"
PROF="$COV/prof"

LLVM_BIN="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin"
PROFDATA="$LLVM_BIN/llvm-profdata"
LLVMCOV="$LLVM_BIN/llvm-cov"
[ -x "$PROFDATA" ] || { echo "llvm-tools missing: rustup component add llvm-tools" >&2; exit 1; }

rm -rf "$PROF" && mkdir -p "$PROF"
if [ ! -d "$SRC/.git" ]; then
  git clone --quiet "$MSR_REPO" "$SRC"
fi
git -C "$SRC" checkout --quiet --detach "$MSR_REV"
mkdir -p "$SRC/examples"
cp "$ROOT/scripts/http_cov.rs" "$SRC/examples/http_cov.rs"

export CARGO_TARGET_DIR="$COV/target"
RUSTFLAGS="-C instrument-coverage" cargo build --quiet \
  --manifest-path "$SRC/Cargo.toml" --bin mcp-server-rs --example http_cov
BIN="$CARGO_TARGET_DIR/debug/mcp-server-rs"
HTTP="$CARGO_TARGET_DIR/debug/examples/http_cov"
unset CARGO_TARGET_DIR
cargo build --quiet --release --manifest-path "$ROOT/Cargo.toml" -p rmcp-echo -p mcp-diffuzz

# 1. Official conformance suite over HTTP.
FIFO="$COV/http.stdin"; rm -f "$FIFO"; mkfifo "$FIFO"
LLVM_PROFILE_FILE="$PROF/conformance-%p-%m.profraw" "$HTTP" < "$FIFO" > "$COV/http.addr" &
HTTP_PID=$!
exec 3> "$FIFO"
for _ in $(seq 50); do [ -s "$COV/http.addr" ] && break; sleep 0.1; done
URL="$(head -n1 "$COV/http.addr")"
(cd "$COV" && npx -y "$CONFORMANCE_PKG" server --url "$URL") > "$COV/conformance.log" 2>&1 || true
exec 3>&-
wait "$HTTP_PID"

# 2. mcp-diffuzz over stdio (every child writes its own profile).
LLVM_PROFILE_FILE="$PROF/diffuzz-%p-%m.profraw" "$ROOT/target/release/mcp-diffuzz" fuzz \
  --target "mcp-server-rs=$BIN" --target "rmcp=$ROOT/target/release/rmcp-echo" \
  --seed "$SEED" --cases "$CASES" --no-minimize > "$COV/diffuzz-summary.json" 2> /dev/null

"$PROFDATA" merge -sparse "$PROF"/conformance-*.profraw -o "$COV/conformance.profdata"
"$PROFDATA" merge -sparse "$PROF"/diffuzz-*.profraw -o "$COV/diffuzz.profdata"
"$PROFDATA" merge -sparse "$COV/conformance.profdata" "$COV/diffuzz.profdata" -o "$COV/union.profdata"

IGNORE='(\.cargo|rustc|/examples/|/tests/)'
for w in conformance diffuzz union; do
  "$LLVMCOV" export -summary-only -instr-profile "$COV/$w.profdata" \
    -object "$BIN" -object "$HTTP" -ignore-filename-regex "$IGNORE" > "$COV/$w.json"
done

python3 - "$COV" <<'PY'
import json, sys, os
cov = sys.argv[1]
data = {w: json.load(open(os.path.join(cov, w + ".json")))["data"][0] for w in ("conformance", "diffuzz", "union")}
files = sorted({f["filename"].split("/src/", 1)[-1] for f in data["union"]["files"]})
core = {"jsonrpc.rs", "server.rs", "protocol.rs", "tools.rs"}
def pick(w, name):
    for f in data[w]["files"]:
        if f["filename"].endswith("/src/" + name):
            return f["summary"]
def pct(s, k):
    c = s[k]
    return 100.0 * c["covered"] / c["count"] if c["count"] else 0.0
def agg(w, names, k):
    tot = cov_ = 0
    for n in names:
        s = pick(w, n)
        if s: tot += s[k]["count"]; cov_ += s[k]["covered"]
    return 100.0 * cov_ / tot if tot else 0.0
rows = []
print("| file | conformance lines | diffuzz lines | union lines | conformance regions | diffuzz regions | union regions |")
print("|---|---|---|---|---|---|---|")
for n in files:
    vals = [pct(pick(w, n), "lines") for w in ("conformance", "diffuzz", "union")] + \
           [pct(pick(w, n), "regions") for w in ("conformance", "diffuzz", "union")]
    print("| " + n + " | " + " | ".join("%.1f%%" % v for v in vals) + " |")
for label, names in (("core (jsonrpc+server+protocol+tools)", sorted(core)), ("all files", files)):
    vals = [agg(w, names, "lines") for w in ("conformance", "diffuzz", "union")] + \
           [agg(w, names, "regions") for w in ("conformance", "diffuzz", "union")]
    print("| **" + label + "** | " + " | ".join("%.1f%%" % v for v in vals) + " |")
PY
echo
grep -E "Total|passed|failed" "$COV/conformance.log" | tail -n 5 || true
