//! The pure [`assemble`] function — resolves a Mote's explicit dependency
//! closure (Data-edge parent `result_ref`s + warrant `tool_grants`) into
//! byte-deterministic [`crate::AssembledContext`].

use std::collections::BTreeSet;
use std::fmt::Write as _;

use bytes::Bytes;
use kx_content::{ContentRef, ContentStore};
use kx_mote::{decode_context_items, ConfigKey, EdgeKind, Mote, MoteId, CONTEXT_ITEMS_KEY};
use kx_projection::Snapshot;
use kx_tool_registry::{InputSchema, ParamType, ToolDef, ToolRegistry};
use kx_warrant::{ToolGrant, WarrantSpec};

use crate::errors::AssemblyError;
use crate::types::{AssembledContext, AssembledItem};

/// The longest tool `description` rendered into the menu. `def.description` is the
/// SOLE uncapped tool-menu field (name/version/`Inputs:`/`Example:` are all bounded),
/// so a pathological MCP tool with a multi-KB description would dominate the prompt.
/// Reuses the file's `SNIPPET_MAX = 400` rerank precedent (`render_rerank_prompt`). A
/// description at or under the cap renders BYTE-IDENTICALLY (the bundled tools are all
/// well under it); a longer one is truncated on a char boundary with a single-char
/// ellipsis. Advisory prompt bytes only — off `source_ref` / the journal / `MoteId`, so
/// digest-neutral (see the [`tool_menu_text`] doc).
const DESCRIPTION_MAX: usize = 400;

/// Cap `description` to [`DESCRIPTION_MAX`] chars, ellipsizing when it is longer.
/// Char-count semantics (mirrors `render_rerank_prompt`'s `SNIPPET_MAX` take) so a
/// multi-byte UTF-8 description is never split mid-codepoint. `≤ cap` ⇒ returned
/// unchanged, so the common case (every bundled tool) is byte-identical.
fn cap_description(description: &str) -> String {
    if description.chars().count() <= DESCRIPTION_MAX {
        return description.to_string();
    }
    let head: String = description.chars().take(DESCRIPTION_MAX).collect();
    format!("{head}…")
}

/// PR-6a/PR-1: render the tool-menu text the model sees for a granted tool — the
/// EXACT callable name (`grant_id`, PR-1/BUG-32 name-steering), then its description
/// PLUS, when the tool declares a typed `inputSchema`, a deterministic
/// one-line-per-parameter block (name · type · required/optional). This is the
/// "suggest better tools/steps" lever: the model proposes well-formed calls with
/// the granted name (a dialed/local tool is registered NAMESPACED `<server>/<remote>`
/// — leading with `name:` steers the model to emit it verbatim) instead of guessing.
/// The runtime still validates the proposed name+args fail-closed against the grant
/// set and the SAME schema (SN-8 — advisory in, exact enforced). The `name:` line is
/// advisory prompt bytes only: it lands in `AssembledItem.bytes` (read by the model),
/// never in `source_ref`/the journal/`MoteId`, so it moves no committed-fact digest.
fn tool_menu_text(grant_id: &str, def: &ToolDef) -> String {
    // Lead with the EXACT callable name AND the pinned version (PR-1/BUG-32 name-
    // steering + RC3): the call envelope is `{"name":…,"version":…,"args":…}`, and the
    // runtime matches the version EXACTLY (SN-8). Without the version in the menu a
    // model guesses (e.g. emits `"1.0"` for a `"1"` grant) and the call is refused —
    // observed live on Ollama gemma3 (the llama.cpp grammar enumerates the pair, but
    // the Ollama honest-degrade path has only the prompt to go on).
    let head = format!("name: {grant_id}\nversion: {}\n", def.tool_version.0);
    // Cap the SOLE uncapped menu field (name/version/Inputs/Example are all bounded);
    // `Inputs:`/`Example:` stay byte-intact below.
    let description = cap_description(&def.description);
    let Some(schema) = &def.input_schema else {
        return format!("{head}{description}");
    };
    let mut text = format!("{head}{description}");
    text.push_str("\nInputs:");
    for p in &schema.params {
        let ty = match &p.ty {
            ParamType::Int { .. } => "integer",
            ParamType::Bytes { .. } => "bytes",
            ParamType::Str { .. } => "string",
            ParamType::Bool => "bool",
            ParamType::Enum { .. } => "enum",
        };
        let req = if p.required { "required" } else { "optional" };
        text.push_str("\n  - ");
        text.push_str(&p.name);
        text.push_str(" (");
        text.push_str(ty);
        text.push_str(", ");
        text.push_str(req);
        text.push(')');
    }
    // PR-3 (A3a): a deterministic, well-formed `Example:` call so the model emits
    // a syntactically-correct args bag with the RIGHT keys on the first try
    // (the §2.246 finding: a capable model guessed `{"text":…}` for a `q` param).
    // Required params only (the minimal valid call), declared order, type-keyed
    // placeholders. Advisory prompt bytes only (digest-neutral — see the fn doc);
    // the runtime still validates the model's REAL proposal fail-closed (SN-8).
    text.push_str("\nExample: ");
    text.push_str(&example_call_json(schema));
    text
}

