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
- `grounded` — the datasets rail (`dataset_ref` + `cas_refs`) + a rule +
  `steering_config.tools.requested_grants` + `steering_config.context.dataset_refs`.
- `reach` — the `steering_config.tools.reach` selector (`inherit_principal`), sorted
  before `requested_grants`, proving the additive field's canonical placement.

Regenerate by constructing the envelopes via the typed `kx-app` API and printing
`to_canonical_json()`; never hand-edit the `canonical` strings.

---

# `kortecx.appbundle/v1` — portable App bundle golden corpus

`bundle_corpus.json` is the **byte-shape parity gate** for the portable App bundle
(the archive `kx app export --bundle` writes and `kx app import` reads). It pins the
exact canonical serialization of representative `kortecx.appbundle/v1` documents so
the Rust (`kx-appbundle`), Python (`kortecx.appbundle`), and TypeScript
(`@kortecx/sdk` appbundle) codecs stay byte-for-byte identical.

A bundle packages an App for portability: the **canonical envelope bytes** plus the
base64 closure of every content-store blob the App references (its `content_refs`).

Each entry is `{ "name": <case>, "bundle": <string> }` where `bundle` is the exact
canonical bundle bytes `AppBundle::to_json()` (and each SDK equivalent) must emit.

## The canonical bundle form (every surface MUST obey)

1. **All-string document** — the top level has NO numeric fields (any size is
   derivable), so integer/float encoding never diverges across surfaces.
2. **Keys sorted** lexicographically: `app_digest` · `blobs` · `envelope` · `schema`
   · `source_digest`. The `blobs` map keys (content refs) are sorted too.
3. **Compact separators** — `,` / `:`, no whitespace.
4. **`envelope`** is the App's **exact canonical envelope string, verbatim** (the
   same bytes `corpus.json` locks), embedded as a JSON string — reusing the proven
   envelope parity and sidestepping any outer-serializer number/escape risk.
5. **`blobs`** maps a 64-hex content ref → the blob bytes in **base64, STANDARD
   alphabet, `=`-padded, single-line** (never url-safe; never `\n`-wrapped).
6. **Omit-empty** — an empty `blobs` and an absent `source_digest` are omitted
   entirely (byte-invariant when unset). `app_digest`, `envelope`, `schema` always present.
7. **`schema`** is the literal `"kortecx.appbundle/v1"` — readers fail closed on mismatch.
8. **`app_digest` / `source_digest`** are lowercase 64-hex. `source_digest` is a
   lineage HINT (the digest an App was exported/cloned from), never authenticity —
   OSS has no signing; signed provenance is a Cloud concern.

## The contract each surface tests

For every committed `bundle` string `s`: `from_json(s)` → re-serialize canonical →
**must equal `s` byte-for-byte** (idempotent canonicalization). Because all three
surfaces run this against the SAME committed strings, any divergence in key order,
base64 alphabet, envelope escaping, or omit-empty fails the gate.

## Cases

- `empty-closure` — a minimal envelope, no blobs, no lineage (omit-empty for both).
- `single-blob` — one prompt body travels; the common export shape.
- `multi-blob` — a text rule + a **binary** blob (`[0,1,2,253,254,255]` → `AAEC/f7/`,
  exercising the STANDARD `/` alphabet) inserted out of order (locks ref sorting).
- `clone-lineage` — carries a `source_digest` (a clone/import records provenance).

Regenerate via `KX_REGEN_GOLDEN=1 cargo test -p kx-appbundle --test golden
regen_prints -- --nocapture`, then paste; never hand-edit the `bundle` strings.
