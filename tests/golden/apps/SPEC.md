# `kortecx.app/v1` envelope — cross-surface golden corpus

`corpus.json` is the **byte-shape parity gate** for the App envelope. It pins the
exact **canonical** serialization of representative `kortecx.app/v1` envelopes so the
Rust (`kx-app`), Python (`kortecx.app`), and TypeScript (`@kortecx/sdk`) serializers
stay byte-for-byte identical (GR12 tri-surface).

Each entry is `{ "name": <case>, "canonical": <string> }` where `canonical` is the
exact bytes `to_canonical_json()` (Rust) / `json.dumps(env, sort_keys=True,
separators=(",",":"), ensure_ascii=False)` (Python) / the TS equivalent must emit.

## The canonical form (every surface MUST obey)

1. **Keys sorted** lexicographically at every object level — including the opaque
   `blueprint` sub-tree (it is re-canonicalized, not passed through). Relies on
   `serde_json`'s `preserve_order` feature being **off** (pinned by a unit test).
2. **Compact separators** — `,` between members, `:` between key and value, no
   whitespace.
3. **Integers only** — no floats anywhere (SN-8; identity bytes are integer-only).
   A float in any field fails validation.
4. **UTF-8** — non-ASCII emitted verbatim (`ensure_ascii=False`), `/` not escaped.
5. **Omit-empty** — an optional/empty field is omitted entirely (not `null`/`[]`/
   `{}`). `schema`, `name`, `version`, `blueprint` are always present.
6. **`schema`** is the literal `"kortecx.app/v1"` — readers fail closed on mismatch.
7. **`content_ref`** values are lowercase 64-hex.

The **pretty** form used by `kx app export` (`to_pretty_json`) is 2-space-indented +
sorted + a trailing newline; it round-trips to the same canonical bytes.

## The contract each surface tests

For every committed `canonical` string `s`: `parse(s)` → re-serialize canonical →
**must equal `s` byte-for-byte** (idempotent canonicalization). Because all three
surfaces run this against the SAME committed strings, any divergence in key order,
separators, number format, or escaping fails the gate.

## Cases

- `minimal` — only the required fields (`schema`/`name`/`version`/`blueprint`).
- `agentic` — an authored agentic `@`-step (model step + `tool_contract` + budget
  params) round-tripped inside the blueprint, plus `description` + `tags`.
- `full` — `references` (a multi-modal `media_type` context ref + a tool ref +
  a `skills` SkillRef with `instructions_ref` + a tool wish) + `steering_config`
  (model route + a guard) + `branch_handle`.

Regenerate by constructing the envelopes via the typed `kx-app` API and printing
`to_canonical_json()`; never hand-edit the `canonical` strings.