/// Render a deterministic, well-formed example JSON args object over a schema's
/// REQUIRED params (declared order; optionals omitted to model the minimal valid
/// call). PURE + total: type-keyed constant placeholders (`Int` → an in-range
/// integer; `Bool` → `false`; `Enum` → the `BTreeSet`-least allowed value;
/// `Str`/`Bytes` → a quoted placeholder), no map re-sort (declared order is the
/// tool's identity contract), no clock/RNG. Zero required params → `{}`.
fn example_call_json(schema: &InputSchema) -> String {
    let mut parts: Vec<String> = Vec::new();
    for p in schema.params.iter().filter(|p| p.required) {
        let val = match &p.ty {
            ParamType::Int { min, max } => match (min, max) {
                (Some(lo), _) => lo.to_string(),
                (None, Some(hi)) if *hi < 0 => hi.to_string(),
                _ => "0".to_string(),
            },
            ParamType::Bytes { .. } => "\"<bytes>\"".to_string(),
            ParamType::Str { .. } => "\"<string>\"".to_string(),
            ParamType::Bool => "false".to_string(),
            ParamType::Enum { allowed } => allowed
                .iter()
                .next()
                .map_or_else(|| "\"<enum>\"".to_string(), |v| format!("\"{v}\"")),
        };
        // Param names are declared identifiers; the example is advisory prompt
        // text (never parsed back), so a literal-quoted key is sufficient.
        parts.push(format!("\"{}\": {val}", p.name));
    }
    format!("{{{}}}", parts.join(", "))
}

/// RC3 (T-REACT-TOOL-MENU): render the advisory tool MENU a tool-eligible ReAct
/// turn shows the model so it PROPOSES well-formed calls autonomously. RC2's
/// grammar only CONSTRAINS a proposal once made; the menu is what makes the model
/// propose at all. Pure, total and deterministic: it iterates `grants` in
/// `BTreeSet` `(tool_id, tool_version)` order, looks each one up, and renders it
/// through the SAME `tool_menu_text` the context-assembly path uses (name
/// steering, typed params and a worked example) so the harness menu and the
/// live-serve menu can never drift. A grant the registry cannot resolve degrades
/// to a name, version and envelope shape only (fail-soft — it never panics a
/// dispatch and never silently omits a granted tool). It renders ONLY `grants`
/// (= `warrant.tool_grants`), so no UNGRANTED tool can ever leak. The output is
/// advisory prompt bytes only (SN-8): the runtime still validates the model's
/// REAL proposal fail-closed (`kx_toolcall::parse_tool_call` + `validate_args`).
/// Empty `grants` yields `""`, so the dispatch menu gate prepends nothing and the
/// canonical no-tools demo stays byte-unchanged.
#[must_use]
pub fn render_tool_menu(grants: &BTreeSet<ToolGrant>, registry: &dyn ToolRegistry) -> String {
    if grants.is_empty() {
        return String::new();
    }
    let mut out = String::from("You can call the following tools:\n");
    for grant in grants {
        out.push('\n');
        if let Some(def) = registry.lookup(&grant.tool_id, &grant.tool_version) {
            out.push_str(&tool_menu_text(&grant.tool_id.0, &def));
        } else {
            // Fail-soft: the registry could not resolve a granted tool (a
            // BUG-33-class id skew or a registry-open race). Emit the name,
            // version and the canonical envelope shape so the grant is never
            // silently dropped from the menu. Plain `push_str` (no `format!`)
            // keeps the clippy `format_push_string` lint clean.
            out.push_str("name: ");
            out.push_str(&grant.tool_id.0);
            out.push_str("\nversion: ");
            out.push_str(&grant.tool_version.0);
            out.push_str("\n(schema unavailable — call as {\"tool_call\":{\"name\":\"");
            out.push_str(&grant.tool_id.0);
            out.push_str("\",\"version\":\"");
            out.push_str(&grant.tool_version.0);
            out.push_str("\",\"args\":{}}})");
        }
        out.push('\n');
    }
    out
}

