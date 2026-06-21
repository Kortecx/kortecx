//! The pure [`assemble`] function — resolves a Mote's explicit dependency
//! closure (Data-edge parent `result_ref`s + warrant `tool_grants`) into
//! byte-deterministic [`crate::AssembledContext`].

use bytes::Bytes;
use kx_content::{ContentRef, ContentStore};
use kx_mote::{decode_context_items, ConfigKey, EdgeKind, Mote, MoteId, CONTEXT_ITEMS_KEY};
use kx_projection::Snapshot;
use kx_tool_registry::{ParamType, ToolDef, ToolRegistry};
use kx_warrant::WarrantSpec;

use crate::errors::AssemblyError;
use crate::types::{AssembledContext, AssembledItem};

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
    let head = format!("name: {grant_id}\n");
    let Some(schema) = &def.input_schema else {
        return format!("{head}{}", def.description);
    };
    let mut text = format!("{head}{}", def.description);
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
    text
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
    use super::tool_menu_text;
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
            "name: fs-list\nList a directory."
        );
    }

    #[test]
    fn namespaced_grant_id_steers_the_model() {
        // A dialed/local tool is granted NAMESPACED; the `name:` line carries the
        // full callable id (the grant id, not the def's bare name).
        assert_eq!(
            tool_menu_text("kxlocal-a1b2c3d4/multiply", &def(None)),
            "name: kxlocal-a1b2c3d4/multiply\nList a directory."
        );
    }

    #[test]
    fn schema_appends_typed_params() {
        let schema = InputSchema {
            params: vec![ParamSpec {
                name: "path".into(),
                ty: ParamType::Str { max_len: 4096 },
                required: false,
            }],
            deny_unknown: true,
        };
        assert_eq!(
            tool_menu_text("fs-list", &def(Some(schema))),
            "name: fs-list\nList a directory.\nInputs:\n  - path (string, optional)"
        );
    }
}
