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

## Grounding with datasets (RAG)

When a chat turn **names a dataset**, the runtime answers grounded on **your own
documents** instead of the model's parametric memory. The flow is a new recipe,
`kx/recipes/chat-rag`: the server embeds your message, runs **HNSW top-k
retrieval** over that dataset, and folds the **exact retrieved document refs**
into the prompt — so the model reasons over the matched content and replies
grounded on it.

The folded context is **edge-free, durable, replayable, and exactly-once**: the
retrieved content refs become part of the committed run, so re-opening or
replaying the turn reproduces the same grounded answer. Grounding turns on only
**after you ingest a corpus** — there is nothing to retrieve against until then.

This needs an inference build **with retrieval** and a served model:

```bash
kx serve --features inference,hnsw --dev-allow-local
```

### Ingest first, then chat

Build the corpus, then name it on the chat turn:

```bash
# 1. Ingest your documents into a named dataset (text or a file).
kx datasets ingest my-notes --text "Kortecx serves world-mutating steps from their committed result on replay."
kx datasets ingest my-notes --file ./design-notes.md

# 2. Chat grounded on that dataset (a bare positional is the message).
kx chat "What does Kortecx do on replay?" --dataset my-notes --k 4
#    → grounded on 'my-notes' — the answer cites your ingested docs

# Ask without a dataset and it is a plain chat (no retrieval).
kx chat "Say hello"
```

The CLI prints an **honest grounding indicator** for each turn: *grounded on
'my-notes'* when retrieval succeeds, or *not found / empty → plain* when it
falls back. `--k N` bounds how many documents are retrieved (default 4).

### Python

```python
from kortecx import KxClient

with KxClient("http://127.0.0.1:50151", token="…") as kx:
    # Grounded — retrieves the top-k docs from `my-notes` and folds the refs in.
    answer = kx.chat("What does Kortecx do on replay?", dataset="my-notes", k=4)
    print(answer)

    # No dataset → an honest plain chat, never faked grounding.
    print(kx.chat("Say hello"))
```

`client.chat(prompt, dataset="…", k=4) -> str` is a thin wrapper over an invoke
of `chat-rag`; it returns the committed answer.

### TypeScript

```ts
import { KxClient } from "@kortecx/sdk";

const kx = new KxClient("http://127.0.0.1:50151", { token });

// Grounded on your dataset.
const answer = await kx.chat("What does Kortecx do on replay?", {
  dataset: "my-notes",
  k: 4,
});
console.log(answer);

// No dataset → plain chat.
console.log(await kx.chat("Say hello"));
kx.close();
```

`client.chat(prompt, { dataset, k }) -> Promise<string>` returns the answer.

### Honest degrade — grounding is never faked

If grounding cannot run, the turn answers as a **plain chat with a notice** — it
never invents citations or pretends to be grounded. The fallbacks are:

- **No dataset selected** — a normal chat turn.
- **Named dataset missing or empty** — answers plainly and tells you the dataset
  was not found or had nothing to retrieve.
- **No embedder** (a serve without an inference model) — there is nothing to
  embed the message with, so it degrades to plain chat. (Retrieval also needs the
  `hnsw` feature — see [Data Lab → Degraded states](./datasets.md#degraded-states).)

### Scores are display-only (SN-8)

Retrieval ranks hits by an approximate similarity **score**, but the score is
**display-only** — it never reaches run identity. Only the **exact content refs**
of the retrieved documents are folded into the prompt, and they are matched
downstream by exact hash. So the grounded turn stays deterministic and
replayable: the same ingested corpus yields the same folded refs, and the
build-order-sensitive ANN ranking never routes a `MoteId`. See
[Data Lab → Scores are display-only](./datasets.md#scores-are-display-only-sn-8)
and [Security → model proposes, runtime
enforces](./security.md#model-proposes-runtime-enforces).

## Reading an answer

- **Reasoning.** If a model emits a leading `<think>…</think>` block, it is split
  into a collapsible **Reasoning** disclosure above the answer. The answer always
  renders; the disclosure is gated by a persisted *Show reasoning* setting. The
  reasoning is **already durably captured** in the committed result — the toggle is
  presentation only.
- **Copy.** Each settled answer has a **Copy** action that copies the answer text
  (not the reasoning block) to the clipboard.
- **Feedback.** 👍 / 👎 record your rating on the answer. See below.

## Live token streaming

A model's tokens stream to the chat **as they generate** (time‑to‑first‑token),
so you see the answer build instead of waiting for the whole completion.

- **Advisory + out‑of‑band — the committed result is the authority.** The stream
  is *not* the durable fact. When the run commits, the console fetches the
  committed result (its content hash) and the bubble **reconciles** to it. A
  client that ignores the stream still polls and shows the identical committed
  answer; the journal, the canonical digest, and replay are **unchanged**.
- **Simple & vision chat.** Tokens stream straight into the answer bubble (an
  `aria-live` region — screen readers announce incrementally), then settle to the
  committed answer.
- **Agent mode.** The current turn's tokens stream into a muted **reasoning/acting
  trace** line while the chain runs (a tool turn's raw call envelope never poses
  as the answer); the committed final answer lands in the bubble when the chain
  settles.
- **Honest degrade.** A serve that predates streaming, or one built without a
  model, simply shows no live tokens — the answer still arrives via the poll path.

### CLI

```bash
# Stream the terminal model mote's tokens to stdout, then resolve the result.
kx invoke kx/recipes/chat --args '{"message":"explain backpressure"}' --stream
```

`--stream` never hangs on a token‑less (non‑model) terminal — it resolves the
committed result concurrently. Add `--json` or `--out <file>` to also capture the
committed bytes.

### SDK

The stream is subscribed by `instance_id` **and** the model `mote_id` (the run's
terminal mote, or — for an agent chain — the in‑flight turn). A subscriber must
**own the run** (the same gate as the event stream); the `mote_id` is the
unguessable server‑derived key that selects which model mote to stream.

```ts
// TypeScript — browser path (WebSocket bridge) or native gRPC.
for await (const chunk of kx.wsTokens(instanceId, terminalMoteId)) {
  process.stdout.write(chunk.text);          // concatenation == the committed result
  if (chunk.done) break;
}
```

```python
# Python — sync or async; ws_stream_model_tokens for the WS bridge.
for chunk in kx.stream_model_tokens(instance_id, terminal_mote_id):
    print(chunk.text, end="", flush=True)
    if chunk.done:
        break
```

A serve without the streaming surface answers `UNIMPLEMENTED` (gRPC) or closes the
WebSocket — the SDK degrades to the poll path.

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