/// RC4c-2b: bound a listwise-rerank turn's output — the permutation array (`n`
/// indices ≈ 6 tokens each) PLUS generous headroom for a model that reasons before
/// answering (e.g. Gemma's `<|channel>…<channel|>` preamble). Too tight a cap
/// truncates the decode before the array and the parse fail-closes to input order;
/// still bounded so a runaway decode can't burn the budget. The SHARED renderer for
/// BOTH the authored-DAG harness rerank (`kx-model-harness::rag`) and the LIVE serve
/// rerank turn (`kx-gateway::model_exec`), so the two paths never drift (the
/// `render_tool_menu` precedent). Pure + FFI-free.
#[must_use]
pub fn rerank_output_cap(n: usize) -> u32 {
    // RC4c-2c: raised 256 → 512. A 12B instruct model (Gemma-4 on llama.cpp, which
    // runs the permutation turn GRAMMAR-FREE — the char-level GBNF crashes on digit
    // tokens, T-RERANK-GBNF-CRASH, so only `parse_permutation` enforces the shape)
    // can emit a short reasoning/`<|channel>` preamble before the array; too tight a
    // cap truncated the decode mid-preamble so the close tag never arrived and the
    // strip yielded `""` → fail-closed to input order (the GR24 llama.cpp parity gap).
    // 512 covers a brief preamble + the array while staying bounded (a runaway decode
    // still can't burn the budget). Paired with the array-FIRST prompt + the
    // trailing-tolerant parser so the array is reached well within the cap.
    const REASONING_HEADROOM: usize = 512;
    u32::try_from(n.saturating_mul(6).saturating_add(REASONING_HEADROOM)).unwrap_or(u32::MAX)
}

/// RC4c-2b: build the listwise-rerank prompt — the query + the `n` candidate passages
/// (each truncated to `SNIPPET_MAX` chars), instructing the model to emit ONLY a
/// permutation array of indices. The SHARED renderer for the harness rerank and the
/// live serve rerank turn (byte-identical prompts ⇒ no harness↔serve drift). The
/// runtime still enforces validity fail-closed via `kx_toolcall::parse_permutation`
/// (SN-8: the model proposes an order; the parser is authority). Pure + FFI-free.
#[must_use]
pub fn render_rerank_prompt(query: &str, texts: &[String]) -> String {
    const SNIPPET_MAX: usize = 400;
    let n = texts.len();
    let mut p = String::with_capacity(128 + n * SNIPPET_MAX);
    let _ = write!(
        p,
        "Rank the passages by how well each answers the query. Your entire response must be \
         a single JSON array of the passage indices, most relevant first — a permutation of \
         0..{n} with no duplicates (example: [2,0,1]). Output the array FIRST and nothing \
         else: no reasoning, no explanation, no text before or after it.\n\n\
         Query: {query}\n\nPassages:\n"
    );
    for (i, t) in texts.iter().enumerate() {
        let snippet: String = t.chars().take(SNIPPET_MAX).collect();
        let _ = writeln!(p, "[{i}] {snippet}");
    }
    p
}

