#!/usr/bin/env bash
# package-release.sh — build + package the release artefacts for ONE target triple.
#
#   bash scripts/package-release.sh <target-triple> [outdir]
#
# This script is the SINGLE owner of the release build + packaging: release.yml's
# build job calls it on a tag, and `just verify-release-parity` calls it at PR
# time — so the packaging the gate proves is byte-for-byte the packaging the tag
# runs. (release.yml executes only on tags — "a tag IS its first execution" — so
# a packaging body inlined there is untestable; this script is the seam that
# makes it testable.)
#
# Produces, in <outdir> (default: dist/):
#   kx-<triple>                       the FFI-free `kx` binary (release feature set)
#   kx-<triple>.sha256
#   kx-tools-<triple>.tar.gz          the 7 bundled tool binaries, flat:
#                                       kx-mcp-echo, kx-mcp-calc, kx-mcp-kv,
#                                       kx-connector-{gmail,discord,slack,notion}
#   kx-tools-<triple>.tar.gz.sha256
#
# The sha256 sidecars are written from INSIDE <outdir> so their rows carry the
# bare filename — install.sh's checksums.txt lookup matches on exactly that
# (`awk '$2 == a'`), and release.yml's publish glob (`cat kx-*.sha256`) folds
# both sidecars into checksums.txt unchanged.
#
# NOTE: <triple> is a NAMING LABEL, not a cross-compilation request — the build
# is native (release.yml runs this on a native runner per target), matching the
# pre-existing release pipeline exactly.
set -euo pipefail

TARGET="${1:?usage: package-release.sh <target-triple> [outdir]}"
OUTDIR="${2:-dist}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# The release feature set. This string is authoritative for the released
# artefact; the FFI/otel walls below assert on the SAME string so the two can
# never drift apart.
FEATURES="console,hnsw,serve-engine,hosted-apps"

# The bundled stdio tool bins. kx-mcp is built per-bin BY NAME: the crate also
# carries two failure-injection TEST servers (kx-mcp-mock-stdio,
# kx-mcp-bench-flaky) that must never ship, so no glob and no bare `--bins`.
MCP_BINS="kx-mcp-echo kx-mcp-calc kx-mcp-kv"
CONNECTOR_CRATES="kx-connector-gmail kx-connector-discord kx-connector-slack kx-connector-notion"
TOOL_BINS="$MCP_BINS $CONNECTOR_CRATES"

echo "== package-release: $TARGET → $OUTDIR (features: $FEATURES)"

# ---- 0. The FFI + observability walls, on the EXACT shipped feature set ------
# None of the release features may drag the llama.cpp FFI into the closure
# (serve-engine is the FFI-free Ollama backend; the in-process llama.cpp path is
# the opt-in `inference` build). And the release deliberately excludes the
# observability stack (kx-otel in THIS closure means the exclusion silently
# regressed). Extended to the tool/connector crates: they are FFI-free leaves
# and must stay that way.
for pkg_spec in "kx-cli --features $FEATURES" "kx-mcp" \
    "kx-connector-gmail" "kx-connector-discord" "kx-connector-slack" "kx-connector-notion"; do
    # pkg_spec is a deliberate word-split of "<pkg> [--features ...]":
    # shellcheck disable=SC2086
    closure="$(cargo tree -p $pkg_spec -e normal)"
    if echo "$closure" | grep -qE 'kx-llamacpp'; then
        echo " ✗ FAIL: the release closure of '$pkg_spec' linked the llama.cpp FFI" >&2
        exit 1
    fi
    if echo "$closure" | grep -qE 'kx-otel'; then
        echo " ✗ FAIL: the release closure of '$pkg_spec' linked kx-otel (observability must stay opt-in)" >&2
        exit 1
    fi
done
echo " ✓ walls — no llama.cpp FFI, no kx-otel, in any shipped closure"

