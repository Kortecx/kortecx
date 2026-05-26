# WARNINGS.md — correctness-relevant warnings log

> Artifact mandated by the **Engineering Discipline / Rule 2 — the warning
> protocol**. Every correctness-relevant warning surfaced during testing or
> validation gets a four-field entry here. Style warnings do **not** belong
> here — they are caught and fixed under the existing
> `cargo clippy --workspace --all-targets -- -D warnings` CI gate without
> ceremony.

## What counts as "correctness-relevant"

A warning is correctness-relevant — and gets an entry below — if it touches
any of these paths:

- The **Mote lifecycle** (`kx-mote`: `Mote`, `MoteId`, `MoteGraph`,
  `MoteDef`, `NdClass`, `EffectPattern`, `EdgeKind`, `AttemptState`,
  `transition`).
- The **journal write path** (`kx-journal`: `Journal` trait, `JournalEntry`,
  the SQLite atomicity sweep, the in-memory backend).
- **Commit / repudiate logic** (`kx-executor`'s `commit_protocol.rs`,
  `lifecycle.rs`, `refusal.rs`; `kx-journal`'s dedupe-by-key path).
- **Idempotency-key derivation** (anywhere `idempotency_key` is computed
  per D38 §1).
- **Recovery replay** (`kx-projection` fold path; `kx-executor`
  recovery wiring; the 9-cell cross-product surface).
- **Any `unused_must_use` / unhandled-`Result` / unreachable-code /
  lifetime warning** that surfaces on any of the above paths.

Everything else — formatting, naming, pedantic clippy lints with no
correctness bearing — is a **style warning**. Fix it under the normal
clippy gate; do not write an entry here.

## Entry shape (per Rule 2)

Each entry follows this four-field shape. Keep entries tight; long-form
debugging notes belong in the PR description that introduced the fix, with a
link from the entry back to that PR.

```markdown
### YYYY-MM-DD — <one-line summary>

- **Identified**: what surfaced, where (file:line), at what stage (which
  test, which CI job, which manual run), and which path it touches from the
  list above.
- **Debug report**: root cause — what was actually wrong, not the symptom.
  One paragraph max. If a fuller trace is needed, link to the PR.
- **Fix**: what changed, in which file, and **why the change is
  functionally neutral** (no public-API drift; no behavior change observable
  to a downstream caller). Reference the commit SHA once merged.
- **Improvement**: how this makes the agentic runtime more solid going
  forward — what class of latent bug it ruled out, which invariant it
  pinned, or which seam it hardened.
```

A recurring class of warning in this log is a **design smell** and triggers
a Rule 1 review of the relevant trait seam — not just another entry.

## Entries

_(none yet — initial template introduced as part of the Engineering
Discipline / Phase A adoption PR.)_
