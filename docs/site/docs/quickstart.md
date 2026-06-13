---
id: quickstart
title: Quickstart
sidebar_label: Quickstart
description: Install kx, start the runtime, and run your first chain from the CLI and the Python and TypeScript SDKs.
---

# Quickstart

Install the runtime, start it locally, and run your first Blueprint and chain
from the CLI and both SDKs. Every command below is real and copy-paste-correct.

## Install

```bash
# Prebuilt binary (Linux x86_64/arm64, macOS arm64) — SHA-256 verified, no sudo,
# installs to ~/.local/bin. The prebuilt ships the web console + Datasets built in.
curl -fsSL https://raw.githubusercontent.com/Kortecx/kortecx/main/scripts/install.sh | sh
```

From source (Rust 1.94+; each variant adds a capability):

```bash
git clone https://github.com/Kortecx/kortecx.git && cd kortecx
cargo install --path crates/kx-cli                            # the core runtime — no C++, no node
cargo install --path crates/kx-cli --features hnsw            # + Datasets/RAG (still no C++)
cargo install --path crates/kx-cli --features inference,hnsw  # + local LLM inference (needs a C++ toolchain)
```

Plain `cargo install` never needs node or C++. Local LLM inference (Tier 1)
additionally needs a C++ toolchain (CMake, clang/libclang) and a GGUF model.

## Prove exactly-once

Run the canonical demo workflow, crash it mid-commit, and replay. The digest is
identical across the clean run and the crash-then-replay run:

```bash
# 1. Run the demo to completion, capturing its deterministic digest.
kx run    --journal /tmp/kx.db --content /tmp/kx-content
#    → 7d22d4bdfc6f68a4311f40b20f3fe7c67f4c5d2b352f3bff8722b439e94a5af9 (8/8 committed)

# 2. Start fresh, but hard-abort right after a side effect commits.
rm -f /tmp/kx.db; rm -rf /tmp/kx-content
kx run    --journal /tmp/kx.db --content /tmp/kx-content --crash-at post-commit-vtc

# 3. Recover from the journal and finish the run.
kx replay --journal /tmp/kx.db --content /tmp/kx-content
#    → same digest — the crashed step was re-read, not re-run.
```

Same digest = the exactly-once property, demonstrated.

## Start the runtime

One command starts the gateway, the embedded worker, the live-event bridge, and
(prebuilt binaries) the web console. It's **zero-config** — you only pass the
auth posture; the journal, content store, and catalog auto-resolve:

```bash
kx serve --dev-allow-local
#    gRPC on 127.0.0.1:50151 · events on ws://127.0.0.1:50152
#    web console at http://127.0.0.1:50180  ← open this in your browser
```

On start, the server prints a banner with every resolved path and endpoint:

```text
kx-gateway STARTUP — resolved durable layout + endpoints
  data_dir=~/.kortecx  journal=~/.kortecx/kx.db  content_dir=~/.kortecx/content
  catalog_dir=~/.kortecx/catalog  (catalog.db · members.db · telemetry.db · capture.db · uploads.db · datasets/)
  grpc_endpoint=http://127.0.0.1:50151  ws_endpoint=ws://127.0.0.1:50152  console_url=http://127.0.0.1:50180/
  auth_mode=dev-allow-local  connect_hint=kx runs list --endpoint http://127.0.0.1:50151
```

The base directory is **stable across restarts** (your runs, telemetry, and
content persist). Relocate it with `KX_DATA_DIR=/path/to/data`, or pin the
individual paths explicitly:

```bash
kx serve --dev-allow-local --journal /tmp/kx.db --content /tmp/kx-content
```

:::note Auth is required (deny-all by default)
A bare `kx serve` with no auth posture fails fast with a hint — it never opens
an unauthenticated server. Pass `--dev-allow-local` (loopback development; alias
`--allow-local-dev`) or bearer tokens (`--auth-token <token>=<party>`). See
[Security](./security.md).
:::

## Run your first Blueprint

In another terminal:

```bash
# A single-step echo — the canonical hello-world (typed input: topic).
kx invoke kx/recipes/echo --args '{"topic":"durable agents"}' --wait

# A real multi-node DAG, model-free: root → 3 children → gather (5 steps, all committed).
kx invoke kx/recipes/fanout-demo --args '{}' --wait
```

Inspect anything — the DAG, a committed result, the live event stream:

```bash
kx projection --instance <instance-id>                       # the run as a DAG of step states
kx projection --instance <instance-id> --at-seq 3            # …time-traveled to any point
kx content    --ref <content-ref> --instance <instance-id>   # a committed result (raw bytes)
kx events     --instance <instance-id> --follow              # live-tail the run's events
```

## Run your first chain

A **chain** composes published task handles into a DAG with a small string DSL
(`>` sequential, `&` / `|` parallel, `[ … ]` grouping). The same expression
lowers identically across the CLI and both SDKs.

### CLI

```bash
# A fan-out → gather chain: `a` feeds both `b` and `c`.
kx chain run "a > [b & c]" \
  --task a=pure --task b=pure --task c=pure \
  --wait
```

### Python

```bash
pip install kortecx            # core client
pip install 'kortecx[ws]'      # + the optional WebSocket live-tail
```

```python
from kortecx import KxClient

with KxClient("http://127.0.0.1:50151") as kx:
    result = kx.invoke("kx/recipes/echo", {"topic": "hello"}, wait=True)
    print(result.text)             # the committed output
    print(result.instance_id)      # server-derived run id (hex)
```

The chain string DSL and the per-language operator API are in
[Chains in Python](./chains/python.md).

### TypeScript

```bash
npm install @kortecx/sdk       # node + browser entry points
npm install ws                 # optional: node live-tail
```

```ts
import { KxClient } from "@kortecx/sdk";

const kx = new KxClient("http://127.0.0.1:50151");
const result = await kx.invoke("kx/recipes/echo", { topic: "hello" }, { wait: true });
console.log(result.state, result.instanceId, result.text);
kx.close();
```

The chain string DSL and the per-language operator API are in
[Chains in TypeScript](./chains/typescript.md).

## Run the agent loop

With an inference build (`--features inference,hnsw`) and a fit GGUF model, you
can drive chat and the full ReAct agent loop on-device. Download any fit model
and point the server at it:

```bash
curl -fsSL -o qwen3-0.6b-q4_k_m.gguf \
  https://huggingface.co/unsloth/Qwen3-0.6B-GGUF/resolve/main/Qwen3-0.6B-Q4_K_M.gguf

KX_SERVE_MODEL_GGUF="$PWD/qwen3-0.6b-q4_k_m.gguf" \
  kx serve --dev-allow-local
```

```bash
# One-shot chat: greedy decode over your model, committed like any other step.
kx invoke kx/recipes/chat --args '{"prompt":"What is the capital of France?"}' --wait

# A full ReAct agent: reason → call a tool → observe → answer, every turn a durable fact.
kx invoke kx/recipes/react \
  --args '{"instruction":"Echo the word kortecx via your tool, then summarize.","max_turns":4,"max_tool_calls":2}' \
  --wait
```

Crash the server mid-run and start it again: the loop resumes from its committed
turns — that's the whole point.

## Next steps

- **[Concepts](./concepts.md)** — the model behind the guarantees.
- **[Chains DSL reference](./chains/dsl-reference.md)** — the full operator
  grammar, precedence, and worked examples.
- **[Security](./security.md)** — the deny-by-default posture and the
  model-proposes / runtime-enforces boundary.
