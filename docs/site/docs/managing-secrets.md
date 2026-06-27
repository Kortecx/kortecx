---
id: managing-secrets
title: Managing secrets
sidebar_label: Managing secrets
description: Store credentials in the OS keychain-backed local secret store, reference them by name from connectors, and keep their values out of journals, content and model context (MM-3 / D110 / D81).
---

# Managing secrets

A **secret** is a credential — an API token, a webhook signing key, a connector
password — that a `kx serve` runtime needs at the moment it dials an external tool
or verifies an inbound request, but which must **never** be written down where it
could leak. Kortecx keeps these in a **local secret store**: an OS keychain-backed
vault (MM-3 / D110) that the runtime reads transiently and discards.

The rule that makes it safe: a secret is always referenced **by name**, and the
**value** is resolved only at the connector transport — it reaches no journal, no
content store, no telemetry sink, and no model context (D81).

## How resolution works

You store a value under a name (`GITHUB_TOKEN`), then reference that **name**
everywhere — a connector registration, a trigger. When the runtime needs the
credential it resolves the name **transiently** at the transport boundary, injects
the value into that one dial, and forgets it. The lookup chain is:

1. **OS keychain** — the durable, encrypted-at-rest store (macOS Keychain, the
   Linux Secret Service / libsecret, Windows Credential Manager).
2. **Environment fallback** — if no keychain entry exists, an environment variable
   of the **same name** still resolves. This keeps older `credential_ref` setups
   working unchanged (back-compat) and supports ephemeral / CI runtimes that inject
   secrets through the environment.

Either way only the **name** ever travels through the registry, the journal, or the
model's context. The value lives for the duration of one transport call.

:::tip Why by-name, never by-value
A connector registration, a trigger definition, and every durable run fact carry
the secret's **name** — never its bytes. Resolving late, at the transport, means an
injected credential can never land in a journal, a content payload, a staged result,
a `MoteId`, or a telemetry record (D81). It is the same secret-by-reference contract
a [connector](./authoring-a-connector.md#secrets-by-reference-d81) relies on.
:::

## Writing is loopback-only + authenticated

Setting or removing a secret **mutates the host's keychain**, so the write path is
held to the same posture as every other privileged RPC: it is **authenticated** and
accepted only over a **loopback** bind. A remote client cannot write secrets into a
runtime's keychain. Listing returns **names only** — never values.

## The secret namespace across surfaces

Store, list, and remove secrets from whichever surface you operate from — the shape
is identical across the SDKs and the CLI.

```python title="Python — kx.secrets namespace"
import kortecx as kx

kx.secrets.set("GITHUB_TOKEN", "ghp_xxxxxxxxxxxxxxxxxxxx")  # write to the keychain
kx.secrets.list()                                            # names only, never values
kx.secrets.remove("GITHUB_TOKEN")
```

```ts title="TypeScript — client.secrets namespace"
await client.secrets.set("GITHUB_TOKEN", "ghp_xxxxxxxxxxxxxxxxxxxx");
await client.secrets.list();   // names only, never values
await client.secrets.remove("GITHUB_TOKEN");
```

```bash title="CLI (operator)"
kx secrets set --name GITHUB_TOKEN --value ghp_xxxxxxxxxxxxxxxxxxxx
kx secrets list                       # prints names only
kx secrets rm --name GITHUB_TOKEN
```

## Using a secret from a connector

A stored secret becomes useful when a [connector](./authoring-a-connector.md) dials
a tool that needs to authenticate. Register the connector with a `credential_ref`
naming the secret — the runtime resolves it at dial and injects it into that
connector's transport, so the dialed tool authenticates with the keychain-stored
value:

```python title="Python"
import kortecx as kx

kx.secrets.set("GITHUB_TOKEN", "ghp_xxxxxxxxxxxxxxxxxxxx")

# only the NAME "GITHUB_TOKEN" is stored on the connection; the value is resolved
# transiently at dial and never journaled
kx.connections.add("gh", endpoint="npx", args=["-y", "@some/github-mcp"],
                   credential_ref="GITHUB_TOKEN")
```

```ts title="TypeScript"
await client.secrets.set("GITHUB_TOKEN", "ghp_xxxxxxxxxxxxxxxxxxxx");

await client.connections.add({ name: "gh", endpoint: "npx",
  args: ["-y", "@some/github-mcp"], credentialRef: "GITHUB_TOKEN" });
```

```bash title="CLI"
kx secrets set --name GITHUB_TOKEN --value ghp_xxxxxxxxxxxxxxxxxxxx
kx connections add --name gh --command "npx -y @some/github-mcp" --credential-ref GITHUB_TOKEN
```

The connector itself must never echo this value back — see the connector
[security contract](./authoring-a-connector.md#what-a-connector-must-implement).

## OSS / Cloud line

OSS keeps secrets in the **local, single-node** keychain store for a runtime you
operate yourself. A hardened, multi-tenant **KMS / HSM-backed vault** — rotation,
per-party scoping, audit, and break-glass — is a Cloud capability (D129).
