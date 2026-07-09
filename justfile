# kortecx build orchestration.
# All commands assume the workspace root (this directory).
#
# Locally:  `just check`   — dev quick sweep (fmt + clippy + test)
#           `just ci`       — exact mirror of CI gates
#           `just deny`     — RustSec + license + bans + sources
#           `just doctor`   — preflight check (toolchain + C++ deps + submodule)

# Default recipe — show available commands.
default:
    @just --list

# ============================================================================
# Dev workflow recipes
# ============================================================================

# Quick local sweep — fmt + clippy + test. The recipe a developer runs
# before pushing. Fast subset of `ci`; skips deny/doc/ffi-link/reproducible.
check: fmt-check clippy test

# Exact mirror of the CI workflow's gates (in dependency order). Runs every
# job .github/workflows/ci.yml runs in parallel, here sequentially. Modify
# this recipe in lock-step with ci.yml.
ci: fmt-check clippy test eval deny doc ffi-link build-no-inference build-serve-engine features-guard check-reproducible scale-smoke test-connector-real test-skill registry-check

# Run the SDK CI gates locally — the exact `sdk-python` + `sdk-typescript` jobs from ci.yml
# (codegen-fresh, ruff+mypy / biome+tsc, and the unit/contract tests). Run before an SDK PR:
# the SDK gates are NOT part of `just ci` (Rust-only), so a Rust-only local pass is a false-green
# for a formatting/typing miss. Assumes the dev extras are installed (`uv pip install -e
# 'bindings/python[dev]'` + `npm ci` in bindings/typescript). Keep in lock-step with ci.yml.
pre-push-sdk:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "▶ sdk-python — codegen-fresh + ruff + mypy + pytest"
    ( cd bindings/python && ./codegen.sh >/dev/null \
        && { git diff --quiet -- src/kortecx/v1 || { echo "✗ python stubs drifted — commit codegen.sh output"; exit 1; }; } \
        && python -m ruff check . \
        && python -m ruff format --check src tests examples \
        && python -m mypy src \
        && python -m pytest -q )
    echo "▶ sdk-typescript — codegen-fresh + biome + tsc + vitest"
    ( cd bindings/typescript && ./codegen.sh >/dev/null \
        && { git diff --quiet -- src/gen || { echo "✗ TS stubs drifted — commit codegen.sh output"; exit 1; }; } \
        && npx biome ci . \
        && npx tsc --noEmit \
        && npx vitest run )
    echo "✓ SDK gates green"

# Run the UI CI gates locally — the `ui` job from ci.yml minus Playwright (biome + tsc + vitest +
# build + bundle-size). Run before a UI PR. Assumes `npm ci` was run in ui/ (and the TS SDK built).
# `npx playwright test` is CI-only here (needs a browser + a live serve). Keep in lock-step with ci.yml.
pre-push-ui:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "▶ ui — biome + tsc + vitest + build + size"
    ( cd ui && npx biome ci . && npx tsc --noEmit && npx vitest run && npm run build && npm run size )
    echo "✓ UI gates green (run \`cd ui && npx playwright test\` for the e2e — CI does this)"

# Verify code is formatted per rustfmt.toml. Fails on any drift.
fmt-check:
    cargo fmt --all -- --check

# Apply formatting in-place. Use locally; CI runs fmt-check instead.
fmt:
    cargo fmt --all

# Lint workspace with clippy; deny warnings (no allowed unwrap/expect in library code).
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Build all workspace crates.
build:
    cargo build --workspace --all-targets

# Run all workspace tests.
test:
    cargo test --workspace -- --nocapture

# Build documentation; deny warnings (catches broken intra-doc links).
doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

# ============================================================================
# Connector / Extension SDK (D167 E0) conformance
# ============================================================================

# Dial a connector through the real gateway path + run the D167 Extension
# Acceptance Gate subset (items 3/5/7/10). No endpoint → the bundled reference
# connector (kx-connector-example).
#   just test-connector                            # the reference connector
#   just test-connector ./my-mcp-server --flag     # a stdio server + args
#   just test-connector https://mcp.example/rpc    # a Streamable-HTTP server
test-connector *endpoint:
    cargo build -q -p kx-extension-sdk
    cargo run -q -p kx-extension-sdk --example conformance -- {{endpoint}}

# The connector-conformance HARD GATE: dial a pinned REAL third-party MCP server
# (the official filesystem server) and run the gate. `npm ci` restores the EXACT
# committed lockfile (the only network op — cached in CI); the dial + every
# assertion then run OFFLINE over a stdio subprocess, so no network flakiness
# enters the gate (GR12). Run as a required CI check + part of `ci`.
test-connector-real:
    #!/usr/bin/env bash
    set -euo pipefail
    FIX="crates/kx-extension-sdk/tests/fixtures/real-connector"
    echo "Installing the pinned real MCP server (npm ci, deterministic)…"
    npm ci --prefix "$FIX"
    BIN="$FIX/node_modules/.bin/mcp-server-filesystem"
    ROOT="$(mktemp -d)"
    echo "hello from kortecx" > "$ROOT/note.txt"
    echo "Dialing $BIN (offline) through the conformance harness…"
    cargo run -q -p kx-extension-sdk --example conformance -- "$BIN" "$ROOT"

# The LIVE agentic tool-calling drive over a freshly-registered connector. Locally
# validate on BOTH engines (Ollama + llama.cpp) with Gemma-4 (GR24); in CI it rides
# the real-model-e2e job (Qwen3-0.6B). #[ignore]'d; needs a GGUF.
test-connector-live: fetch-gemma-model
    cargo test -p kx-gateway --features inference react_serve_connector -- --ignored --nocapture

# The DECLARATIVE-family conformance gate over kortecx.skill/v1 packs.
# No args ⇒ every in-tree skills/** reference pack; pass dirs to gate your own
# (external authors run this before submitting). Offline, no model, <1s.
test-skill *packs:
    cargo run -q -p kx-extension-sdk --example skill_conformance -- {{packs}}

# The registry-consistency check — registry/index.json entries must
# agree with the tree (skills/** ⟷ skill entries, integrations/kx-connector-*
# ⟷ integration entries, sources exist, ledger ids real). Pure file reads.
registry-check:
    @[ -f scripts/registry-check.sh ] && bash scripts/registry-check.sh || echo "registry-check: helper script not present — skipped"

# ============================================================================
# Onboarding / install automation (sudo-free, opt-in)
# ============================================================================

