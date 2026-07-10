# Security policy

kortecx is a distributed agentic runtime that **loads third-party models and runs third-party tools**.
Its security model assumes untrusted models, untrusted tool output, and untrusted content payloads, and
holds the line with a small set of enforced seams. This document is how to report a vulnerability and what
is in scope.

## Reporting a vulnerability

**Please do not open a public issue for a security vulnerability.** Report privately, one of:

1. **GitHub private vulnerability reporting** (preferred): the **Security** tab of this repository →
   *Report a vulnerability*. This opens a private advisory only the maintainers can see.
2. **Email**: `hello@kortecx.com` with subject `SECURITY:` and a description + reproduction.

We aim to acknowledge a report within a few business days, agree on a disclosure timeline, and credit
reporters who wish to be credited. Please give us reasonable time to ship a fix before public disclosure.

## Supported versions

kortecx is pre-1.0 (`0.x`). Security fixes land on `main` and the latest `0.x` release. Older `0.x`
releases are not separately patched — upgrade to the latest.

## Security model (what the runtime enforces)

The runtime is built on **"the model proposes; the runtime enforces"**:

- **Capability boundary** — a workflow declares the effects it may cause; the runtime grants only the
  **intersection** of the request with the caller's policy (monotonic narrowing, never a widen). Tool
  resolution is *fuzzy-in / exact-out*: similarity may *surface* a tool, never *grant* one.
- **Deny-by-default egress** — outbound network / tool access is fail-closed and SSRF-guarded (link-local
  and metadata endpoints are always refused).
- **Exactly-once, journal-backed effects** — a committed effect is a durable fact, never re-run; identity
  is exact cryptographic equality on the identity/commit path (no similarity operator there).
- **Content is content-addressed + size-capped** on ingest.

## In scope

- Memory-safety or logic bugs in the Rust runtime crates (`crates/kx-*`) reachable from untrusted input:
  a malformed **model file (GGUF)**, untrusted **tool output**, an untrusted **content-store / journal /
  checkpoint payload**, or an untrusted **plan / tool-call string**.
- A capability **widen** (obtaining an effect the policy did not grant), an egress bypass, an
  exactly-once violation, or an identity/idempotency confusion on the commit path.
- Sandbox / isolation escapes in the per-effect executor (`crates/kx-executor`).

## Known trust boundaries (see `docs/threat-model.md`)

- The **C++ FFI boundary** (llama.cpp via `kx-llamacpp-sys` / `kx-llamacpp`) and the vendored image/GGUF
  parsers execute **below** the Rust safety model. This surface is being fuzzed (`fuzz/`); it is the
  highest-value target for review. A malicious GGUF or media blob is an assumed adversary input.
- Loading a model or running a tool you do not trust is **inherently** giving it your compute — the
  runtime bounds *what a workflow may cause* (effects, egress, resources), not what a model *is*.

## Out of scope

- Denial of service from a model you chose to load being large/slow (resource limits are an operability
  concern, not a vulnerability).
- Findings that require an already-privileged local attacker (they already own the process).
- The hosted cloud offering (reported through its own channel).
