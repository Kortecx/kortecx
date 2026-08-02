---
id: managing-secrets
title: Managing secrets
sidebar_label: Managing secrets
description: Store credentials in the local secret store, reference them by name from connectors, and keep their values out of journals, content and model context.
---

# Managing secrets

A **secret** is a credential — an API token, a webhook signing key, a connector
password — that a `kx serve` runtime needs at the moment it dials an external tool
or verifies an inbound request, but which must **never** be written down where it
could leak. Kortecx keeps these in a **local secret store**: a single file on the
host that the runtime reads transiently and discards.

The rule that makes it safe: a secret is always referenced **by name**, and the
**value** is resolved only at the connector transport — it reaches no journal, no
content store, no telemetry sink, and no model context (D81).

## How resolution works

You store a value under a name (`GITHUB_TOKEN`), then reference that **name**
everywhere — a connector registration, a trigger. When the runtime needs the
credential it resolves the name **transiently** at the transport boundary, injects
the value into that one dial, and forgets it. The lookup chain is:

1. **The local store** — `secrets.json` in the runtime's catalog directory, created
   owner-only (`0600`).
2. **Environment fallback** — if the store has no entry for the name, an environment
   variable of the **same name** still resolves. This keeps older `credential_ref`
   setups working unchanged and supports ephemeral / CI runtimes that inject secrets
   through the environment.

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

Setting or removing a secret **writes host credential material**, so the write path
is held to the same posture as every other privileged RPC: it is **authenticated**
and accepted only over a **loopback** bind. A remote client cannot write secrets
into a runtime's store. Listing returns **names only** — never values.

## The store file, and editing it by hand

The store is `secrets.json` in the runtime's catalog directory. The runtime creates
it owner-only (`0600`) and rewrites it atomically, so an interrupted write can never
leave you with a half-written store.

You can open it and add a credential yourself — the short form is all that is
required, and the runtime fills in the timestamps the next time it writes:

```json title="secrets.json"
{
  "GITHUB_TOKEN": "ghp_xxxxxxxxxxxxxxxxxxxx"
}
```

If you create the file by hand, set its permissions:

```bash
chmod 600 secrets.json
```

Two cases are refused at startup rather than repaired, and in both the file is left
exactly as you wrote it:

- **Readable by group or others.** The runtime reports the mode it found and the
  `chmod` to run. File permissions are the only protection these values have.
- **Does not parse.** The runtime reports the parse error. It will not recreate the
  file, because doing so would silently discard every credential in it.

In either case the runtime still starts; the secret RPCs report that the store is
unavailable, and name resolution falls back to the environment.

:::caution The values are plaintext
This is a local-first store, not a vault: anything running as your user can read the
file. Keep it on a machine you control, and prefer the environment fallback for CI
runners and other shared hosts.
:::

## The secret namespace across surfaces

Store, list, and remove secrets from whichever surface you operate from — the shape
is identical across the SDKs and the CLI.

```python title="Python — kx.secrets namespace"
import kortecx as kx

kx.secrets.set("GITHUB_TOKEN", "ghp_xxxxxxxxxxxxxxxxxxxx")  # write to the store
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
connector's transport, so the dialed tool authenticates with the stored value:

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

## Scoping a secret to an App's agentic loop

A [connector](./authoring-a-connector.md) dial you trigger yourself resolves its
`credential_ref` directly. When an **[App](./apps.md)** dials a credentialed
connector *inside its agentic loop* — the model proposes the tool, and the runtime
dials it on the model's behalf — the App must first **scope** which secrets that run
is allowed to resolve. An App declares this once, in its envelope:

- `references.connections` — a by-reference pointer to the connection the App uses.
- `guards.secret_scope` — the secret names the run's tool-firing warrant may resolve.

Running the App resolves the pointer against your own connection registry and grants
exactly that scope to the agentic warrant, so the model can dial the credentialed
connector — and **nothing else**. The scope is a least-privilege bound: a run can
only resolve the secrets it declared, enforced at the capability broker (D110.3).

```python title="Python — build, save, and run an App that dials a credentialed connector"
import kortecx as kx

# The blueprint is an agentic step granting gmail/search; with_gmail() points the App
# at the bundled Gmail connection AND adds KX_GMAIL_CREDENTIAL to guards.secret_scope.
app = (kx.app("gmail-triage")
       .blueprint(kx.flow().agent(
           "Search my unread mail with gmail/search, then summarize it.",
           tools=["gmail/search"]))
       .with_gmail())
app.save(handle="apps/local/gmail-triage")

# Running it via RunApp reads the STORED envelope (its connection + secret_scope) and
# dials the credentialed connector inside the agentic loop. `run_app` is a client method.
kx.default_client().run_app("apps/local/gmail-triage")
```

```ts title="TypeScript"
const app = kx.app("gmail-triage")
  .blueprint(kx.flow().agent(
    "Search my unread mail with gmail/search, then summarize it.",
    { tools: ["gmail/search"] }))
  .withGmail();
await app.save({ handle: "apps/local/gmail-triage" });

const handle = await kx.runApp("apps/local/gmail-triage");
```

```bash title="CLI"
# Register the bundled Gmail connection once, then run the App.
kx connections add --provider gmail
kx app run apps/local/gmail-triage
```

For a custom (non-bundled) connector, use `.with_connection(descriptor,
credential_ref)` / `.withConnection(...)` instead of `.with_gmail()` — the credential
is scoped automatically (pass `scope_secret=False` for a credential-less connection).

### Seeing what a run may resolve

Because the scope is a governance decision, it is **visible** in the run trace — the
tools a chain may fire and the secret **names** it may resolve travel with every
ReAct turn (names/refs only, never a value). Inspect them from any surface:

```bash title="CLI — the run trace shows the chain's grants"
kx react list --instance <run-id>
# turn 0  branch pending  …  grants[tools: gmail/search@1; secrets: KX_GMAIL_CREDENTIAL]
```

The Console renders the same as a **"Governed by:"** line on the agent-loop strip, so
an operator can confirm at a glance what a run was authorized to do — a dropped
capability axis is visible, never silent.

## OSS / Cloud line

OSS keeps secrets in a **local, single-node** store for a runtime you operate
yourself: the values are plaintext on disk, protected by file permissions, and never
leave the host. A hardened, multi-tenant **KMS / HSM-backed vault** — encryption at
rest, rotation, per-party scoping, audit, and break-glass — is a Cloud capability.
