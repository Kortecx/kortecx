---
id: authoring-a-trigger
title: Authoring a trigger
sidebar_label: Authoring a trigger
description: Bind an inbound source (webhook / cron / grpc) to a recipe OR a saved App so an event starts a fresh durable run — with 5-field cron + timezones, per-trigger HITL approval, and per-trigger HMAC/bearer auth on the untrusted webhook surface (D113 / T-APP-TRIGGER-TARGET).
---

# Authoring a trigger

A **trigger** binds an inbound source to a **recipe handle**: when an event arrives,
the runtime starts a fresh durable run for that recipe (D113). It is how a `kx serve`
runtime reacts to the outside world — a schedule firing, a webhook landing, a client
poke — instead of only running on an explicit `Invoke` from an operator.

A trigger is the run's **origin**: the runtime starts the run through the **same
Invoke path** as a manual submission, so everything downstream — warrants, the
journal, replay, exactly-once — behaves identically. Each event carries an
idempotency key, so a **replayed event is deduped** to its original run rather than
starting a second one.

There are three inbound kinds:

| Kind | Source | Auth surface |
|---|---|---|
| `webhook` | an inbound HTTP POST | **untrusted** — HMAC / bearer required off loopback |
| `cron` | an interval (seconds) **or** a 5-field crontab expression in a timezone | internal scheduler — no inbound auth |
| `grpc` | an authenticated gRPC `FireTrigger` | the gateway's existing bearer gate |

A trigger targets **either a recipe handle or a saved App**. An **App target** fires a
credentialed [App](./apps.md) unattended — its declared `references.connections` +
`guards.secret_scope` resolve on the run so an integration (Gmail, Slack, Discord,
Notion…) can be dialed inside the agentic loop, exactly as an interactive `RunApp`. This
is how a persona-App reacts to the world on a schedule or an event.

## The webhook is the untrusted-inbound surface

A webhook accepts a POST from anywhere you expose the listener, so it is the one
trigger surface that **must** authenticate the caller before it starts a run. Each
trigger pins its own posture:

- **`hmac_sha256`** — the caller signs the **raw request body** with a per-trigger
  shared secret and sends the hex digest in `X-Kx-Signature-256: sha256=<hex>`. The
  runtime recomputes the HMAC over the exact bytes and compares by exact equality.
- **`bearer`** — the caller presents a per-trigger bearer token.
- **`none`** — accepted **only** when the webhook listener is bound to loopback. Off
  loopback, a `none`-auth webhook is refused.

On top of auth, every webhook enforces a **payload size cap** and a **per-trigger
rate limit**, so a flood or an oversized body is rejected before it can start a run.

:::tip This is the minimal local trigger
OSS ships a minimal, single-node trigger listener you run yourself. A hosted,
horizontally-scaled **trigger gateway** — managed ingress, delivery retries,
multi-tenant routing — is a Cloud capability (D129).
:::

## Starting serve with the webhook listener