# ---- 1. The web console dist (embedded into `kx` by the console feature) -----
# D139: the prebuilt binary ships the EMBEDDED WEB CONSOLE, so the TS SDK and
# the SPA are built first. Node is a release/packaging tool only; plain
# `cargo build -p kx-cli` stays node-free (feature off).
npm --prefix bindings/typescript ci
npm --prefix bindings/typescript run build
npm --prefix ui ci
npm --prefix ui run build

# ---- 2. Build ----------------------------------------------------------------
cargo build --release -p kx-cli --features "$FEATURES"
# Feature-free, by explicit bin name (see MCP_BINS note above). Bare `--features`
# with multiple `-p` selections is not a safe cargo edit, so the tool bins get
# their own invocations. The flag strings are derived from the SAME variables the
# staging/tar steps consume, so the built set and the shipped set cannot drift.
mcp_flags=""
for b in $MCP_BINS; do mcp_flags="$mcp_flags --bin $b"; done
# shellcheck disable=SC2086
cargo build --release -p kx-mcp $mcp_flags
connector_flags=""
for c in $CONNECTOR_CRATES; do connector_flags="$connector_flags -p $c"; done
# shellcheck disable=SC2086
cargo build --release $connector_flags

# ---- 3. Package --------------------------------------------------------------
mkdir -p "$OUTDIR"
cp "target/release/kx" "$OUTDIR/kx-$TARGET"
strip "$OUTDIR/kx-$TARGET" || true

TOOLS_STAGE="$(mktemp -d)"
trap 'rm -rf "$TOOLS_STAGE"' EXIT
for b in $TOOL_BINS; do
    cp "target/release/$b" "$TOOLS_STAGE/$b"
    strip "$TOOLS_STAGE/$b" || true
done
# A FLAT tarball (no directories): install.sh extracts straight into
# $KX_INSTALL_DIR beside `kx`, which is exactly where both resolution paths
# (sibling-of-exe for the connectors, bundled_binary_path's sibling arm for the
# kx-mcp bins) look.
tar -czf "$OUTDIR/kx-tools-$TARGET.tar.gz" -C "$TOOLS_STAGE" $TOOL_BINS

# ---- 4. Checksums (bare filenames — from INSIDE the outdir) ------------------
(
    cd "$OUTDIR"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "kx-$TARGET" > "kx-$TARGET.sha256"
        sha256sum "kx-tools-$TARGET.tar.gz" > "kx-tools-$TARGET.tar.gz.sha256"
    else
        shasum -a 256 "kx-$TARGET" > "kx-$TARGET.sha256"
        shasum -a 256 "kx-tools-$TARGET.tar.gz" > "kx-tools-$TARGET.tar.gz.sha256"
    fi
)

# ---- 5. The hard manifest — every expected artefact, or fail -----------------
# `upload-artifact`'s if-no-files-found:error only fires when NOTHING matches;
# a partially-failed package would ship the bare binary and re-create the
# original incident. This manifest is the fail-closed backstop.
fail=0
for f in "kx-$TARGET" "kx-$TARGET.sha256" "kx-tools-$TARGET.tar.gz" "kx-tools-$TARGET.tar.gz.sha256"; do
    if [ ! -f "$OUTDIR/$f" ]; then
        echo " ✗ MISSING artefact: $OUTDIR/$f" >&2
        fail=1
    fi
done
members="$(tar -tzf "$OUTDIR/kx-tools-$TARGET.tar.gz" | sort | tr '\n' ' ')"
expected="$(printf '%s\n' $TOOL_BINS | sort | tr '\n' ' ')"
if [ "$members" != "$expected" ]; then
    echo " ✗ tools tarball members mismatch:" >&2
    echo "   have: $members" >&2
    echo "   want: $expected" >&2
    fail=1
fi
[ "$fail" -eq 0 ] || exit 1

echo " ✓ package-release: $(find "$OUTDIR" -maxdepth 1 -name 'kx-*' -type f | wc -l | tr -d ' ') artefact files for $TARGET"
echo "   tools: $members"