/// Assemble the Mote's explicit dependency closure into byte-deterministic
/// resolved content.
///
/// # Algorithm (deterministic, pure)
///
/// 1. For each parent in `mote.parents` where `edge.kind == Data`:
///    - Look up `result_ref` via `snapshot.result_ref_of(parent_id)`.
///    - Fetch bytes via `store.get(result_ref)`.
///    - Emit one `AssembledItem` with label `"parent.<hex prefix>"`.
/// 2. For each `tool_grant` in `warrant.tool_grants`:
///    - Resolve via `registry.resolve(grant, warrant)`.
///    - Hash the resolved `ToolDef` via canonical bincode → `source_ref`.
///    - Emit one `AssembledItem` carrying the tool's menu text, labeled
///      `"tool.<name>@<version>"`.
///    - **PR-6a: the menu text is the tool's `description` PLUS its typed
///      `inputSchema` parameters** (name · type · required), so the model
///      proposes well-formed calls; a schema-less tool is byte-identical to the
///      pre-PR-6a description-only menu.
/// 3. Sort items deterministically: parents first by `MoteId` bytes; tools
///    second by `(tool_id, tool_version)`.
/// 4. Compute total bytes; if `> window_bytes` → return `OverflowDecisionRequired`.
/// 5. Return `Ok(AssembledContext { items })`.
///
/// # Window
///
/// Pass `window_bytes = usize::MAX` to disable the overflow check. Pass a
/// real model-context-window byte budget (typically `4 * max_input_tokens` as
/// a rough heuristic; backends vary) to fail fast on overflow.
///
/// # Errors
///
/// See [`AssemblyError`] variants.
///
/// # Example
///
/// ```no_run
/// use kx_context_assembler::assemble;
/// // (Full example requires constructing Mote + Snapshot + ContentStore +
/// // ToolRegistry + WarrantSpec — see the integration tests for a runnable
/// // setup. This doctest exists to verify the import path compiles.)
/// fn _smoke() { let _ = assemble::<kx_content::InMemoryContentStore>; }
/// ```
#[tracing::instrument(level = "debug", skip_all, fields(mote_id = ?mote.id))]
pub fn assemble<S: ContentStore>(
    mote: &Mote,
    warrant: &WarrantSpec,
    snapshot: &Snapshot,
    store: &S,
    registry: &dyn ToolRegistry,
    window_bytes: usize,
) -> Result<AssembledContext, AssemblyError> {
    let mut items: Vec<AssembledItem> = Vec::new();

    // 1. Parents on Data edges, sorted by MoteId bytes.
    let mut data_parents: Vec<MoteId> = mote
        .parents
        .iter()
        .filter(|p| p.edge.kind == EdgeKind::Data)
        .map(|p| p.parent_id)
        .collect();
    data_parents.sort_by_key(|m| m.0);

    for parent_id in data_parents {
        let result_ref =
            snapshot
                .result_ref_of(&parent_id)
                .ok_or(AssemblyError::UpstreamNotCommitted {
                    parent_mote_id: parent_id,
                })?;
        let payload = store
            .get(&result_ref)
            .map_err(|_| AssemblyError::ContentStoreMiss {
                content_ref: result_ref,
            })?;
        let bytes = Bytes::copy_from_slice(&payload);
        let label = format!("parent.{}", &result_ref.to_hex()[..16]);
        items.push(AssembledItem {
            label,
            bytes,
            source_ref: result_ref,
        });
    }

    // 1b. PR-7: attached context-bundle items, carried in the ENTRY Mote's
    //     `config_subset[CONTEXT_ITEMS_KEY]` (canonical-encoded by the bind layer).
    //     Absent ⇒ byte-identical to pre-PR-7 (no key, this block is skipped). Each
    //     item's blob lives in the SAME content store (a `PutContent` ref); a
    //     missing ref FAILS CLOSED — a run that asked for grounding must never run
    //     silently without it.
    if let Some(encoded) = mote
        .def
        .config_subset
        .get(&ConfigKey(CONTEXT_ITEMS_KEY.to_string()))
    {
        for item in decode_context_items(&encoded.0) {
            let content_ref = ContentRef(item.content_ref);
            let payload = store
                .get(&content_ref)
                .map_err(|_| AssemblyError::ContentStoreMiss { content_ref })?;
            let bytes = Bytes::copy_from_slice(&payload);
            let label = if item.name.is_empty() {
                format!("context.{}", &content_ref.to_hex()[..16])
            } else {
                format!("context.{}", item.name)
            };
            items.push(AssembledItem {
                label,
                bytes,
                source_ref: content_ref,
            });
        }
    }

    // 2. Tools from warrant.tool_grants, sorted by (tool_id, tool_version).
    // BTreeSet iteration is already in (tool_id, tool_version) lex order.
    for grant in &warrant.tool_grants {
        let resolved = registry.resolve(grant, warrant).map_err(|reason| {
            AssemblyError::ToolNotResolvable {
                grant: grant.clone(),
                reason,
            }
        })?;
        // The tool's description PLUS — PR-6a, the "richer formatting at P1.8"
        // hook — its typed input parameters, so the model proposes well-formed
        // tool calls (the runtime still validates args fail-closed against the
        // same `inputSchema`, SN-8). A tool with NO schema is byte-unchanged (the
        // description alone), so legacy menus are identical.
        let desc_bytes =
            Bytes::copy_from_slice(tool_menu_text(&grant.tool_id.0, &resolved.def).as_bytes());
        let label = format!("tool.{}@{}", grant.tool_id.0, grant.tool_version.0);
        items.push(AssembledItem {
            label,
            bytes: desc_bytes,
            source_ref: resolved.event.resolved_def_hash,
        });
    }

    // 3. Sort already enforced above (parents by mote_id, tools by BTreeSet
    //    order). Final pass: verify monotonic invariant for tests.

    // 4. Overflow check — against the TEXT window only.
    //
    //    Image-typed items (recognized by a cheap magic-byte sniff) do NOT
    //    consume the text token budget: the multi-modal backend flows them to
    //    the projector as `content_ref`s, and their token cost is computed by
    //    mtmd (bounded separately by the projector + the warrant). Counting a
    //    multi-MB JPEG against `window_bytes` would spuriously trip overflow.
    //    For a text-only closure (no image-sniffed parents) this sum is
    //    byte-identical to the prior `total_bytes()` — the digest stays
    //    invariant. The pre-decode size cap on image bytes is enforced by the
    //    inference backend against `warrant.resource_ceiling.mem_bytes`.
    let total: usize = items
        .iter()
        .filter(|i| kx_content::sniff_image_format(&i.bytes).is_none())
        .map(|i| i.bytes.len())
        .sum();
    if total > window_bytes {
        return Err(AssemblyError::OverflowDecisionRequired {
            closure_size_bytes: total,
            window_bytes,
        });
    }

    Ok(AssembledContext { items })
}

