---
id: skills
title: Skills
sidebar_label: Skills
description: Package know-how as a declarative kortecx.skill/v1 bundle — instructions + tool wishes an App attaches; the server grants only wish ∩ grants ∩ fireable.
---

# Skills

A **skill** packages *know-how* — instructions plus the tools they expect — as a
declarative `kortecx.skill/v1` bundle an [App](./apps.md) attaches. Think of it
as a reusable operating manual for the agent: "how to triage an inbox", "how to
research and summarize", "how to digest a channel".

A skill is **declarative only**. It carries markdown instructions and a tool
**wish** set (`tool_id → version`) — never code, and never authority. There is
no warrant, grant, secret, or credential in a skill (validation refuses those
keys anywhere in the manifest, fail-closed). The executable leg is always an
out-of-process [connector](./authoring-a-connector.md) or a bundled runtime
capability.

**Attaching a skill grants nothing.** When you run a skill-bearing App, the
server intersects the skill's wishes against *your* grants and the live broker:

```text
granted = wish ∩ your grants ∩ fireable-on-this-serve
```

The survivors fold into the App's entry agentic step (they appear in the tool
menu and are fireable under the server-minted warrant); an unfulfillable wish is
dropped with a warning — the run proceeds honestly with what could be granted,
and a skill on its own can never mint authority. The instructions bind as
labeled context (`skill:<name>`) on the same step, durably and replayably.

## A skill pack

```text
skills/email-triage/
  skill.json          # the kortecx.skill/v1 manifest
  instructions.md     # the know-how (content-addressed at add time)
  README.md           # optional docs
```

```json title="skill.json"
{
  "schema": "kortecx.skill/v1",
  "name": "email-triage",
  "version": "1",
  "description": "Triage a Gmail inbox: search, read, and DRAFT replies.",
  "tags": ["email", "gmail"],
  "tools": { "gmail/search": "1", "gmail/read": "1", "gmail/draft": "1" }
}
```

Scaffold one with `kx new skill <name>` (offline), and gate it with the
declarative conformance harness: `just test-skill <pack-dir>` (external authors
run the same checks CI runs on the in-tree reference packs).

## CLI

```sh
kx new skill my-skill                     # scaffold a pack (offline)
kx skills add --dir skills/email-triage   # validate + add to your catalog
kx skills list
kx skills show --name email-triage        # wishes + the advisory "registered" bit
kx app new triager --from-blueprint bp.json --skill email-triage
kx app run triager
kx skills remove --name email-triage
```

## Python

```python
import kortecx as kx

with kx.KxClient() as client:
    client.skills.add(
        {
            "schema": "kortecx.skill/v1",
            "name": "research-summarize",
            "tools": {"retrieve": "1", "fs-read": "1"},
        },
        instructions="# Research\nRetrieve first; cite what you read.",
    )
    app = (
        kx.app("researcher")
        .blueprint(kx.flow().agent("Answer the question."))
        .skill(kx.Skill(name="research-summarize", instructions_ref=...))
    )
    client.save_app(app.to_envelope())
    client.run_app("apps/local/researcher", wait=True)
```

## TypeScript

```ts
import { KxClient, app } from "@kortecx/sdk";

const kx = new KxClient();
await kx.skills.add({
  manifest: {
    schema: "kortecx.skill/v1",
    name: "research-summarize",
    tools: { retrieve: "1", "fs-read": "1" },
  },
  instructions: "# Research\nRetrieve first; cite what you read.",
});
const form = await kx.skills.show("research-summarize"); // wishes + registered bits
await kx.saveApp(
  app("researcher")
    .blueprint(/* … */)
    .skill({ name: "research-summarize", instructionsRef: form!.summary.instructionsRef })
    .toEnvelope(),
);
await kx.runApp("apps/local/researcher", { wait: true });
```

## Console

**Integrations → Skills** lists your catalog (add / inspect / remove; each wish
shows whether *this* serve could currently fire it), and an App's detail page
carries a **Skills** rail to attach or detach catalog skills (a structure edit —
a locked App refuses it).

## The reference skills

The runtime ships three conformance-gated reference packs (see
`registry/index.json`):

| Skill | Wishes | Posture |
| --- | --- | --- |
| `research-summarize` | `retrieve@1`, `fs-read@1` | Grounded answers; no connector needed |
| `email-triage` | `gmail/search`, `gmail/read`, `gmail/draft` | Draft-not-send — sending stays a human act |
| `channel-digest` | `discord/list_channels`, `discord/read_channel` | Read-only; never posts |

Each demonstrates the boundary the format enforces: the *instructions* say what
the agent should do; the *wish set* is the hard line on what it can ever touch;
and your grants + the live broker decide what it actually gets.