# One-shot onboarding for the FFI-FREE runtime (Tier 0). Builds + installs the
# `kx` binary with NO C++ toolchain and NO llama.cpp submodule. NEVER runs
# sudo / brew / apt. Opt into local inference separately (`just setup-inference`).
#   just setup            # install `kx` to ~/.cargo/bin
#   INSTALL=0 just setup  # just `cargo build --release -p kx-cli` (no install)
setup:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "kortecx setup — FFI-free runtime (Tier 0: Rust only, no C++ toolchain)"
    # Install the repo's local git hooks if present (see `just install-hooks`).
    [ -d scripts/hooks ] && git config core.hooksPath scripts/hooks 2>/dev/null && chmod +x scripts/hooks/* 2>/dev/null || true
    if ! command -v cargo >/dev/null 2>&1; then
        echo " ✗ cargo not found — install Rust from https://rustup.rs, then re-run." >&2
        exit 1
    fi
    if [ "${INSTALL:-1}" = "1" ]; then
        echo "Installing the kx binary (cargo install --path crates/kx-cli)..."
        cargo install --path crates/kx-cli
        BIN="kx"
    else
        echo "Building the kx binary (cargo build --release -p kx-cli)..."
        cargo build --release -p kx-cli
        BIN="./target/release/kx"
    fi
    echo ""
    echo " ✓ ready. Next steps:"
    echo "     ${BIN} run    --journal /tmp/kx.db --content /tmp/kx-content"
    echo "     ${BIN} replay --journal /tmp/kx.db --content /tmp/kx-content"
    echo "     ${BIN} serve  --dev-allow-local                # zero-config: auto data dir under ~/.kortecx"
    echo "     ${BIN} serve  --dev-allow-local --journal /tmp/kx.db --content /tmp/kx-content  # or pin the paths"
    echo "       (zero-config serve prints a startup banner with the resolved journal/content/catalog +"
    echo "        the gRPC/console endpoints; set KX_DATA_DIR to relocate the base data dir.)"
    echo ""
    echo "For REAL local LLM inference (opt-in; needs a C++ toolchain), run:"
    echo "     just doctor   # per-OS install hints   →   just setup-inference"

# install-hooks — install the repo's local git hooks (if present) via core.hooksPath. The hooks run
# quick pre-push quality checks. core.hooksPath is per-clone local config that cannot self-install,
# so run this once per checkout (the `setup` recipe also runs it when the hooks are present).
# Emergency bypass of a hook: `git push --no-verify`.
install-hooks:
    @if [ -d scripts/hooks ]; then \
        git config core.hooksPath scripts/hooks && chmod +x scripts/hooks/* && \
        echo "hooks installed: core.hooksPath → scripts/hooks ($(ls scripts/hooks | tr '\n' ' '))"; \
     else echo "install-hooks: no scripts/hooks/ in this repo — nothing to install"; fi

# OPT-IN inference tier (Tier 1). Inits the llama.cpp submodule and builds the FFI
# link. Requires a C++ toolchain — run `just doctor` first for per-OS hints.
# Deliberately separate from `setup` so the default flow never triggers CMake.
setup-inference:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Initializing the llama.cpp submodule (crates/kx-llamacpp-sys/llama.cpp)..."
    git submodule update --init --recursive crates/kx-llamacpp-sys/llama.cpp
    echo "Building the FFI link (runs CMake on the submodule)..."
    cargo build -p kx-llamacpp-sys --release
    cargo build -p kx-llamacpp --release
    echo " ✓ inference tier ready. Try:  just fetch-demo-model  &&  just smoke-test-with-model"

# Download the tiny demo GGUF (stories260K, ~1.2 MB) to target/models/ with
# SHA-256 verification. Idempotent: skips if already present + valid. Triggers NO
# build (does not enable the model-smoke-test feature). Feed it to the examples:
#   cargo run -p kx-llamacpp --example generate -- target/models/stories260K.gguf "Once upon a time"
fetch-demo-model:
    #!/usr/bin/env bash
    set -euo pipefail
    URL="https://huggingface.co/ggml-org/models/resolve/main/tinyllamas/stories260K.gguf"
    SHA="270cba1bd5109f42d03350f60406024560464db173c0e387d91f0426d3bd256d"
    DEST="target/models/stories260K.gguf"
    mkdir -p target/models
    if [ -f "$DEST" ] && [ "$(shasum -a 256 "$DEST" | cut -d' ' -f1)" = "$SHA" ]; then
        echo " ✓ demo model already present + verified: $DEST"
        exit 0
    fi
    echo "Downloading $URL → $DEST ..."
    rm -f "$DEST" "$DEST.partial"
    if   command -v curl >/dev/null 2>&1; then curl -fsSL "$URL" -o "$DEST.partial"
    elif command -v wget >/dev/null 2>&1; then wget -q "$URL" -O "$DEST.partial"
    else echo " ✗ neither curl nor wget found on PATH" >&2; exit 1; fi
    GOT="$(shasum -a 256 "$DEST.partial" | cut -d' ' -f1)"
    if [ "$GOT" != "$SHA" ]; then
        echo " ✗ SHA-256 mismatch: expected $SHA, got $GOT" >&2
        rm -f "$DEST.partial"; exit 1
    fi
    mv "$DEST.partial" "$DEST"
    echo " ✓ verified + saved: $DEST"

# Download a PUBLIC Qwen3 stand-in GGUF (unsloth Qwen3-0.6B-Q4_K_M, ~397 MB) to
# target/models/ with SHA-256 verification — the drop-in test vehicle for the
# Qwen3-4B agent campaign until the private finetune ships. Same `qwen3` arch +
# ChatML + native tool-calling shape as the real model. Idempotent.
#
# Then point the harness at it:
#   export KX_MODEL_NAME=qwen3-0.6b
#   export KX_MODEL_HARNESS_GGUF="$(pwd)/target/models/qwen3-0.6b-q4_k_m.gguf"
#
# For the REAL (private) Qwen3-4B q4_k_m, override the source — a SHA is REQUIRED
# (never fetch a model unverified); a gated repo also needs HF_TOKEN:
#   KX_AGENT_MODEL_URL=<hf-resolve-url> KX_AGENT_MODEL_SHA=<sha256> \
#     KX_AGENT_MODEL_DEST=target/models/qwen3-4b-q4_k_m.gguf HF_TOKEN=<tok> just fetch-agent-model
fetch-agent-model:
    #!/usr/bin/env bash
    set -euo pipefail
    URL="${KX_AGENT_MODEL_URL:-https://huggingface.co/unsloth/Qwen3-0.6B-GGUF/resolve/main/Qwen3-0.6B-Q4_K_M.gguf}"
    SHA="${KX_AGENT_MODEL_SHA:-ac2d97712095a558e31573f62f466a3f9d93990898b0ec79d7c974c1780d524a}"
    DEST="${KX_AGENT_MODEL_DEST:-target/models/qwen3-0.6b-q4_k_m.gguf}"
    mkdir -p "$(dirname "$DEST")"
    if [ -f "$DEST" ] && [ "$(shasum -a 256 "$DEST" | cut -d' ' -f1)" = "$SHA" ]; then
        echo " ✓ agent stand-in already present + verified: $DEST"
        exit 0
    fi
    echo "Downloading $URL → $DEST ..."
    rm -f "$DEST" "$DEST.partial"
    # Token-aware fetch (no bash arrays — macOS ships bash 3.2 where an empty
    # `"${arr[@]}"` trips `set -u`).
    if command -v curl >/dev/null 2>&1; then
        if [ -n "${HF_TOKEN:-}" ]; then
            curl -fsSL -H "Authorization: Bearer ${HF_TOKEN}" "$URL" -o "$DEST.partial"
        else
            curl -fsSL "$URL" -o "$DEST.partial"
        fi
    elif command -v wget >/dev/null 2>&1; then
        if [ -n "${HF_TOKEN:-}" ]; then
            wget -q --header="Authorization: Bearer ${HF_TOKEN}" "$URL" -O "$DEST.partial"
        else
            wget -q "$URL" -O "$DEST.partial"
        fi
    else
        echo " ✗ neither curl nor wget found on PATH" >&2; exit 1
    fi
    GOT="$(shasum -a 256 "$DEST.partial" | cut -d' ' -f1)"
    if [ "$GOT" != "$SHA" ]; then
        echo " ✗ SHA-256 mismatch: expected $SHA, got $GOT" >&2
        echo "   (overriding the model? pass the matching KX_AGENT_MODEL_SHA)" >&2
        rm -f "$DEST.partial"; exit 1
    fi
    mv "$DEST.partial" "$DEST"
    echo " ✓ verified + saved: $DEST"
    echo "   Point the harness at it:"
    echo "     export KX_MODEL_NAME=qwen3-0.6b"
    echo "     export KX_MODEL_HARNESS_GGUF=\"$(pwd)/$DEST\""

# Download the Gemma-4-12B omni model (Q4_K_M GGUF ~7.1 GB + its vision mmproj
# ~175 MB) from unsloth to target/models/, SHA-256-verified + idempotent. This is
# the LOCAL / CLOUD real-test + benchmark model going forward (D158); CI stays on
# the tiny public Qwen3-0.6B (`fetch-agent-model`). REQUIRES the gemma4uv-capable
# llama.cpp pin (>= d2462f8f7; see crates/kx-llamacpp-sys/PIN.md). Override the
# quant via KX_GEMMA_MODEL_{URL,SHA,DEST} (a SHA is REQUIRED — never unverified).
fetch-gemma-model:
    #!/usr/bin/env bash
    set -euo pipefail
    BASE="https://huggingface.co/unsloth/gemma-4-12b-it-GGUF/resolve/main"
    dl() {
      url="$1"; dest="$2"; sha="$3"
      if [ -f "$dest" ] && [ "$(shasum -a 256 "$dest" | cut -d' ' -f1)" = "$sha" ]; then
        echo " ✓ already present + verified: $dest"; return 0
      fi
      echo "Downloading $url → $dest ..."
      rm -f "$dest" "$dest.partial"
      if [ -n "${HF_TOKEN:-}" ]; then
        curl -fsSL -H "Authorization: Bearer ${HF_TOKEN}" "$url" -o "$dest.partial"
      else
        curl -fsSL "$url" -o "$dest.partial"
      fi
      got="$(shasum -a 256 "$dest.partial" | cut -d' ' -f1)"
      if [ "$got" != "$sha" ]; then
        echo " ✗ SHA-256 mismatch for $dest: expected $sha got $got" >&2
        rm -f "$dest.partial"; exit 1
      fi
      mv "$dest.partial" "$dest"; echo " ✓ verified + saved: $dest"
    }
    mkdir -p target/models
    dl "${KX_GEMMA_MMPROJ_URL:-$BASE/mmproj-F16.gguf}" \
       "${KX_GEMMA_MMPROJ_DEST:-target/models/gemma-4-12b-it-mmproj-f16.gguf}" \
       "${KX_GEMMA_MMPROJ_SHA:-91f086971e56d7a7d8d39e271873fccdb49541bd259d6e02c401a4f1cb7a219e}"
    dl "${KX_GEMMA_MODEL_URL:-$BASE/gemma-4-12b-it-Q4_K_M.gguf}" \
       "${KX_GEMMA_MODEL_DEST:-target/models/gemma-4-12b-it-q4_k_m.gguf}" \
       "${KX_GEMMA_MODEL_SHA:-43fec98c5102b1c446b4ddd0a9439f1db3a2e1f2e0b8cd143ce1ea619a9403d6}"
    echo "   Serve it (text + vision): just review-serve-gemma"

# POC-3 (MODELS-LOCAL-LIFECYCLE): fetch a SECOND, small, different-family model so
# the local multi-model routing / load / offload / swap path can be driven LIVE on
# this 16 GB box (Gemma-4-12B ~7 GB + Qwen2.5-3B ~2 GB co-reside under capacity-2).
# Qwen2.5-3B-Instruct (Apache-2.0) emits the Hermes `<tool_call>{…}</tool_call>`
# format ≠ Gemma's native `call:NAME{…}` — so two REAL models exercise the
# multi-format tool-call parser. NOT the CI Qwen3-0.6B (too weak to tool-call).
# A SHA is REQUIRED (never an unverified download); override via KX_MODEL2_{URL,SHA,DEST}.
fetch-2nd-model:
    #!/usr/bin/env bash
    set -euo pipefail
    URL="${KX_MODEL2_URL:-https://huggingface.co/bartowski/Qwen2.5-3B-Instruct-GGUF/resolve/main/Qwen2.5-3B-Instruct-Q4_K_M.gguf}"
    DEST="${KX_MODEL2_DEST:-target/models/qwen2.5-3b-instruct-q4_k_m.gguf}"
    SHA="${KX_MODEL2_SHA:-9c9f56a391a3abbd5b89d0245bf6106081bcc3173119d4229235dd9d23253f94}"
    mkdir -p target/models
    if [ -f "$DEST" ]; then
      got="$(shasum -a 256 "$DEST" | cut -d' ' -f1)"
      if [ -z "$SHA" ] || [ "$got" = "$SHA" ]; then
        echo " ✓ already present: $DEST (sha256 $got)"; exit 0
      fi
    fi
    echo "Downloading $URL → $DEST ..."
    rm -f "$DEST" "$DEST.partial"
    if [ -n "${HF_TOKEN:-}" ]; then
      curl -fsSL -H "Authorization: Bearer ${HF_TOKEN}" "$URL" -o "$DEST.partial"
    else
      curl -fsSL "$URL" -o "$DEST.partial"
    fi
    got="$(shasum -a 256 "$DEST.partial" | cut -d' ' -f1)"
    if [ -n "$SHA" ] && [ "$got" != "$SHA" ]; then
      echo " ✗ SHA-256 mismatch: expected $SHA got $got" >&2; rm -f "$DEST.partial"; exit 1
    fi
    mv "$DEST.partial" "$DEST"
    echo " ✓ saved: $DEST (sha256 $got)"
    echo "   Serve BOTH (primary Gemma + this): KX_SERVE_MODEL_GGUF=<abs gemma> KX_SERVE_MODELS=<abs $DEST> just review-serve-gemma"

# The ONE-COMMAND inference serve (§2.194 guardrail): fetch the stand-in model
# if absent (idempotent, checksum-verified), then start `kx serve` with it +
# the embedded console at :8888. Model paths are DETERMINISTIC
# (target/models/ via the fetch recipes, or the KX_AGENT_MODEL_* overrides) —
# never hunt the filesystem for GGUFs. Journal/content are PINNED under
# target/serve/ here for a repeatable, inspectable layout (override:
# `just serve-inference /path/kx.db /path/blobs`). NOTE: bare `kx serve
# --dev-allow-local` is now zero-config (auto paths under ~/.kortecx); we keep
# the explicit paths here so the inference demo's state lives beside the repo.
serve-inference journal="target/serve/kx.db" content="target/serve/blobs": fetch-agent-model console-dist
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "$(dirname "{{journal}}")" "{{content}}"
    export KX_SERVE_MODEL_GGUF="${KX_AGENT_MODEL_DEST:-$(pwd)/target/models/qwen3-0.6b-q4_k_m.gguf}"
    echo " ▶ inference serve (model: $KX_SERVE_MODEL_GGUF)"
    cargo run --release -p kx-cli --features inference,hnsw,console --bin kx -- \
        serve --journal "{{journal}}" --content "{{content}}" --dev-allow-local

# THE PR-REVIEW serve (§2.208 + §2.194 + GR15 guardrail). Guarantees a reviewer
# sees a FRESH console + REAL (non-echo) chat — closing the three reviewer failure
# modes (STALE BINARY · STALE UI EMBED · NO MODEL → echo). Rebuilds ui/dist
# (console-dist) → builds kx with inference+console → frees the console port (kills
# any stale kx on :8888) → serves WITH the fetched model → HARD-verifies: (a) the
# served console index byte-matches the just-built ui/dist [stale-embed catch;
# relies on the console serving raw `include_bytes!` bytes, no Content-Encoding],
# (b) ListModels is non-empty [a model is loaded ⇒ chat uses kx/recipes/chat, NOT
# echo], (c) a known-prompt completion is real + non-echo. Fails LOUDLY on any.
# Use this for EVERY PR-review console (never a bare `kx serve`, never a
# separately-built long-lived target/release/kx — `check-reproducible` /
# `verify-quickstart` clobber it to a console-less FFI-free binary).
review-serve journal="target/serve/kx.db" content="target/serve/blobs": fetch-agent-model console-dist
    #!/usr/bin/env bash
    set -euo pipefail
    PORT=8888; GRPC="http://127.0.0.1:50151"
    MODEL="${KX_AGENT_MODEL_DEST:-$(pwd)/target/models/qwen3-0.6b-q4_k_m.gguf}"
    test -f "$MODEL" || { echo " ✗ model GGUF missing: $MODEL" >&2; exit 1; }
    cargo build --release -p kx-cli --features inference,hnsw,console --bin kx
    KX="$(pwd)/target/release/kx"
    PIDS="$(lsof -ti tcp:$PORT 2>/dev/null || true)"
    [ -n "$PIDS" ] && { echo " ! killing stale process on :$PORT ($PIDS)"; kill $PIDS 2>/dev/null || true; sleep 1; }
    mkdir -p "$(dirname "{{journal}}")" "{{content}}"
    export KX_SERVE_MODEL_GGUF="$MODEL"
    echo " ▶ review serve (model: $KX_SERVE_MODEL_GGUF)"
    "$KX" serve --journal "{{journal}}" --content "{{content}}" --dev-allow-local &
    SERVE_PID=$!; trap 'kill $SERVE_PID 2>/dev/null || true' EXIT
    for i in $(seq 1 60); do curl -fsS "http://127.0.0.1:$PORT/" -o /dev/null 2>/dev/null && break; sleep 1; done
    # (a) stale-embed catch — served console index == the just-built ui/dist.
    SERVED="$(curl -fsS "http://127.0.0.1:$PORT/" | shasum -a 256 | cut -d' ' -f1)"
    DISK="$(shasum -a 256 ui/dist/index.html | cut -d' ' -f1)"
    [ "$SERVED" = "$DISK" ] || { echo " ✗ STALE EMBED: served $SERVED != ui/dist $DISK — rebuild"; exit 1; }
    echo " ✓ fresh console embed ($SERVED)"
    # (b) a model is loaded ⇒ chat is the real chat recipe, NOT echo.
    "$KX" models list --endpoint "$GRPC" --json \
      | python3 -c "import json,sys; sys.exit(0 if len(json.load(sys.stdin).get('models',[]))>=1 else 1)" \
      || { echo " ✗ NO MODEL: ListModels empty — chat would echo. Set KX_SERVE_MODEL_GGUF."; exit 1; }
    echo " ✓ model loaded (ListModels non-empty)"
    # (c) a known-prompt completion is REAL + non-echo (the model is answering).
    PROMPT="Reply with only the digit: what is 2+2?"
    OUT="$("$KX" invoke kx/recipes/chat --args "{\"prompt\":\"$PROMPT\"}" --wait --json --endpoint "$GRPC" \
      | python3 -c "import json,sys; print(json.load(sys.stdin).get('result_utf8',''))" 2>/dev/null || true)"
    [ -n "$OUT" ] || { echo " ✗ chat returned no committed result — model not answering."; exit 1; }
    case "$OUT" in *"$PROMPT"*) echo " ✗ ECHO DETECTED: chat returned the prompt verbatim."; exit 1;; esac
    echo " ✓ real non-echo completion: $(printf '%s' "$OUT" | tr '\n' ' ' | head -c 70)"
    echo ""
    echo " ✅ REVIEW SERVE READY — console http://127.0.0.1:$PORT/  ·  connect $GRPC"
    echo "    (review in BOTH themes; chat returns a real <think>+answer, never echo)"
    wait $SERVE_PID

# The Gemma-4-12B omni PR-review serve (TEXT + VISION) — the `review-serve` twin
# for the local/cloud real-test model (D158). Same three guardrails (FRESH embed ·
# model loaded · REAL non-echo completion). Sets KX_SERVE_MMPROJ_GGUF so the
# vision recipe is provisioned (the gemma4uv projector; needs the PIN.md bump).
# 12B loads slower than the stand-in, so the readiness wait is longer. Review in
# BOTH themes; chat returns a real answer (Gemma's reasoning channel is collapsed
# by the model-agnostic templating, never echo).
review-serve-gemma journal="target/serve/kx.db" content="target/serve/blobs": fetch-gemma-model console-dist
    #!/usr/bin/env bash
    set -euo pipefail
    PORT=8888; GRPC="http://127.0.0.1:50151"
    MODEL="${KX_GEMMA_MODEL_DEST:-$(pwd)/target/models/gemma-4-12b-it-q4_k_m.gguf}"
    MMPROJ="${KX_GEMMA_MMPROJ_DEST:-$(pwd)/target/models/gemma-4-12b-it-mmproj-f16.gguf}"
    test -f "$MODEL" || { echo " ✗ model GGUF missing: $MODEL" >&2; exit 1; }
    test -f "$MMPROJ" || { echo " ✗ mmproj GGUF missing: $MMPROJ" >&2; exit 1; }
    cargo build --release -p kx-cli --features inference,hnsw,console --bin kx
    KX="$(pwd)/target/release/kx"
    PIDS="$(lsof -ti tcp:$PORT 2>/dev/null || true)"
    [ -n "$PIDS" ] && { echo " ! killing stale process on :$PORT ($PIDS)"; kill $PIDS 2>/dev/null || true; sleep 1; }
    mkdir -p "$(dirname "{{journal}}")" "{{content}}"
    export KX_SERVE_MODEL_GGUF="$MODEL" KX_SERVE_MMPROJ_GGUF="$MMPROJ"
    echo " ▶ Gemma review serve (model: $MODEL  +  vision mmproj: $MMPROJ)"
    "$KX" serve --journal "{{journal}}" --content "{{content}}" --dev-allow-local &
    SERVE_PID=$!; trap 'kill $SERVE_PID 2>/dev/null || true' EXIT
    for i in $(seq 1 120); do curl -fsS "http://127.0.0.1:$PORT/" -o /dev/null 2>/dev/null && break; sleep 1; done
    SERVED="$(curl -fsS "http://127.0.0.1:$PORT/" | shasum -a 256 | cut -d' ' -f1)"
    DISK="$(shasum -a 256 ui/dist/index.html | cut -d' ' -f1)"
    [ "$SERVED" = "$DISK" ] || { echo " ✗ STALE EMBED: served $SERVED != ui/dist $DISK — rebuild"; exit 1; }
    echo " ✓ fresh console embed ($SERVED)"
    "$KX" models list --endpoint "$GRPC" --json \
      | python3 -c "import json,sys; sys.exit(0 if len(json.load(sys.stdin).get('models',[]))>=1 else 1)" \
      || { echo " ✗ NO MODEL: ListModels empty — chat would echo. Set KX_SERVE_MODEL_GGUF."; exit 1; }
    echo " ✓ model loaded (ListModels non-empty)"
    PROMPT="Reply with only the digit: what is 2+2?"
    OUT="$("$KX" invoke kx/recipes/chat --args "{\"prompt\":\"$PROMPT\"}" --wait --json --endpoint "$GRPC" \
      | python3 -c "import json,sys; print(json.load(sys.stdin).get('result_utf8',''))" 2>/dev/null || true)"
    [ -n "$OUT" ] || { echo " ✗ chat returned no committed result — model not answering."; exit 1; }
    case "$OUT" in *"$PROMPT"*) echo " ✗ ECHO DETECTED: chat returned the prompt verbatim."; exit 1;; esac
    echo " ✓ real non-echo Gemma completion: $(printf '%s' "$OUT" | tr '\n' ' ' | head -c 70)"
    echo ""
    echo " ✅ GEMMA REVIEW SERVE READY — console http://127.0.0.1:$PORT/  ·  connect $GRPC"
    echo "    (text + vision; review in BOTH themes; chat returns a real answer, never echo)"
    wait $SERVE_PID

# D139: build the web console's embed input — the TS SDK (the UI's file: dep)
# then the SPA production bundle into ui/dist. The exact sequence release.yml
# runs; needed BEFORE any `--features console` cargo build (build.rs embeds the
# dist at compile time and fails loudly when it's missing). Needs node >= 22.
console-dist:
    npm --prefix bindings/typescript ci
    npm --prefix bindings/typescript run build
    npm --prefix ui ci
    npm --prefix ui run build

# D139: a release-grade kx with the embedded console + the Datasets data-plane —
# exactly what the prebuilt release ships (`--features console,hnsw`; FFI-free).
console-build: console-dist
    cargo build --release -p kx-cli --features console,hnsw

# Docs-as-test gate: run the README quickstart end to end and assert the canonical
# projection digest. Builds the FFI-free `kx` binary (no C++ toolchain) and drives
# run → crash → replay → digest over temp dirs, asserting the canonical digest
# (8/8 committed) at every step. Cleans up. Fails LOUDLY on any drift — this is the
# gate that keeps the README honest. NOT part of `just ci` (a separate, fast gate).
verify-quickstart:
    #!/usr/bin/env bash
    set -euo pipefail
    CANON="7d22d4bdfc6f68a4311f40b20f3fe7c67f4c5d2b352f3bff8722b439e94a5af9"
    echo "Building the FFI-free kx binary..."
    cargo build --release -p kx-cli
    KX="{{justfile_directory()}}/target/release/kx"
    WORK="$(mktemp -d)"
    trap 'rm -rf "$WORK"' EXIT
    J="$WORK/kx.db"; C="$WORK/kx-content"

    echo "[1/3] clean run → digest"
    RUN_OUT="$("$KX" run --journal "$J" --content "$C")"
    echo "    $RUN_OUT"
    if [ "${RUN_OUT%% *}" != "$CANON" ]; then
        echo " ✗ FAIL: clean-run digest ${RUN_OUT%% *} != canonical $CANON" >&2; exit 1
    fi
    case "$RUN_OUT" in *"(8/8 committed)"*) ;; *)
        echo " ✗ FAIL: expected 8/8 committed, got: $RUN_OUT" >&2; exit 1 ;; esac

    echo "[2/3] fresh crash run (aborts mid-commit; non-zero is expected)"
    rm -f "$J"; rm -rf "$C"
    set +e
    "$KX" run --journal "$J" --content "$C" --crash-at post-commit-vtc >/dev/null 2>&1
    set -e

    echo "[3/3] replay (recover) → digest, then a standalone digest fold"
    REPLAY_OUT="$("$KX" replay --journal "$J" --content "$C")"
    echo "    $REPLAY_OUT"
    DIGEST_ONLY="$("$KX" digest --journal "$J" --content "$C")"
    if [ "${REPLAY_OUT%% *}" != "$CANON" ]; then
        echo " ✗ FAIL: replay digest ${REPLAY_OUT%% *} != canonical $CANON" >&2; exit 1
    fi
    if [ "$DIGEST_ONLY" != "$CANON" ]; then
        echo " ✗ FAIL: standalone digest $DIGEST_ONLY != canonical $CANON" >&2; exit 1
    fi
    echo ""
    echo " ✓ verify-quickstart PASS — clean run, crash-then-replay, and a fresh"
    echo "   digest fold all produce the canonical digest (8/8 committed)."

# ============================================================================
# Container (Docker / OCI) recipes
# ============================================================================

# Build the FFI-free `kx` runtime image (no C++ toolchain, no llama.cpp submodule).
# Compiles from source via cargo-chef (dependency layer cached across source-only
# rebuilds). Tags `kortecx/kx:dev` by default; override with KX_IMAGE.
#   just docker-build                       # → kortecx/kx:dev
#   KX_IMAGE=ghcr.io/you/kx:v0 just docker-build
docker-build:
    DOCKER_BUILDKIT=1 docker build -f Dockerfile -t {{ env_var_or_default("KX_IMAGE", "kortecx/kx:dev") }} .

# Build the FFI-free image WITHOUT compiling — COPY the SHA-256-verified `kx` from
# the GitHub Release (near-zero build). Requires KX_VERSION (the Release tag).
#   KX_VERSION=v0.1.0 just docker-build-fast
docker-build-fast:
    DOCKER_BUILDKIT=1 docker build -f Dockerfile \
      --build-arg KX_SOURCE=prebuilt \
      --build-arg KX_VERSION={{ env_var_or_default("KX_VERSION", "") }} \
      -t {{ env_var_or_default("KX_IMAGE", "kortecx/kx:dev") }} .

# Build the CPU inference image (fetches the PINNED llama.cpp inside the builder;
# needs a C++ toolchain in the BUILDER only, not at runtime). CPU-only — Metal is
# macOS-host-only and GPU/CUDA is the cloud-side seam (Dockerfile.cuda, D28).
docker-build-inference:
    DOCKER_BUILDKIT=1 docker build -f Dockerfile.inference -t {{ env_var_or_default("KX_IMAGE", "kortecx/kx:inference-cpu") }} .

# In-container docs-as-test: build the FFI-free image, then reproduce the canonical
# projection digest THROUGH the container (clean run · crash-then-replay over a
# persisted volume · read-only rootfs). The Docker analog of `verify-quickstart`,
# and what the CI `docker-smoke` job runs. Requires a working Docker daemon. NOT
# part of `just ci` (a separate, Docker-dependent gate — like verify-quickstart).
docker-smoke:
    @[ -f scripts/docker-smoke.sh ] && ./scripts/docker-smoke.sh || echo "docker-smoke: helper script not present — skipped"

# ============================================================================
# Policy + supply-chain recipes
# ============================================================================

# cargo-deny: RustSec advisories + license allowlist + bans + sources.
# Configuration lives in `deny.toml` (Item 4 of the repo-baseline sweep).
# CI runs the same command in the `deny` job — keep them in lock-step.
deny:
    cargo deny check

# ============================================================================
# C++ FFI recipes — the load-bearing native-build path
# ============================================================================

# Explicit C++ FFI link path: builds `kx-llamacpp-sys` (the bindgen + CMake
# layer) and `kx-llamacpp` (the safe wrapper that LINKS against the static
# archives). Mirrors the CI `ffi-link` job. Local runs use the existing
# target/ cache; CI does NOT cache target/ for this job so a broken native
# link cannot hide. Use `just clean && just ffi-link` locally to reproduce
# the CI environment.
ffi-link:
    cargo build -p kx-llamacpp-sys --release
    cargo build -p kx-llamacpp --release

# Prove the runtime installs/builds with NO native FFI in its dependency closure
# (no C++ toolchain, no llama.cpp submodule). Step 1.1 of the OSS Adoption & Trust
# track: `cargo install kx-runtime` must not require a C++ build. Asserts
# kx-llamacpp{,-sys} is absent from the runtime's normal dependency tree, then
# builds the runtime + the feature-off inference lib (the bring-your-own-backend
# path). CI runs this on a runner WITHOUT a C++ toolchain or submodule, so a
# regression that leaks the FFI back into the runtime closure fails loudly.
build-no-inference:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Asserting kx-runtime's dependency closure is FFI-free..."
    if cargo tree -p kx-runtime -e normal | grep -qE 'kx-llamacpp'; then
        echo " ✗ FAIL: kx-llamacpp is in kx-runtime's dependency closure (the FFI leaked into the runtime)"
        cargo tree -p kx-runtime -e normal | grep -E 'kx-llamacpp' || true
        exit 1
    fi
    echo " ✓ no kx-llamacpp in kx-runtime closure"
    echo "Asserting kx-cli's dependency closure is FFI-free (the user-facing binary installs without a C++ toolchain)..."
    if cargo tree -p kx-cli -e normal | grep -qE 'kx-llamacpp'; then
        echo " ✗ FAIL: kx-llamacpp is in kx-cli's dependency closure (the FFI leaked into the CLI)"
        cargo tree -p kx-cli -e normal | grep -E 'kx-llamacpp' || true
        exit 1
    fi
    echo " ✓ no kx-llamacpp in kx-cli closure"
    cargo build -p kx-runtime
    cargo build -p kx-cli
    cargo build -p kx-inference --no-default-features
    echo " ✓ build-no-inference: PASS"

# PR-A.1 / GR24 (dual-engine parity): prove the prebuilt release feature set
# (`console,hnsw,serve-engine`) serves local models via Ollama with NO C++
# toolchain. Asserts the EXACT shipped graph pulls no llama.cpp FFI, then compiles
# the FFI-relevant serve loop (`serve-engine,hnsw`). `console` is skipped in the
# build (it needs a node `ui/dist`, built by the `ui` job) but IS covered by the
# cargo-tree scan. CI runs this on a runner WITHOUT a C++ toolchain or submodule,
# so a regression that leaks the FFI into the serve-engine closure fails loudly.
# The `inference-checks` serve-engine step runs WITH the toolchain; this is the
# clean-room FFI-FREE complement.
build-serve-engine:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Asserting kx-cli --features console,hnsw,serve-engine stays FFI-free (the prebuilt artifact)..."
    if cargo tree -p kx-cli --features console,hnsw,serve-engine -e normal | grep -qE 'kx-llamacpp'; then
        echo " ✗ FAIL: console,hnsw,serve-engine dragged the FFI into kx-cli"
        cargo tree -p kx-cli --features console,hnsw,serve-engine -e normal | grep -E 'kx-llamacpp' || true
        exit 1
    fi
    echo " ✓ no kx-llamacpp in the prebuilt (console,hnsw,serve-engine) closure"
    echo "Asserting kx-gateway --features serve-engine,hnsw stays FFI-free..."
    if cargo tree -p kx-gateway --features serve-engine,hnsw -e normal | grep -qE 'kx-llamacpp'; then
        echo " ✗ FAIL: serve-engine,hnsw dragged the FFI into kx-gateway"
        cargo tree -p kx-gateway --features serve-engine,hnsw -e normal | grep -E 'kx-llamacpp' || true
        exit 1
    fi
    echo " ✓ no kx-llamacpp in the kx-gateway serve-engine,hnsw closure"
    cargo build -p kx-cli --features serve-engine,hnsw
    cargo build -p kx-gateway --features serve-engine,hnsw
    # RC3 (GR23 CI-hardening): the standard `clippy`/`test` stages run with DEFAULT
    # features, which EXCLUDE `serve-engine` — so the live `kx serve` model loop
    # (`model_exec`: dispatch, grammar-constrained tool-calling, the RC3 tool menu +
    # curated agentic prompt) was previously only `cargo build`-checked, never
    # clippy-linted or unit-tested in CI. Lint + run its FFI-free inline unit tests
    # here (this recipe runs in the toolchain-free `build-serve-engine` CI job).
    echo "Linting + unit-testing the FFI-free serve-engine model loop (model_exec)..."
    cargo clippy -p kx-gateway --features serve-engine,hnsw --lib -- -D warnings
    cargo test -p kx-gateway --features serve-engine,hnsw --lib -- --nocapture
    echo " ✓ build-serve-engine: PASS"

# The installed-binary feature matrix stays buildable (the v0.1.0 campaign
# guard): `cargo install -p kx-cli --features hnsw` (Datasets, FFI-free) and
# `--features inference,hnsw` (the full Tier-2 install) must both CHECK, and
# the hnsw-alone closure must stay FFI-free (a C++-toolchain-less user can
# install Datasets support). The inference half only type-checks here — the
# real FFI link is `ffi-link`/`smoke-test-with-model`.
features-guard:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Asserting kx-cli --features hnsw stays FFI-free..."
    if cargo tree -p kx-cli --features hnsw -e normal | grep -qE 'kx-llamacpp'; then
        echo " ✗ FAIL: the hnsw feature dragged the FFI into kx-cli"
        exit 1
    fi
    echo " ✓ hnsw closure is FFI-free"
    cargo check -p kx-cli --features hnsw
    cargo check -p kx-cli --features inference,hnsw
    echo " ✓ features-guard: hnsw + inference,hnsw both build"
    # W1a (SN-6): the gateway-only / external-coordinator config (default feature
    # `embedded-worker` OFF) reserved by the `start_impl` stub must stay BUILDABLE,
    # so a feature-independent struct field never references a feature-gated import
    # (the W1a-1 audit-sink cfg-leak class). CI builds only the default features.
    cargo check -p kx-gateway --no-default-features
    echo " ✓ features-guard: kx-gateway --no-default-features builds"

# Byte-determinism check (I1.c). Two consecutive release builds must produce
# bit-identical artifacts. Failure indicates the build is nondeterministic and
# must be fixed before the affected change can merge.
check-reproducible:
    #!/usr/bin/env bash
    set -euo pipefail
    # I1.c is path- + metadata-sensitive: without pinned crate metadata and a
    # stripped absolute workspace path, two clean builds embed path-dependent
    # bytes and differ. CI exports these in $GITHUB_ENV before calling this
    # recipe; locally we DEFAULT them (only if unset) so `just ci` / `just
    # check-reproducible` reproduce CI's result without a manual export. CI's
    # own value (using $GITHUB_WORKSPACE) is preserved when already set.
    : "${RUSTFLAGS:=--remap-path-prefix={{justfile_directory()}}= -Cmetadata=kortecx-v0}"
    export RUSTFLAGS
    # GUARDRAIL (§2.194 deterministic-homes): `cargo clean` wipes ALL of
    # target/ — including target/models/, the checksum-verified model cache the
    # fetch recipes maintain (a 2+ GB re-download per `just ci` otherwise).
    # Stash the cache across the two clean builds; restore even on failure.
    MODELS_STASH=""
    if [ -d target/models ]; then
        MODELS_STASH="$(mktemp -d)"
        mv target/models "$MODELS_STASH/models"
    fi
    restore_models() {
        if [ -n "$MODELS_STASH" ] && [ -d "$MODELS_STASH/models" ]; then
            mkdir -p target
            rm -rf target/models
            mv "$MODELS_STASH/models" target/models
            rmdir "$MODELS_STASH" 2>/dev/null || true
        fi
    }
    trap restore_models EXIT
    cargo clean
    cargo build --release --workspace
    find target/release -maxdepth 1 -type f \( -name "*.rlib" -o -name "*.a" \) -print0 \
        | sort -z | xargs -0 shasum -a 256 > /tmp/kortecx-build-1.sha256
    cargo clean
    cargo build --release --workspace
    find target/release -maxdepth 1 -type f \( -name "*.rlib" -o -name "*.a" \) -print0 \
        | sort -z | xargs -0 shasum -a 256 > /tmp/kortecx-build-2.sha256
    # Strip absolute paths in the sha256 output (they include /tmp suffixes that vary).
    sed 's|target/release/||' /tmp/kortecx-build-1.sha256 > /tmp/kortecx-build-1.norm
    sed 's|target/release/||' /tmp/kortecx-build-2.sha256 > /tmp/kortecx-build-2.norm
    diff /tmp/kortecx-build-1.norm /tmp/kortecx-build-2.norm
    echo "I1.c byte-determinism: PASS"

# Run the kx-llamacpp model-smoke-test feature: downloads a ~1.2 MB GGUF and
# runs the full safe-wrapper inference pipeline (load → tokenize → decode →
# sample → detokenize). Gated separately so the default `just ci` doesn't
# need network access.
smoke-test-with-model:
    cargo test -p kx-llamacpp --features model-smoke-test -- --nocapture

# GR15 real-model behavioral gate (`real-model-e2e`) — fetch the public Qwen3-0.6B
# stand-in, then serve `kx/recipes/chat` through the full path (invoke → worker →
# real inference → commit → GetContent) and assert the completion is CLEAN (no
# ChatML scaffolding leak, no `kx demo result` placeholder) AND greedy decode is
# deterministic across gateways. Builds the llama.cpp FFI; the CI `real-model-e2e`
# job runs exactly this. Gated separately so the default `just ci` stays FFI-free.
real-model-e2e: fetch-agent-model
    # `--test-threads=1`: run the real-inference tests SERIALLY. Each serves a model
    # and llama.cpp already uses every CPU core, so running them concurrently on a
    # CPU-only CI runner starves each inference (neither commits in time). Serial =
    # one inference at a time = full CPU each.
    cargo test -p kx-gateway --features inference --test al1_serve -- --ignored --nocapture --test-threads=1
    # RC1 (D172): the real-model eval witness — score a live ReAct chain via ScoreRun
    # (advisory Tier-B floors over the Qwen3 stand-in; the flake-proof gate is `just eval`).
    cargo test -p kx-gateway --features inference --test eval_real_model -- --ignored --nocapture --test-threads=1
    # T-RUNAPP-SECRET-SCOPE-OBSERVATION: the RunApp credentialed-connector live gate.
    # Build the bundled Gmail connector, then drive it via RunApp under the served model.
    # KX_GMAIL_FAKE lives in the ENVIRONMENT (never a racy runtime `set_var` — that
    # intermittently spawned the connector without FAKE, wedging register on a real dial),
    # so the connector reliably answers canned data (no egress). On Qwen3-0.6B the model
    # typically answers without dialing (the observation-commit oracle passes vacuously);
    # the DETERMINISTIC proof of the fix is `kx-proto` (wire round-trip) +
    # `kx-coordinator::observation_dispatch_preserves_the_chain_secret_scope`, always run
    # by `cargo test`. Locally, drive on BOTH engines with Gemma-4 (GR24) — llama.cpp fires
    # `gmail/search` → observation commits → answer.
    cargo build -p kx-connector-gmail
    KX_GMAIL_FAKE=1 cargo test -p kx-gateway --features inference --test app_live_serve runapp_gmail_connection_and_secret_scope_live -- --ignored --nocapture --test-threads=1

# RC1 (D172) — the real-model eval witness, LOCAL Gemma deep-test (Tier-B, ADVISORY).
# Drives a live ReAct chain on a real OSS model and scores it through ScoreRun (the
# per-run quality readout proven over genuine model output, GR15/GR24). The flake-proof
# regression GATE is the deterministic `just eval`; these numbers are advisory Spikes.
#   KX_SERVE_OLLAMA=on KX_SERVE_OLLAMA_MODELS=gemma3:12b just eval-real   # local Gemma (Ollama)
#   just fetch-agent-model && just eval-real                              # GGUF stand-in
eval-real:
    cargo build -p kx-mcp  # the bundled stdio tool bins (echo / calc / kv)
    cargo test -p kx-gateway --features inference --test eval_real_model -- --ignored --nocapture --test-threads=1

# LOCAL / manual witness (NOT a CI job): drive a LIVE ReAct chain that FIRES a real
# tool on a capable model. The DETERMINISTIC, CI-runnable regression guard for this
# lives model-free in `crates/kx-coordinator/tests/react_live.rs` — it pins the
# fire-commits invariant (a `world_mutating` observation COMMITS) for BOTH the JSON
# envelope AND the Gemma-native `<|tool_call>` shape, closing the BUG-28 gap (no e2e
# ever asserted a tool FIRES, only that an answer settled). This recipe is the
# real-model witness on top: it serves `kx/recipes/react-auto` (`KX_SERVE_AUTOGRANT`)
# with the bundled `mcp-echo`, so the live loop has a tool to call. Set
# `KX_SERVE_MODEL_GGUF=<gemma.gguf>` to exercise the Gemma-native format (the BUG-28
# scenario); the default Qwen3 stand-in fires via the JSON envelope. The full
# cross-surface `world_mutating`-fire assertion is the deep-test campaign's "ReAct
# tool-calling" matrix row.
react-fire-local: fetch-agent-model
    cargo build -p kx-mcp # the bundled kx-mcp-echo firing bin (or set KX_MCP_ECHO_PATH)
    KX_SERVE_AUTOGRANT=1 cargo test -p kx-gateway --features inference \
        --test react_auto_serve -- --ignored --nocapture --test-threads=1

# LOCAL / manual gate (NOT a CI job): exercise the safe-wrapper inference pipeline
# with GPU offload forced ON (`KX_N_GPU_LAYERS=-1`) so an Apple-Silicon dev sees
# Metal actually used (look for `offloaded N/N layers to GPU` + `ggml_metal_*`),
# plus the q8_0 KV-cache + flash-attn determinism cases. Pairs with the AL1
# live-serve witness (`cargo test -p kx-gateway --features inference -- --ignored`).
# CPU elsewhere (CUDA is cloud-only, D28); harmless no-op without a GPU.
metal-smoke:
    KX_N_GPU_LAYERS=-1 cargo test -p kx-llamacpp --features model-smoke-test -- --nocapture

# LOCAL / manual gate (NOT a CI job): downloads a small VLM (Qwen2-VL-2B-Instruct
# GGUF + mmproj, ~1.6 GB) and runs the full IMAGE pipeline through the safe wrapper
# (load → projector → decode bitmap → tokenize → mtmd prefill → generate),
# asserting image→text non-empty + greedy determinism + fail-closed on corrupt
# bytes. Run it before a release or when touching the mtmd FFI. Deliberately kept
# OUT of CI: a 2B VLM (multi-GB download + a CPU inference run, twice) is too heavy
# for the runner, and `ffi-link` + the no-model tests already cover the Linux link
# + the dispatch/sniff/routing logic. NOT part of `just ci`.
smoke-test-multimodal:
    cargo test -p kx-llamacpp --features model-smoke-test-multimodal --release -- --nocapture

# ============================================================================
# Scale / performance gate
# ============================================================================

# Scale smoke (M2.1 + M2.2 + M2.2b + M2.x-E / D92 — the resume-availability
# invariant): fold a 25k+ Mote journal in RELEASE and assert (M2.1) the incremental
# children-index re-fold stays ~linear, (M2.2) resume-from-`FoldCheckpoint`
# reproduces the full fold exactly + its cost is bounded by live-state size under
# churn (not by journal length), (M2.2b) the SAME bound holds end-to-end over a real
# disk-backed SQLite journal + an on-disk checkpoint sidecar, and (M2.x-E / IMP-2)
# offline schema migration (`migrate_to`) stays O(entries) so resume-after-upgrade
# is not an outage, and (IMP-4 / D116) the cold-recovery projection fold stays
# O(entries) at 10^5 — the read side of the single-writer ceiling (cold resume folds
# the whole log; a super-linear fold turns a large-log resume into an outage). The
# fold cases live in `kx-projection`; the migration case in `kx-journal` (both
# llamacpp-free — no C++ FFI in this lean job). A super-linear resume is an outage,
# and resume IS the product. `--release` is REQUIRED — in a debug build the
# differential oracle re-imposes the O(n^2) full rebuild on every fold and the ratio
# assertions are skipped. Also gates (M7 / D86) the kx-catalog governance paths —
# signature register/lookup, the grant-ledger authorization fold, and the
# depth-bounded deep-chain query — all stay sub-linear / depth-bounded at 25k-50k.
# Also gates (R6) the Morphic recipe library's wide fan-out (`map_reduce` to a
# 10k-wide reduce) — deterministic compile + reproducible run + ~linear curve.
scale-smoke:
    cargo test -p kx-projection --release --test incremental_children_index -- --ignored --nocapture --test-threads=1
    cargo test -p kx-projection --release --test fold_checkpoint -- --ignored --nocapture --test-threads=1
    cargo test -p kx-projection --release --test run_metadata_scale -- --ignored --nocapture --test-threads=1
    cargo test -p kx-projection --release --test fold_curve_scale -- --ignored --nocapture --test-threads=1
    cargo test -p kx-journal --release --test schema_evolution -- --ignored --nocapture --test-threads=1
    cargo test -p kx-capture --release --test scale -- --ignored --nocapture --test-threads=1
    cargo test -p kx-audit --release --test scale -- --ignored --nocapture --test-threads=1
    cargo test -p kx-catalog --release --test scale -- --ignored --nocapture --test-threads=1
    cargo test -p kx-fleet --release --test scale -- --ignored --nocapture --test-threads=1
    cargo test -p kx-gateway-core --release --test scale -- --ignored --nocapture --test-threads=1
    cargo test -p kx-workflow --release --test stress_fanout -- --ignored --nocapture --test-threads=1
    cargo test -p kx-dataset-hnsw --release --test scale -- --ignored --nocapture --test-threads=1
    cargo test -p kx-gateway --release --test global_tail_stress -- --ignored --nocapture --test-threads=1

# IMP-4 (D116) single-writer scale-readiness measurement spike — publish the
# single-writer journal commit ceiling + the projection-fold curve so a real number
# replaces the "qualitatively true, quantitatively unproven" placeholder (HANDOFF
# §3.9 §A). NON-GATING (not part of `just ci`): every test is `#[ignore]`, prints
# commits/s + µs/entry, and asserts only a loose catastrophic-regression floor (the
# fold-curve linearity ratio is the only gated piece, run by `scale-smoke` above).
# `--release` is REQUIRED. `KX_CEILING_HUGE=1 just bench-ceiling` adds the 10^6 tier
# (hundreds of MB RAM + on-disk WAL — local only). On-disk commits/s is platform-
# sensitive (macOS fsync is weaker than Linux) — label numbers with their environment.
bench-ceiling:
    cargo test -p kx-journal --release --test ceiling_throughput -- --ignored --nocapture --test-threads=1
    cargo test -p kx-coordinator --release --test ceiling_e2e -- --ignored --nocapture --test-threads=1
    cargo test -p kx-projection --release --test fold_curve_scale -- --ignored --nocapture --test-threads=1

# Golden Rule 10 — the runtime profiling harness. NON-GATING (not part of `just
# ci`): captures an environment-labelled, schema-1 JSON report of the FFI-free
# runtime's client-observable costs, then re-runs the existing throughput spikes
# so a single invocation prints the full single-node picture. The `kx-profile`
# JSON lands in `target/profile/` (gitignored); COPY it into the PRIVATE
# `docs/benchmarks/YYYY-MM-DD-<topic>.json` trend record (never committed to OSS
# — SN-2). Absolute latencies are platform-sensitive (macOS fsync ≠ Linux) — the
# report's `env` block labels every number. `--release` is REQUIRED for honest
# numbers. M1 warm-up (start→SERVING) + M2 submit→Committed come from kx-profile;
# M3 fold curve + M4 commit ceiling + M5 catalog discovery come from the spikes;
# M7a react answer-settle + M7b react tool-round (PR-2d-2) need the bundled
# stdio tool bin, built first so M7b is never silently skipped.
# RC1 (D172) — the measure-first eval gate (the regression ratchet every later RC PR
# must hold). Scores the embedded `golden-v1` corpus (Tier A: scripted transcripts —
# FFI-free, no model / network / clock, so it cannot flake) and fails CLOSED on any
# regression vs the COMMITTED `crates/kx-eval/corpus/golden-v1/baseline.json` or on
# corpus drift. `cargo run -p kx-eval -- run --update-baseline` (manual) re-captures the
# baseline — RC2 (grammar) raises it in-PR. The same scorers also run under `test` (the
# kx-eval integration test); the real-model Tier-B numbers are advisory Spikes (`eval-real`).
eval:
    cargo run -q -p kx-eval -- run

profile iterations="8":
    cargo build --release -p kx-mcp
    cargo run --release -p kx-profile -- --iterations {{iterations}}
    cargo test -p kx-coordinator --release --test ceiling_e2e -- --ignored --nocapture --test-threads=1
    cargo test -p kx-projection --release --test fold_curve_scale -- --ignored --nocapture --test-threads=1
    cargo test -p kx-catalog --release --test scale -- --ignored --nocapture --test-threads=1

# Golden Rule 10 — inference-path profiling (M6: model warm-up + TTFT +
# tokens/sec). Requires the `model-smoke-test` GGUF (auto-downloaded, ~1.2 MB,
# SHA-verified) + the llama.cpp FFI; LOCAL-only (Metal on Apple Silicon for real
# numbers — in-container CPU inference is not representative, D135). Prints the
# timing the standard smoke test now captures; copy the numbers into the private
# trend record alongside the kx-profile JSON.
profile-inference:
    cargo test -p kx-llamacpp --release --features model-smoke-test -- --nocapture

# ============================================================================
# Preflight diagnostic
# ============================================================================

# Preflight check — verify the host has every toolchain + system library the
# build needs BEFORE the first `cargo build` invocation. Useful for new-
# contributor onboarding + CI runner debugging. Each check reports green/red;
# missing optional tools are flagged as warnings, not errors.
doctor:
    #!/usr/bin/env bash
    set -uo pipefail

    OK=" ✓"
    FAIL=" ✗"
    WARN=" !"
    errors=0
    warnings=0

    check() {
        local what="$1"
        local cmd="$2"
        if eval "$cmd" >/dev/null 2>&1; then
            echo "${OK} ${what}"
            return 0
        else
            echo "${FAIL} ${what}"
            errors=$((errors + 1))
            return 1
        fi
    }

    warn_check() {
        local what="$1"
        local cmd="$2"
        if eval "$cmd" >/dev/null 2>&1; then
            echo "${OK} ${what}"
        else
            echo "${WARN} ${what}"
            warnings=$((warnings + 1))
        fi
    }

    # Print a per-OS install hint for a missing optional (Tier 1) dependency.
    # $1 = Homebrew formula (macOS, empty ⇒ Xcode CLT); $2 = apt/dnf package(s).
    os_hint() {
        case "$(uname)" in
            Darwin)
                if [ -n "$1" ]; then echo "       fix (macOS):         brew install $1"
                else                 echo "       fix (macOS):         xcode-select --install" ; fi ;;
            Linux)
                if   command -v apt-get >/dev/null 2>&1; then echo "       fix (Debian/Ubuntu): sudo apt-get install -y $2"
                elif command -v dnf     >/dev/null 2>&1; then echo "       fix (Fedora):        sudo dnf install -y $2"
                else echo "       install '$2' via your distro's package manager" ; fi ;;
            *) echo "       install it for your platform" ;;
        esac
    }

    # Like warn_check, but prints an install hint when the dep is missing.
    # $1 = label, $2 = test cmd, $3 = brew formula, $4 = apt/dnf package(s).
    warn_with_hint() {
        if eval "$2" >/dev/null 2>&1; then
            echo "${OK} $1"
        else
            echo "${WARN} $1"
            warnings=$((warnings + 1))
            os_hint "$3" "$4"
        fi
    }

    echo "kortecx preflight — TIERED"
    echo "  Tier 0 (REQUIRED): Rust only — runs kx run / replay / serve (FFI-free)."
    echo "  Tier 1 (OPTIONAL): C++ toolchain + llama.cpp submodule — local LLM inference."
    echo ""

    echo "Tier 0 — required Rust toolchain:"
    check "Rust toolchain installed and resolves to rust-toolchain.toml pin" \
        "rustup show active-toolchain"
    check "cargo on PATH" "command -v cargo"
    check "rustc on PATH" "command -v rustc"
    check "rustfmt + clippy components present" \
        "rustup component list --installed | grep -q rustfmt && rustup component list --installed | grep -q clippy"

    echo ""
    echo "Tier 1 — local LLM inference (optional; skip for the FFI-free runtime):"
    warn_with_hint "cmake on PATH" "command -v cmake" "cmake" "cmake"
    warn_with_hint "clang on PATH" "command -v clang" "llvm" "clang"

    # Platform-specific libclang + C++ toolchain checks (bindgen needs libclang;
    # the static-archive link needs a C++ compiler). Tier 1 ⇒ warnings, not errors.
    if [ "$(uname)" = "Linux" ]; then
        warn_with_hint "libclang available (bindgen)" \
            "ldconfig -p 2>/dev/null | grep -q libclang || dpkg -s libclang-dev 2>/dev/null | grep -q 'install ok installed'" \
            "llvm" "libclang-dev"
        warn_with_hint "C++ toolchain (g++)" "command -v g++" "" "build-essential"
    elif [ "$(uname)" = "Darwin" ]; then
        warn_with_hint "Xcode Command Line Tools (libclang + clang++)" "xcode-select -p" "" ""
    fi

    warn_check "llama.cpp submodule checked out (just setup-inference)" \
        "test -f crates/kx-llamacpp-sys/llama.cpp/CMakeLists.txt"

    if [ -f crates/kx-llamacpp-sys/llama.cpp/CMakeLists.txt ] && command -v git >/dev/null 2>&1; then
        pinned=$(git -C crates/kx-llamacpp-sys/llama.cpp rev-parse HEAD 2>/dev/null || echo "unknown")
        echo "   note: submodule HEAD = ${pinned}"
        echo "         see crates/kx-llamacpp-sys/PIN.md for the audit ritual on advancing the pin."
    fi

    echo ""
    echo "Optional tools:"
    warn_check "just on PATH (for just ci recipes)" "command -v just"
    warn_check "cargo-deny installed (for just deny)" "cargo deny --version"
    warn_check "git on PATH" "command -v git"

    echo ""
    if [ "${errors}" -gt 0 ]; then
        echo "${FAIL} preflight FAILED: ${errors} Tier-0 errors, ${warnings} warnings"
        echo "    Tier 0 is required to build/run the runtime — fix the errors above."
        exit 1
    elif [ "${warnings}" -gt 0 ]; then
        echo "${WARN} Tier 0 OK (the FFI-free runtime is good to go); ${warnings} Tier-1/optional warnings"
        echo "    Run \`just setup\` to install the kx binary now; the warnings only"
        echo "    matter if you want local LLM inference (\`just setup-inference\`)."
        exit 0
    else
        echo "${OK} preflight passed — Tier 0 + Tier 1 ready"
        exit 0
    fi

# ============================================================================
# Parallel lanes (D175) — isolated git worktrees so concurrent sessions never
# contend on the build lock or clobber target/. Each worktree gets its own
# default target dir; the printed CARGO_TARGET_DIR export makes that explicit
# (and lets a lane point at a faster disk). sccache is suggested as a DEV-LOCAL
# opt-in only — never wired into .cargo/config.toml or CI, so the I1.c
# byte-determinism / check-reproducible path is untouched.
# ============================================================================

# Create an isolated work lane: a git worktree at ../kortecx-lane-<name> on a
# fresh local branch lane/<name> (rename to feat/... before pushing).
lane-new name:
    #!/usr/bin/env bash
    set -euo pipefail
    ROOT="$(git rev-parse --show-toplevel)"
    LANE_DIR="$(dirname "$ROOT")/kortecx-lane-{{name}}"
    git worktree add "$LANE_DIR" -b "lane/{{name}}"
    echo ""
    echo "lane '{{name}}' ready: $LANE_DIR"
    echo "run these in the lane shell:"
    echo "    cd $LANE_DIR"
    echo "    export CARGO_TARGET_DIR=$LANE_DIR/target"
    if command -v sccache >/dev/null 2>&1; then
        echo "    export RUSTC_WRAPPER=sccache   # optional dev-local compiler cache (shared across lanes)"
    else
        echo "    # tip: 'cargo install sccache' to share compiled deps across lanes (dev-local only)"
    fi
    echo "claim your feature-ledger.toml row (state -> in-progress, set branch) in the lane's FIRST commit."

# Remove a lane created by lane-new (worktree + its local branch when merged).
lane-drop name:
    #!/usr/bin/env bash
    set -euo pipefail
    ROOT="$(git rev-parse --show-toplevel)"
    LANE_DIR="$(dirname "$ROOT")/kortecx-lane-{{name}}"
    git worktree remove "$LANE_DIR"
    git branch -d "lane/{{name}}" 2>/dev/null || echo "branch lane/{{name}} kept (unmerged) — delete manually with -D if abandoned"
    echo "lane '{{name}}' dropped"

# Fast local test runner: cargo-nextest when installed (per-test process
# isolation + better parallelism), plain cargo test otherwise. CI is unchanged.
test-fast:
    #!/usr/bin/env bash
    set -euo pipefail
    if command -v cargo-nextest >/dev/null 2>&1; then
        cargo nextest run --workspace
    else
        echo "cargo-nextest not installed (cargo install cargo-nextest) — falling back to cargo test"
        cargo test --workspace
    fi

# ============================================================================
# Cleanup
# ============================================================================

# Wipe all build artifacts.
clean:
    cargo clean

# Optional local developer recipes, loaded if present.
import? 'justfile.private'