The webhook listener is **off by default** (deny-by-default — see
[Security](./security.md#deny-by-default)). Opt in with an explicit `addr:port`:

```bash
kx serve --dev-allow-local --webhook-listen 127.0.0.1:50190
```

Store the signing secret in the [local secret store](./managing-secrets.md) and
reference it by name — never inline the value into the trigger:

```bash
kx secrets set --name HOOK_SECRET --value <hex-shared-secret>
```

## The trigger namespace across surfaces

Add, test, list, and remove triggers from whichever surface you operate from — the
shape is identical across the SDKs and the CLI. `recipe` is a recipe handle (for
example `kx/recipes/chat` or `kx/recipes/react`); `secret_ref` names a stored
secret, resolved transiently (D81), never inlined.

```python title="Python — kx.triggers namespace"
import kortecx as kx

# a cron trigger — fire kx/recipes/chat every hour (interval in seconds)
kx.triggers.add(name="daily-digest", kind="cron",
                recipe="kx/recipes/chat", schedule="3600")

# a webhook trigger — HMAC-authenticated, secret resolved by name
kx.triggers.add(name="alert", kind="webhook", recipe="kx/recipes/react",
                auth="hmac_sha256", secret_ref="HOOK_SECRET")

kx.triggers.test("alert", payload='{"prompt":"diagnose the alert"}')  # dry-run a payload
kx.triggers.list()
kx.triggers.remove("alert")
```

```ts title="TypeScript — client.triggers namespace"
await client.triggers.add({ name: "daily-digest", kind: "cron",
  recipe: "kx/recipes/chat", schedule: "3600" });

await client.triggers.add({ name: "alert", kind: "webhook",
  recipe: "kx/recipes/react", auth: "hmac_sha256", secretRef: "HOOK_SECRET" });

await client.triggers.test("alert", { payload: '{"prompt":"diagnose the alert"}' });
await client.triggers.fire("alert", { payload: '{"prompt":"diagnose the alert"}' });  // start a real run
await client.triggers.list();
await client.triggers.remove("alert");
```

```bash title="CLI (operator)"
kx triggers add --name daily-digest --kind cron --recipe kx/recipes/chat --schedule 3600
kx triggers add --name alert --kind webhook --recipe kx/recipes/react \
  --auth hmac_sha256 --secret-ref HOOK_SECRET
kx triggers list
kx triggers test --name alert --payload '{"prompt":"diagnose the alert"}'   # dry-run
kx triggers fire --name alert --payload '{"prompt":"diagnose the alert"}'   # start a real run
kx triggers rm --name alert
```

`test` validates a payload against the trigger **without** starting a durable run —
a "does this trigger route" check. `fire` (and a real inbound event) starts a fresh,
journaled, replayable run.

## Firing an App on a schedule

Point a trigger at a **saved App** with `app` instead of `recipe` — the App fires
unattended with its integrations + secret scope resolved. Pair it with a **5-field cron
expression** and a **timezone** for real scheduling, and turn on **`require_approval`**
so an unattended App holds any irreversible action for an operator grant (D114 — see
[Security](./security.md)):

```python title="Python — an App on a weekday-morning schedule, HITL-gated"
import kortecx as kx

# "standup-digest" is a saved, credentialed App (see Apps → author + .save()); it
# declares .with_slack() + .secrets("KX_SLACK_CREDENTIAL") so it can post unattended.

# 09:00 on weekdays, New York time; hold irreversible posts for approval
kx.triggers.add(name="standup", kind="cron", app="standup-digest",
                schedule="0 9 * * 1-5", timezone="America/New_York",
                require_approval=True)
```

```ts title="TypeScript"
await client.triggers.add({ name: "standup", kind: "cron", appHandle: "standup-digest",
  scheduleSpec: "0 9 * * 1-5", timezone: "America/New_York", requireApproval: true });
```

```bash title="CLI"
kx triggers add --name standup --kind cron --app standup-digest \
  --cron "0 9 * * 1-5" --timezone America/New_York --require-approval
```

- **Target** — pass exactly one of `recipe` / `app` (`--recipe` | `--app`). An App
  target needs the App-run seam (`kx serve --features mcp-gateway`, the default).
- **Schedule** — `schedule` accepts either interval seconds (`"300"`) **or** a standard
  5-field crontab expression (`min hour dom month dow`). A cron expression is evaluated
  in `timezone` (any IANA name; empty ⇒ UTC), DST-correct.
- **Approval** — `require_approval` withholds irreversible (world-mutating) tool actions
  until an operator grants them from the **Security → approvals** surface; read-only
  actions still auto-proceed. Recommended for any unattended webhook/cron App that can
  take an irreversible action.

An App-target trigger runs under the **registrant's party** (D102.2) — the same identity
that saved the App and registered its connections — so a webhook caller can start the run
but never escalate beyond what the registrant could `RunApp` themselves (SN-8).

## POSTing a signed webhook

Once `kx serve --webhook-listen 127.0.0.1:50190` is up and the `alert` trigger
exists, deliver an event by POSTing to `/trigger/<name>` with a body signed by the
shared secret. Compute the signature over the **raw body bytes**:

```bash
SECRET="<hex-shared-secret>"          # the value behind HOOK_SECRET
BODY='{"prompt":"diagnose the alert"}'
SIG=$(printf '%s' "$BODY" | openssl dgst -sha256 -hmac "$SECRET" -hex | awk '{print $2}')

curl -sS http://127.0.0.1:50190/trigger/alert \
  -H "Content-Type: application/json" \
  -H "X-Kx-Signature-256: sha256=$SIG" \
  -H "X-Kx-Idempotency-Key: alert-2026-06-27-001" \
  --data-raw "$BODY"
```

The signature header must be `sha256=` followed by the hex HMAC-SHA256 of the exact
bytes you send — sign the **raw** body, before any reformatting.

The response carries the run's origin and whether it was deduped:

```json
{ "instance_id": "a1b2c3…", "deduped": false }
```

- `instance_id` is the server-derived id of the run this event started (or matched).
- `deduped` is `true` when this event collapsed onto an existing run instead of
  starting a new one.

Dedup is automatic for a re-delivered event, and you can make it **explicit** with
the `X-Kx-Idempotency-Key` header: two POSTs carrying the same key start exactly one
run — the second returns the first run's `instance_id` with `deduped: true`. This is
what makes at-least-once webhook delivery safe to point at a side-effecting recipe.

## OSS / Cloud line

OSS runs a single-node webhook + cron listener you operate yourself. Managed inbound
ingress — TLS termination at the edge, delivery retries with backoff, per-tenant
routing, and a horizontally-scaled trigger gateway — is a Cloud capability (D129).
