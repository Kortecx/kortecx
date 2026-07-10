# Threat model

kortecx runs **untrusted models** and **untrusted tools** over **untrusted content**. This document names
the assets it protects, the trust boundaries, the adversaries, and where each is defended — so a reviewer
knows what the runtime does and does not claim. It is distinct from the *correctness* hard-problems list
(exactly-once, deterministic replay); this is about an *adversary*.

## Assets (what must not be violated)

1. **Journal integrity + exactly-once.** A committed effect is a durable fact and is never re-run. The
   journal is the synchronization substrate; corrupting it corrupts the runtime.
2. **The capability boundary.** A workflow causes only the effects its policy grants — never a widen.
3. **Egress control.** Outbound access is deny-by-default and SSRF-guarded.
4. **Content integrity.** Content is content-addressed; a ref resolves to exactly its bytes.
5. **Host integrity.** A tool or model cannot escalate beyond the compute it was given.

## Trust boundaries + adversary inputs

| Boundary | Untrusted input | Adversary goal |
|---|---|---|
| **Model file (GGUF)** → `kx-llamacpp` FFI | a malicious `.gguf` / `mmproj` / media blob | memory corruption below the Rust safety model |
| **Model output** → `kx-toolcall`, `kx-planner` | a crafted tool-call / plan string | conjure an ungranted tool; DoS the parser; a prompt-injection widen |
| **Tool output / content** → `kx-content`, `kx-journal`, `kx-projection` | a corrupt journal entry / checkpoint / content payload | panic/OOM on decode; forge a journaled fact |
| **Tool execution** → `kx-executor` | a hostile third-party tool process | escape the per-effect sandbox / exceed resource ceilings |
| **Network egress** → `kx-mcp` | a tool that dials an internal address | SSRF / metadata-endpoint access |

## Defenses (surface → mitigation)

- **Model → runtime:** *the model proposes, the runtime enforces.* A model's plan / tool-call is **decoded
  fail-closed** (`decode_plan`, `parse_tool_call` — total, panic-free, DoS-capped) and lowered into a
  **registered** DAG; tool resolution is fuzzy-in / **exact-out** (similarity may surface a tool, never
  grant one). No similarity operator on the identity / commit path — exact cryptographic equality only.
- **Capability:** the grant is the **intersection** of the request with the caller's policy (monotonic
  narrowing). A model can never widen its own warrant.
- **Byte parsers:** `decode_entry`, `FoldCheckpoint::from_bytes`, `decode_plan` are length/version/codec
  gated and **fuzzed** (`fuzz/`) — a panic on adversarial bytes is a tracked finding.
- **Egress:** fail-closed; link-local + cloud-metadata endpoints are always refused (`kx-mcp` egress guard).
- **Execution:** each effect runs in a per-effect sandbox with resource ceilings (`kx-executor`).

## Residual risk (named, not hidden)

- **The C++ FFI boundary is the deepest surface.** A malicious GGUF or media blob is parsed by vendored
  C/C++ (llama.cpp, `stb_image`) that runs *below* the Rust safety model — every capability/warrant/broker
  guard is downstream of it. Mitigations: `check_tensors` validation, fuzzing the FFI parse paths
  (in progress), and treating an untrusted model file as an assumed adversary. **Loading a model you do
  not trust is giving it your process** — the runtime bounds what a *workflow* may cause, not what a
  *model is*. An external audit of this surface is the standing recommendation.
- **Client-side-only enforcement in single-operator mode.** Some discipline gates are advisory on an
  unprotected repo; server-side enforcement is the hardened path.
- **Denial of service** from a large/slow model you chose to load is an operability concern, out of scope
  as a vulnerability.

Report a suspected violation of any asset above via `SECURITY.md`.