#[cfg(test)]
mod tool_menu_tests {
    use super::{example_call_json, tool_menu_text};
    use kx_content::ContentRef;
    use kx_mote::{ToolName, ToolVersion};
    use kx_tool_registry::{
        IdempotencyClass, InputSchema, ParamSpec, ParamType, ToolDef, ToolKind,
    };
    use kx_warrant::{FsScope, NetScope, ResourceCeiling, ToolRequirement};

    fn def(input_schema: Option<InputSchema>) -> ToolDef {
        ToolDef {
            tool_id: ToolName("fs-list".into()),
            tool_version: ToolVersion("1".into()),
            kind: ToolKind::Builtin,
            required_capability: ToolRequirement {
                net_scope_required: NetScope::None,
                fs_scope_required: FsScope::empty(),
                syscall_profile_ref: ContentRef::from_bytes([0; 32]),
                min_resource_ceiling: ResourceCeiling {
                    cpu_milli: 0,
                    mem_bytes: 0,
                    wall_clock_ms: 0,
                    fd_count: 0,
                    disk_bytes: 0,
                },
            },
            description: "List a directory.".into(),
            idempotency_class: IdempotencyClass::Readback,
            input_schema,
        }
    }

    #[test]
    fn no_schema_is_name_then_description() {
        // PR-1/BUG-32 name-steering: the menu leads with the EXACT granted name so a
        // model emits it verbatim, then the description.
        assert_eq!(
            tool_menu_text("fs-list", &def(None)),
            "name: fs-list\nversion: 1\nList a directory."
        );
    }

    #[test]
    fn namespaced_grant_id_steers_the_model() {
        // A dialed/local tool is granted NAMESPACED; the `name:` line carries the
        // full callable id (the grant id, not the def's bare name).
        assert_eq!(
            tool_menu_text("kxlocal-a1b2c3d4/multiply", &def(None)),
            "name: kxlocal-a1b2c3d4/multiply\nversion: 1\nList a directory."
        );
    }

    #[test]
    fn schema_appends_typed_params_and_an_example() {
        let schema = InputSchema {
            params: vec![ParamSpec {
                name: "path".into(),
                ty: ParamType::Str { max_len: 4096 },
                required: false,
            }],
            deny_unknown: true,
        };
        // `path` is OPTIONAL, so the minimal valid call has zero required keys: {}.
        assert_eq!(
            tool_menu_text("fs-list", &def(Some(schema))),
            "name: fs-list\nversion: 1\nList a directory.\nInputs:\n  - path (string, optional)\nExample: {}"
        );
    }

