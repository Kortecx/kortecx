---
id: chat
title: Chat
sidebar_label: Chat
description: Talk to the runtime — run a message through a model blueprint, attach files, read the reasoning, copy and rate answers, and keep a client-local history.
---

# Chat

The **Chat** section is the conversational front door to the runtime. Each message
you send **runs a blueprint** (a model recipe such as `kx/recipes/chat`); the
reply is that run's **committed result**, and the answer you see is the resolved
content — never a hash or a placeholder. When a serve provisions the agentic loop
(`kx/recipes/react`), a **Chat / Agent task** toggle appears so a single message
can drive the reason → tool → observe loop until it answers.

> Chat is presentation over the durable runtime. The thread itself is stored
> **client-local** (per gateway endpoint, in your browser); the *answers* are
> committed runs you can re-open, replay, and audit like any other run.

## Send a message

Type in the composer and press **Enter** to send (**Shift+Enter** inserts a
newline). The composer is a Markdown surface, and assistant replies render as
Markdown. While a turn is in flight the **DAG-of-thought** can be shown (toggle it
in chat settings) — the live graph of the run executing.

Pick the model with the **model picker** below the thread. The model list is
discovered from the connected gateway (`ListModels`); an FFI-free serve shows an
honest empty state rather than a fake menu.

When the gateway has **no chat model provisioned**, chat shows an honest
"no model — connect one" notice (start a model with
`kx serve --features inference`) instead of silently echoing your prompt. You can
still choose the model-free `kx/recipes/echo` recipe in chat settings for a
deterministic round-trip — a deliberate choice the console honors as-is.

## Attach

The **attach** button (next to send) opens a menu:

- **Upload a file** — uploads the bytes to the gateway's content store
  (`PutContent`) and rides the message. Images preview inline; the chip shows the
  server-derived content reference.
- **Blueprint / Dataset / Tool / Context** — listed but **disabled until a future
  release**: attaching these as message *context* rides the context-bundle work.
  They are shown (not hidden) so the surface is honest about what is coming.

## Reading an answer

- **Reasoning.** If a model emits a leading `<think>…</think>` block, it is split
  into a collapsible **Reasoning** disclosure above the answer. The answer always
  renders; the disclosure is gated by a persisted *Show reasoning* setting. The
  reasoning is **already durably captured** in the committed result — the toggle is
  presentation only.
- **Copy.** Each settled answer has a **Copy** action that copies the answer text
  (not the reasoning block) to the clipboard.
- **Feedback.** 👍 / 👎 record your rating on the answer. See below.

## Feedback (👍 / 👎)

Rating an answer calls `SubmitFeedback`, which records a row into the gateway's
**`feedback.db`** sidecar: the rating, an optional note, and advisory context (the
backing blueprint, the model, and the run that produced the answer). Re-rating the
same answer **overwrites** your previous rating ("changed my mind") — the feedback
id is derived server-side from the message and your identity, so you cannot forge
or duplicate it.

`feedback.db` is a **rebuildable-to-empty** sidecar: it is never journaled, never
part of the canonical digest, and never gates execution. Dropping it loses product
signal, never truth.

Inspect or export the collected feedback with the CLI or the SDKs:

```bash
# Record feedback (the console does this for you)
kx feedback submit --rating up --message-id <answer-id> --instance <run-hex16>

# Read it back (newest-first, paginated)
kx feedback list --limit 50
kx feedback list --instance <run-hex16> --json
```

```python
from kortecx import KxClient

with KxClient("http://127.0.0.1:50051", token="…") as kx:
    kx.submit_feedback("up", message_id="a1", instance_id=run_hex)
    for row in kx.list_feedback(limit=50).rows:
        print(row.rating, row.message_id, row.model_id)
```

```typescript
import { KxClient } from "@kortecx/sdk";

const kx = new KxClient("http://127.0.0.1:50051", { token });
await kx.submitFeedback({ rating: "up", messageId: "a1", instanceId: runHex });
const page = await kx.listFeedback({ limit: 50 });
```

A serve that predates feedback answers `UNIMPLEMENTED`; the console hides the
control, and the CLI/SDK report it honestly.

## Naming, history, and export

- **Auto-name.** A fresh thread is named from its first message; edit the name any
  time and your choice is kept (auto-naming never overrides a name you set).
- **History.** Every thread autosaves to a per-endpoint, client-local history;
  open the **History** slide-over to restore or delete a past chat.
- **Export.** **Export** downloads the current thread as a self-describing JSON
  file (messages, run attribution, and content references — never transient
  preview URLs).

## See also

- [Agent runner](./agent-runner.md) — how a message becomes an agentic run.
- [Reading run outputs](./reading-run-outputs.md) — how committed answers resolve
  to text across the console.
- [Blueprint builder](./blueprint-builder.md) — author the workflows chat runs.
