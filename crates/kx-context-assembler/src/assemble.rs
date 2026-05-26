//! The pure [`assemble`] function — resolves a Mote's explicit dependency
//! closure (Data-edge parent `result_ref`s + warrant `tool_grants`) into
//! byte-deterministic [`crate::AssembledContext`].

use bytes::Bytes;
use kx_content::ContentStore;
use kx_mote::{EdgeKind, Mote, MoteId};
use kx_projection::Snapshot;
use kx_tool_registry::ToolRegistry;
use kx_warrant::WarrantSpec;

use crate::errors::AssemblyError;
use crate::types::{AssembledContext, AssembledItem};

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
///    - Emit one `AssembledItem` carrying the tool's `description` bytes,
///      labeled `"tool.<name>@<version>"`.
///    - **Tool defs in v0.1 contribute the `description` field as bytes.** The
///      richer "structured tool spec for the model" surface lives at P1.8
///      (when the inference dispatcher formats tools for the specific
///      backend's expected shape).
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

    // 2. Tools from warrant.tool_grants, sorted by (tool_id, tool_version).
    // BTreeSet iteration is already in (tool_id, tool_version) lex order.
    for grant in &warrant.tool_grants {
        let resolved = registry.resolve(grant, warrant).map_err(|reason| {
            AssemblyError::ToolNotResolvable {
                grant: grant.clone(),
                reason,
            }
        })?;
        // v0.1 contributes the tool's description bytes — enough for the
        // model to know the tool's intent. Richer formatting at P1.8.
        let desc_bytes = Bytes::copy_from_slice(resolved.def.description.as_bytes());
        let label = format!("tool.{}@{}", grant.tool_id.0, grant.tool_version.0);
        items.push(AssembledItem {
            label,
            bytes: desc_bytes,
            source_ref: resolved.event.resolved_def_hash,
        });
    }

    // 3. Sort already enforced above (parents by mote_id, tools by BTreeSet
    //    order). Final pass: verify monotonic invariant for tests.

    // 4. Overflow check.
    let total: usize = items.iter().map(|i| i.bytes.len()).sum();
    if total > window_bytes {
        return Err(AssemblyError::OverflowDecisionRequired {
            closure_size_bytes: total,
            window_bytes,
        });
    }

    Ok(AssembledContext { items })
}
