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
# TWO ways to get `kx` into the image (selected with --build-arg KX_SOURCE=…):
#   builder  (DEFAULT) — compile from source with cargo-chef dependency-layer
#                        caching, so source-only rebuilds skip recompiling deps.
#                        Self-contained: needs nothing but this build context.
#   prebuilt (FAST)    — COPY the SHA-256-verified `kx` from the GitHub Release
#                        instead of compiling (near-zero build). Requires
#                        --build-arg KX_VERSION=<release tag>. FFI-free-only,
#                        exactly matching what the Release (A0) publishes.
#
# Durable state lives in mounted volumes (the durability spine survives the
# container boundary — see docker-compose.yml):
#   /var/lib/kortecx/journal   the SQLite journal (sole-writer; exactly-once truth)
#   /var/lib/kortecx/content   the content-addressed store
#   /var/lib/kortecx/catalog   the durable signature/recipe catalog
#
#   build (chef)     : docker build -t kortecx/kx:dev .
#   build (prebuilt) : docker build --build-arg KX_SOURCE=prebuilt \
#                        --build-arg KX_VERSION=v0.1.0 -t kortecx/kx:dev .
#   smoke            : just docker-smoke    (in-container digest reproduction)
#   run              : docker run --rm kortecx/kx:dev run --journal /tmp/kx.db --content /tmp/c
#   serve            : docker compose up    (see docker-compose.yml)

# Global ARG (declared before the first FROM so it can drive `FROM ${KX_SOURCE}`).
# DEFAULT = the self-contained chef builder. `prebuilt` switches to the Release
# download. Only the SELECTED stage is built — the other never runs.
ARG KX_SOURCE=builder

# ---- chef base: cargo-chef for dependency-layer caching --------------------
# Pinned to the workspace toolchain channel (rust-toolchain.toml = 1.94.0). The
# official `rust` image ships rustup, so the exact pinned toolchain is honored on
# the first cargo invocation. cargo-chef is `cargo install`ed from this same pinned
# image (no external base-image dependency — supply-chain minimalism, Rule 6).
# HARDENING (Pass B): pin the base by @sha256 digest for a reproducible supply chain.
FROM rust:1.94-bookworm AS chef
RUN cargo install cargo-chef --locked
WORKDIR /app

# ---- planner: distill the dependency graph into recipe.json -----------------
# recipe.json is a function of the Cargo manifests + lockfile ONLY, so the cooked
# layer below is reused across every source-only change.
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ---- builder (DEFAULT): cook deps, then build the FFI-free `kx` -------------
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Cook ONLY kx-cli's dependency closure. This layer is cached unless the manifests
# / lockfile change — the ~3-5 min dependency-compile win on a typical rebuild. The
# BuildKit cache mounts further persist the cargo registry + target dir locally.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo chef cook --release --recipe-path recipe.json -p kx-cli
# Now the app sources. NO submodules, NO clang/cmake. If the llama.cpp FFI ever
# leaked into kx-cli's normal dependency closure, this stage would suddenly need a
# C++ toolchain — `build-no-inference` catches that earlier, this is a second line
# of defense. `cp` out of the (cache-mounted) target so the binary lands in a layer.
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release -p kx-cli \
 && strip target/release/kx \
 && cp target/release/kx /usr/local/bin/kx

# ---- prebuilt (FAST variant): COPY the verified `kx` from the GitHub Release ---
# Compiles NOTHING. Downloads the per-target binary + its `.sha256` sidecar and
# verifies the bytes before installing. The Release (A0) ships only the FFI-free
# `kx`, so this path is FFI-free-only — exactly this image. Not built unless
# --build-arg KX_SOURCE=prebuilt is set.
FROM debian:bookworm-slim AS prebuilt
ARG KX_REPO=Kortecx/kortecx
ARG KX_VERSION
ARG TARGETARCH
RUN apt-get update \
 && apt-get install -y --no-install-recommends curl ca-certificates \
 && rm -rf /var/lib/apt/lists/*
RUN set -eux; \
    : "${KX_VERSION:?set --build-arg KX_VERSION=<release tag> for the prebuilt variant}"; \
    case "$TARGETARCH" in \
      amd64) triple=x86_64-unknown-linux-gnu ;; \
      arm64) triple=aarch64-unknown-linux-gnu ;; \
      *) echo "prebuilt variant: unsupported TARGETARCH=${TARGETARCH:-<unset>}" >&2; exit 1 ;; \
    esac; \
    base="https://github.com/${KX_REPO}/releases/download/${KX_VERSION}"; \
    curl -fsSL -o /tmp/kx        "${base}/kx-${triple}"; \
    curl -fsSL -o /tmp/kx.sha256 "${base}/kx-${triple}.sha256"; \
    printf '%s  /tmp/kx\n' "$(awk '{print $1}' /tmp/kx.sha256)" | sha256sum -c -; \
    install -m 0755 /tmp/kx /usr/local/bin/kx

# ---- kx-bin: the selected source of the `kx` binary ------------------------
# Aliases either `builder` (default) or `prebuilt`. The runtime copies from here,
# so BuildKit only ever builds the one the operator chose.
FROM ${KX_SOURCE} AS kx-bin

# ---- runtime ---------------------------------------------------------------
# debian-slim: glibc matches the builder (no GLIBC-version drift for a glibc-linked
# binary), ships a shell (the in-container smoke + the CLI healthcheck want one) and
# ca-certificates (for the later TLS + model-fetch paths). Distroless is the
# hardening target (Pass B roadmap) — evaluated and DEFERRED here: it has no shell
# and no tini, and the smoke harness + operational debugging both want one. A
# distroless variant is its own hardening PR.
FROM debian:bookworm-slim AS runtime

# ca-certificates: outbound TLS (A1 + model fetch). tini: a tiny init that becomes
# PID 1 and FORWARDS SIGTERM to `kx serve` (+ reaps any children), so `docker stop`
# triggers the graceful drain rather than a SIGKILL.
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

COPY --from=kx-bin /usr/local/bin/kx /usr/local/bin/kx

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
