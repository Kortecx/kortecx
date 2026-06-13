---
id: security
title: Security
sidebar_label: Security
description: Server-built warrants, the model-proposes / runtime-enforces boundary, and Kortecx's deny-by-default defaults.
---

# Security

Kortecx is **closed by default, opened explicitly**. Identity is always derived
server-side, world effects pass through a single enforcement door, and a model
can *propose* an action but never *authorize* it.

## Deny-by-default

| Surface | Default | How you open it |
|---|---|---|
| Authentication | the server answers **nobody** | `--dev-allow-local` (loopback only) or `--auth-token <token>=<party>` |
| Listeners | bind **loopback** | pass an explicit `addr:port` |
| Browser access (CORS) | **deny all** | `--cors-origin <origin>` (never a wildcard; the embedded console auto-grants only its own loopback origin) |
| Token persistence | tokens live in **memory only** | — (the console and SDKs never persist a bearer token beyond memory) |

With no flags, a `kx serve` answers no one. This is intentional: you opt into
exactly the exposure you want.

## Model proposes, runtime enforces

This is the load-bearing security boundary (internally, **SN-8**).

A model — or any client — may **propose** a topology, a tool call, or an action.
But only the **runtime** decides whether it happens, and it decides by **exact
equality**, never a fuzzy or similarity score:

- A [chain](./chains/dsl-reference.md) only changes what is *proposed*. The server
  still compiles and warrants every step.
- A [warrant](./concepts.md#warrant--capability--capabilitybroker) is built
  server-side and scopes what a Mote may do (filesystem, network, tools,
  resources).
- The **CapabilityBroker** is the *single door* through which every world effect
  passes. Enforcement happens there, and it carries the per-tool idempotency
  contract.
- A world-mutating step with no idempotency-supporting tool, or a run that was
  not registered first, is **refused at submit** — before any dispatch.

> A model can propose an action, but only the runtime's checks can let it happen.

## Identity is server-derived

Every `MoteId`, `instance_id`, `content_ref`, and `terminal_mote_id` is computed
by the runtime. The SDKs and CLI **never** construct one — they only carry the
server's bytes (surfaced as lowercase hex).

This means a client is never a source of identity. A caller's party is derived
from its bearer token, not asserted by the caller. Re-submitting the same recipe
is always a **new run** with a fresh registered instance id (the cross-boundary
idempotency token); the definition/content hash survives only as a
[recipe fingerprint](./concepts.md#run--instance-id--recipe-fingerprint) for
discovery and reuse.

## Governance: warrants, critics, promotion

- **Warrants** scope capability. A capability is a grantable power; warrants are
  composed and resolved server-side (including team membership: a member's
  effective warrant is `intersect(team_warrant, member_role)`).
- **Critics** are deterministic checks on a producer's output, described as data
  (schema / dedup / statistical bounds / PII). A verdict is a content-addressed
  `Valid` / `Invalid` fact, compared by exact equality.
- **Promotion** withholds a world-mutating producer's consumers until its declared
  critic commits a `Valid` verdict — **fail-closed** otherwise. Nothing
  downstream trusts an unvalidated, world-mutating result.
- **Repudiation cascades**: marking a committed result invalid cascades to its
  dependents along edges, so nothing downstream trusts an invalidated input.

## Exactly-once is a safety property

Because a world-mutating step is **served from its committed result on replay,
never re-run** (see [Concepts → Recovery](./concepts.md#recovery--re-fold)),
crashes cannot cause a side effect to fire twice. The
[exactly-once demo](./quickstart.md#prove-exactly-once) demonstrates this with an
identical projection digest across a clean run and a crash-then-replay run.

## Transport security

- **TLS** covers the gRPC listener in-binary (`--tls-cert` / `--tls-key`).
- The **WebSocket bridge** and **web console** are loopback plaintext by default —
  front them with a TLS proxy for remote browsers.
- A bearer token sent over plaintext `http://` to a non-loopback host emits a
  warning from the SDKs. Browser tokens are visible to page JS — use them only for
  trusted first-party dashboards with short-lived, scoped tokens over `https://` /
  `wss://`.

## Audit trail

An **off-the-truth-path** JSONL record of the run lifecycle (join-keys only, never
payloads or secrets) is available with `kx run --audit-log <path>`. Time lives
only on the wire DTO, never in the digest — so the audit trail never changes the
projection digest.