    #[test]
    fn example_shows_a_required_string_param() {
        // The echo-shape: ONE required string param → the model sees the exact
        // well-formed call (the §2.246 A3a fix).
        let schema = InputSchema {
            params: vec![ParamSpec {
                name: "text".into(),
                ty: ParamType::Str { max_len: 4096 },
                required: true,
            }],
            deny_unknown: true,
        };
        assert_eq!(
            tool_menu_text("mcp-echo/echo", &def(Some(schema))),
            "name: mcp-echo/echo\nversion: 1\nList a directory.\nInputs:\n  - text (string, required)\n\
             Example: {\"text\": \"<string>\"}"
        );
    }

    #[test]
    fn example_omits_optional_params() {
        let schema = InputSchema {
            params: vec![
                ParamSpec {
                    name: "query".into(),
                    ty: ParamType::Str { max_len: 256 },
                    required: true,
                },
                ParamSpec {
                    name: "limit".into(),
                    ty: ParamType::Int {
                        min: Some(1),
                        max: Some(100),
                    },
                    required: false,
                },
            ],
            deny_unknown: true,
        };
        // Only the REQUIRED `query` appears in the example; `limit` is omitted.
        assert_eq!(example_call_json(&schema), "{\"query\": \"<string>\"}");
    }

    #[test]
    fn example_renders_each_type_in_declared_order() {
        let mut allowed = std::collections::BTreeSet::new();
        allowed.insert("zebra".to_string());
        allowed.insert("alpha".to_string()); // BTreeSet-least → picked
        let schema = InputSchema {
            params: vec![
                ParamSpec {
                    name: "n".into(),
                    ty: ParamType::Int {
                        min: Some(5),
                        max: None,
                    },
                    required: true,
                },
                ParamSpec {
                    name: "flag".into(),
                    ty: ParamType::Bool,
                    required: true,
                },
                ParamSpec {
                    name: "mode".into(),
                    ty: ParamType::Enum { allowed },
                    required: true,
                },
            ],
            deny_unknown: true,
        };
        // Declared order preserved (NOT alphabetical); Int uses its min, Bool is a
        // bare `false`, Enum picks the lexicographically-least allowed value.
        assert_eq!(
            example_call_json(&schema),
            "{\"n\": 5, \"flag\": false, \"mode\": \"alpha\"}"
        );
    }

    #[test]
    fn long_description_is_capped_but_inputs_and_example_stay_byte_intact() {
        // A pathological >cap description (the SOLE uncapped menu field) is truncated
        // and ellipsized, while `Inputs:`/`Example:` stay byte-for-byte intact.
        let long = "x".repeat(500);
        let schema = InputSchema {
            params: vec![ParamSpec {
                name: "text".into(),
                ty: ParamType::Str { max_len: 4096 },
                required: true,
            }],
            deny_unknown: true,
        };
        let mut d = def(Some(schema));
        d.description = long.clone();
        let out = tool_menu_text("mcp-echo/echo", &d);

        // Truncated to exactly 400 chars + a single-char ellipsis.
        let capped = format!("{}…", "x".repeat(400));
        assert!(out.contains(&capped), "description capped + ellipsized: {out}");
        assert!(
            !out.contains(&long),
            "the full 500-char description body must not survive"
        );
        // `Inputs:`/`Example:` are BYTE-preserved (the exact tail is unchanged).
        assert!(
            out.ends_with(
                "\nInputs:\n  - text (string, required)\nExample: {\"text\": \"<string>\"}"
            ),
            "Inputs:/Example: must be byte-intact: {out}"
        );
    }

    #[test]
    fn description_at_cap_is_byte_identical() {
        // A description exactly at the cap is NOT truncated (no ellipsis) — the boundary,
        // and every real bundled tool (all well under 400), renders byte-identically.
        let at_cap = "y".repeat(400);
        let mut d = def(None);
        d.description = at_cap.clone();
        assert_eq!(
            tool_menu_text("fs-list", &d),
            format!("name: fs-list\nversion: 1\n{at_cap}"),
            "a description at the cap is unchanged (no ellipsis)"
        );
    }
}
