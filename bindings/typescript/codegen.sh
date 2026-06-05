#!/usr/bin/env bash
# Regenerate the vendored protobuf + Connect stubs from the FROZEN proto contract.
#
# The single source of truth is `crates/kx-proto/proto/` (no vendored .proto
# copy). Output lands as a `kortecx.v1` subpackage under `src/gen/`. CI's
# `codegen-fresh` guard re-runs this and fails on any git diff.
#
# Requires the dev deps installed (`npm ci`): the buf CLI (`@bufbuild/buf`) and
# the ES plugin (`@bufbuild/protoc-gen-es`), both pinned by `package-lock.json`
# so this reproduces byte-identical stubs.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
cd "$here"

# Put the locally-installed plugin (`protoc-gen-es`) + buf on PATH.
export PATH="$here/node_modules/.bin:$PATH"

proto_root="$here/../../crates/kx-proto/proto"
if [ ! -d "$proto_root/kortecx/v1" ]; then
    echo "✗ proto root not found at $proto_root (run from the kortecx checkout)" >&2
    exit 1
fi

# Clean-generate so a removed message can never leave a stale file behind.
rm -rf "$here/src/gen"
mkdir -p "$here/src/gen"

buf generate

echo "✓ regenerated stubs into src/gen/kortecx/v1/"
