#!/usr/bin/env bash
# Regenerate the vendored protobuf + gRPC stubs from the FROZEN proto contract.
#
# The single source of truth is `crates/kx-proto/proto/` — there is NO vendored
# copy of the .proto files, so the stubs can never drift from a stale copy. CI's
# `codegen-fresh` guard re-runs this and fails on any git diff.
#
# Output: `src/kortecx/v1/{coordinator,gateway}_pb2.py(i)` + `gateway_pb2_grpc.py`.
# Because the proto package is `kortecx.v1`, generating into `src/` lands the
# stubs as a `kortecx.v1` subpackage of the SDK, so the generated
# `from kortecx.v1 import …` imports resolve natively — no import rewriting.
#
# Requires the PINNED grpcio-tools (ships its own protoc): the `dev` extra pins
# `grpcio-tools==1.67.1` (protobuf 5.27 gencode) so this reproduces byte-identical
# stubs — `uv pip install -e '.[dev]'` or `pip install grpcio-tools==1.67.1`.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
proto_root="$here/../../crates/kx-proto/proto"
out="$here/src"

if [ ! -d "$proto_root/kortecx/v1" ]; then
    echo "✗ proto root not found at $proto_root (run from the kortecx checkout)" >&2
    exit 1
fi

python -m grpc_tools.protoc \
    -I "$proto_root" \
    --python_out="$out" \
    --pyi_out="$out" \
    --grpc_python_out="$out" \
    kortecx/v1/coordinator.proto \
    kortecx/v1/gateway.proto

# `coordinator.proto` carries the shared value messages the gateway imports, but
# its Coordinator SERVICE is the internal coordinator/worker plane — not part of
# a client SDK. Drop the generated internal service stub (the _pb2 messages stay).
rm -f "$out/kortecx/v1/coordinator_pb2_grpc.py"

# Make the generated tree an importable package.
touch "$out/kortecx/v1/__init__.py"

echo "✓ regenerated stubs into src/kortecx/v1/"
