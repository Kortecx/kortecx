# syntax=docker/dockerfile:1
#
# kortecx — the FFI-free `kx` runtime, in a box.
#
# `kx` (the kx-cli binary) is FFI-FREE by DEFAULT: the workspace pins
# `kx-inference = { default-features = false }`, so this image needs NO C++
# toolchain, NO CUDA, and NO llama.cpp submodule — the exact property the
# `build-no-inference` CI gate proves. The image runs the local engine verbs
# (`kx run|replay|digest`) and the embedded single-system gateway (`kx serve`:
# gRPC + the live-event WebSocket).
#
# For LOCAL LLM inference (CPU), build `Dockerfile.inference` instead.
# GPU/CUDA is the cloud-side seam — see `Dockerfile.cuda` (D28: CUDA inference is
# cloud-tier; OSS ships the seam, not the build).
#
# Durable state lives in mounted volumes (the durability spine survives the
# container boundary — see docker-compose.yml):
#   /var/lib/kortecx/journal   the SQLite journal (sole-writer; exactly-once truth)
#   /var/lib/kortecx/content   the content-addressed store
#   /var/lib/kortecx/catalog   the durable signature/recipe catalog
#
#   build : docker build -t kortecx/kx:dev .
#   smoke : just docker-smoke           (in-container digest reproduction)
#   run   : docker run --rm kortecx/kx:dev run --journal /tmp/kx.db --content /tmp/c
#   serve : docker compose up           (see docker-compose.yml)

# ---- builder ---------------------------------------------------------------
# Pinned to the workspace toolchain channel (rust-toolchain.toml = 1.94.0). The
# official `rust` image ships rustup, so the exact pinned toolchain is honored on
# the first cargo invocation.
# HARDENING (Pass B): pin the base by @sha256 digest for a reproducible supply chain.
FROM rust:1.94-bookworm AS builder

WORKDIR /app

# The whole workspace, minus what `.dockerignore` strips (target/, .git/, the
# PRIVATE kx-cloud/, the llama.cpp submodule, local DBs + models).
COPY . .

# Build the FFI-free `kx`. NO submodules, NO clang/cmake. If the llama.cpp FFI ever
# leaked into kx-cli's normal dependency closure, this stage would suddenly need a
# C++ toolchain — `build-no-inference` catches that earlier, but this is a second
# line of defense.
RUN cargo build --release -p kx-cli \
 && strip target/release/kx \
 && cp target/release/kx /usr/local/bin/kx

# ---- runtime ---------------------------------------------------------------
# debian-slim: glibc matches the builder (no GLIBC-version drift for a glibc-linked
# binary), ships a shell (the in-container smoke + the CLI healthcheck want one) and
# ca-certificates (for the later TLS + model-fetch paths). Distroless is the
# hardening target (Pass B roadmap), traded off now because it has no shell.
FROM debian:bookworm-slim AS runtime

# ca-certificates: outbound TLS (future A1 + model fetch). tini: a tiny init that
# becomes PID 1 and FORWARDS SIGTERM to `kx serve` (+ reaps any children), so
# `docker stop` triggers the graceful drain rather than a SIGKILL.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates tini \
 && rm -rf /var/lib/apt/lists/*

# Non-root runtime identity. Create + own the durable-state dirs so a FRESH named
# volume mounted at any of them inherits uid:gid 10001 (a host BIND mount must be
# chowned by the operator — documented in the README).
RUN groupadd --gid 10001 kx \
 && useradd  --uid 10001 --gid 10001 --home-dir /var/lib/kortecx --shell /usr/sbin/nologin kx \
 && mkdir -p /var/lib/kortecx/journal /var/lib/kortecx/content /var/lib/kortecx/catalog \
 && chown -R 10001:10001 /var/lib/kortecx

COPY --from=builder /usr/local/bin/kx /usr/local/bin/kx

# Convention defaults. NOTE: `kx serve` reads FLAGS, not env — these are
# documentation + compose-interpolation only (the compose passes explicit flags).
ENV KX_JOURNAL=/var/lib/kortecx/journal/kx.db \
    KX_CONTENT=/var/lib/kortecx/content \
    KX_CATALOG=/var/lib/kortecx/catalog \
    RUST_LOG=info

# 50151 gRPC · 50152 live-event WebSocket. Declaring the state dirs as VOLUMEs
# makes them writable mounts even under `docker run --read-only` (so the read-only
# rootfs smoke works without explicit -v).
EXPOSE 50151 50152
VOLUME ["/var/lib/kortecx/journal", "/var/lib/kortecx/content", "/var/lib/kortecx/catalog"]

USER 10001:10001
WORKDIR /var/lib/kortecx

# tini as PID 1 → clean SIGTERM forwarding + zombie reaping. `kx` is the real arg0;
# `--help` is the default verb (override with `run`/`replay`/`digest`/`serve`/…).
ENTRYPOINT ["/usr/bin/tini", "--", "kx"]
CMD ["--help"]
