---
id: settings
title: Settings
sidebar_label: Settings
description: The Workspace view of a gateway — its resolved model, endpoints, durable paths, limits, and the auth/TLS posture, projected read-only and never exposing a secret.
---

# Settings

The **Settings** view is a read-only **Workspace** projection of the gateway you
are connected to: its resolved model, its endpoints, its durable paths, its
limits, and the **auth / TLS posture** it is running under. It answers "what is
this server actually configured as?" without ever returning a secret.

It is backed by a single RPC, `GetServerInfo`, which projects the **non-secret**
server configuration. Every field traces to the gateway's resolved configuration
— nothing here is fabricated, and nothing here is a credential.

## What it shows

`GetServerInfo` reports the resolved configuration in groups:

- **Model** — the resolved model id and its on-disk path (the GGUF the live loop
  serves), or an honest empty state on an FFI-free serve.
- **Endpoints** — the gRPC, WebSocket, web console, and metrics endpoints.
- **Durable paths** — the content, journal, and catalog directories (the same
  layout `kx serve` prints in its startup banner).
- **Limits** — the lease duration and the content/payload size limits.
- **CORS** — the browser-access allowlist (the explicit origins, never a
  wildcard — see [Security → deny-by-default](./security.md#deny-by-default)).
- **Auth & TLS posture** — an `auth_mode` label (`deny-all` · `dev-local` ·
  `token`) and a `tls_enabled` boolean. This is a **posture label, not a
  secret**: it tells you *how* the server authenticates, never *with what*.
- **Feature flags** — which build features are compiled in (`hnsw`,
  `inference`, `console`, `vision`).
- **Audit** — whether an audit log is configured (see
  [Security → audit trail](./security.md#audit-trail)).

## CLI

```bash
# Human-readable summary of the connected gateway.
kx info

# Machine-readable form (byte-shape parity with the SDKs).
kx info --json
```

`kx info` is the quickest way to confirm a serve is configured the way you
expect — the resolved model is the one you meant to load, retrieval is compiled
in (`hnsw`), and the auth posture is what you intended before you expose it.

## Python

```python
from kortecx import KxClient

with KxClient("http://127.0.0.1:50151", token="…") as kx:
    info = kx.get_server_info()
    print(info.model_id, info.model_path)
    print(info.auth_mode, info.tls_enabled)   # posture, never a token or key
    print(info.features)                       # hnsw / inference / console / vision
```

## TypeScript

```ts
import { KxClient } from "@kortecx/sdk";

const kx = new KxClient("http://127.0.0.1:50151", { token });
const info = await kx.getServerInfo();
console.log(info.modelId, info.modelPath);
console.log(info.authMode, info.tlsEnabled);
kx.close();
```

## Non-secret, authenticated-caller contract

`GetServerInfo` is governed by an **authenticated caller** (the same bearer-token
gate as every other RPC — an unauthenticated `kx serve` answers no one; see
[Security](./security.md)). And it **never returns a secret**:

- **No bearer token** — the auth posture is a label (`deny-all` / `dev-local` /
  `token`), never the token value or the party map.
- **No TLS key** — `tls_enabled` is a boolean; the certificate and private key
  never cross the wire.

Identity stays server-derived (see [Security → identity is
server-derived](./security.md#identity-is-server-derived)): this view is a
projection of the server's own resolved configuration, not a client-asserted
one. It is read-only — changing the server's configuration is a restart-time
concern (flags and environment), not an RPC.

## Degraded states

- **Older gateway.** A gateway that predates `GetServerInfo` answers
  `UNIMPLEMENTED`; the CLI and SDKs report it honestly rather than guessing.
- **FFI-free serve.** A build without an inference backend has no resolved model
  — the model fields are an honest empty state, and the `inference` feature flag
  reads false.

## See also

- [Security](./security.md) — the deny-by-default posture, the auth/TLS defaults,
  and the audit trail this view reflects.
- [Models](./models.md) — the catalog of models serving the gateway.
- [Observability](./observability.md) — the live, telemetry-derived view of what
  the server is doing (this view is its static configuration).
