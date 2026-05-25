// SPDX-License-Identifier: Apache-2.0
//! `kx-context-assembler` — deterministic context assembly (D33, first part).
//!
//! Per-Mote, before inference, the executor calls [`assemble`] to resolve the
//! Mote's **explicit dependency closure** (upstream committed `result_ref`s
//! along Data edges + granted tool defs) into actual content bytes. The model
//! reasons over **RESOLVED CONTENT ONLY**. Hashes stay in orchestration and
//! NEVER enter the context window.
//!
//! # The invariant
//!
//! ```text
//! same input hashes  →  byte-identical AssembledContext
//! ```
//!
//! Pure function: no clock, no global state, no I/O outside the explicit
//! interfaces ([`Snapshot`], [`ContentStore`], [`ToolRegistry`]). Recovery
//! re-assembles bit-for-bit; replay determinism flows from this.
//!
//! # Deterministic order
//!
//! Items are emitted in this order:
//!
//! 1. **Parents** along Data edges, sorted by `(MoteId bytes, edge.kind, edge.non_cascade)`.
//!    Control edges contribute no content (they're synchronization, not data).
//! 2. **Tools** resolved via the registry from `warrant.tool_grants`, sorted
//!    by `(tool_id, tool_version)`.
//!
//! Same workflow → same order → same byte stream.
//!
//! # The model NEVER sees a hash
//!
//! Every [`AssembledItem::bytes`] field carries RESOLVED CONTENT (the bytes the
//! content store returned). `source_ref` is internal bookkeeping for replay
//! reproducibility — exposed for journaling but never fed into the model's
//! prompt.
//!
//! # The edge-as-relevance-oracle rule
//!
//! Only the Mote's declared parents contribute context. No history-wide
//! retrieval, no embedding-similarity lookup, no implicit "find related past
//! Motes" path. **Implicit retrieval is forbidden** because it would be
//! non-deterministic on its inputs. If a workflow needs additional context,
//! the author adds a parent Mote that produces it — making the dependency
//! EXPLICIT in the graph.
//!
//! # Context-overflow seam
//!
//! If the assembled closure exceeds `window_bytes`, [`assemble`] returns
//! [`AssemblyError::OverflowDecisionRequired`] with the measured closure size.
//! The caller chooses a deterministic resolution path:
//!
//! - **(a) Fixed deterministic ranking + truncation** — a stable sort key
//!   selects the top-N items that fit. The remaining items are dropped from
//!   this Mote's context; the workflow author can re-add them via explicit
//!   shaping if needed.
//! - **(b) Summarization as its own committed Mote** — a new Mote takes the
//!   overflowing parents as input, calls the model to produce a summary,
//!   commits the summary as its `result_ref`. The original Mote then takes
//!   the SUMMARY's `result_ref` as input.
//!
//! **Forbidden**: letting the model choose at inference time (non-deterministic).
//! Set `window_bytes = usize::MAX` to disable the overflow check (the assembler
//! returns whatever fits).
//!
//! # Reading further
//!
//! - `docs/design/context-assembly.md` (private corpus) — the locked D33 spec.
//! - `docs/design/decisions.md` D33 — interlocking with D29, D30, D32.
//! - `05-progress-tracker.md` SN-8 — *model proposes, runtime enforces*.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown,
    clippy::return_self_not_must_use,
    clippy::needless_pass_by_value,
    // Test fixtures use short paired names like parent_a / parent_b which
    // clippy flags as "too similar" — they're intentionally paired and
    // reading them in pairs is the point. Allow at crate root for test code.
    clippy::similar_names
)]

use bytes::Bytes;
use kx_content::{ContentRef, ContentStore};
use kx_mote::{EdgeKind, Mote, MoteId};
use kx_projection::Snapshot;
use kx_tool_registry::{ResolutionError, ToolRegistry};
use kx_warrant::{ToolGrant, WarrantSpec};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// A single resolved item in the assembled context. The model sees `bytes`
/// (raw content); `source_ref` and `label` are bookkeeping for replay
/// reproducibility and operator inspection respectively.
///
/// # Example
///
/// ```
/// use kx_context_assembler::AssembledItem;
/// use bytes::Bytes;
/// use kx_content::ContentRef;
///
/// let item = AssembledItem {
///     label: "parent.abc123".into(),
///     bytes: Bytes::from_static(b"resolved content"),
///     source_ref: ContentRef::of(b"resolved content"),
/// };
/// // The model reasons over `item.bytes` (NEVER a hash); `source_ref` and
/// // `label` are orchestration-side bookkeeping.
/// assert_eq!(&item.bytes[..], b"resolved content");
/// assert_eq!(item.source_ref, ContentRef::of(b"resolved content"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssembledItem {
    /// Human-readable label for this item: `"parent.<hex>"` or
    /// `"tool.<name>@<version>"`. NEVER parsed by the model; for operator
    /// inspection only.
    pub label: String,
    /// The resolved bytes. The model reasons over these. **NEVER a hash.**
    pub bytes: Bytes,
    /// The `ContentRef` the bytes came from. Carried for replay reproducibility
    /// (so the executor can journal a `ToolResolutionEvent`-shaped fact if
    /// needed). Not fed into the model.
    pub source_ref: ContentRef,
}

/// The full assembled context, in deterministic order (Data-edge parents first
/// by `MoteId` bytes; then tools by `(tool_id, tool_version)`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AssembledContext {
    /// Items in deterministic emission order.
    pub items: Vec<AssembledItem>,
}

impl AssembledContext {
    /// Total bytes across all items. Used by [`assemble`] for the overflow
    /// check; exposed publicly for the executor's diagnostics.
    ///
    /// # Example
    ///
    /// ```
    /// use kx_context_assembler::{AssembledContext, AssembledItem};
    /// use bytes::Bytes;
    /// use kx_content::ContentRef;
    ///
    /// let ctx = AssembledContext { items: vec![
    ///     AssembledItem {
    ///         label: "a".into(),
    ///         bytes: Bytes::from_static(b"hello"),
    ///         source_ref: ContentRef::from_bytes([0; 32]),
    ///     },
    ///     AssembledItem {
    ///         label: "b".into(),
    ///         bytes: Bytes::from_static(b"world!"),
    ///         source_ref: ContentRef::from_bytes([1; 32]),
    ///     },
    /// ]};
    /// assert_eq!(ctx.total_bytes(), 11);
    /// ```
    #[must_use]
    pub fn total_bytes(&self) -> usize {
        self.items.iter().map(|i| i.bytes.len()).sum()
    }

    /// `true` iff there are no items.
    ///
    /// # Example
    ///
    /// ```
    /// use kx_context_assembler::AssembledContext;
    /// let ctx: AssembledContext = Default::default();
    /// assert!(ctx.is_empty());
    /// ```
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Number of items in the context.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Compute a content-addressed `ContentRef` over the assembled bytes
    /// in emission order. Useful as a cache key for cross-Mote context reuse
    /// (per D33 §2.5).
    ///
    /// `assembled_ref = blake3(concat_in_order(item.bytes))`. Note this hashes
    /// only the resolved bytes (not the labels or source_refs) so two contexts
    /// with the same content but different labels resolve to the same ref.
    ///
    /// # Example
    ///
    /// ```
    /// use kx_context_assembler::{AssembledContext, AssembledItem};
    /// use bytes::Bytes;
    /// use kx_content::ContentRef;
    ///
    /// let ctx = AssembledContext { items: vec![
    ///     AssembledItem {
    ///         label: "x".into(),
    ///         bytes: Bytes::from_static(b"deterministic"),
    ///         source_ref: ContentRef::from_bytes([0; 32]),
    ///     },
    /// ]};
    /// // Same bytes → same content_ref (idempotent).
    /// assert_eq!(ctx.content_ref(), ctx.content_ref());
    /// ```
    #[must_use]
    pub fn content_ref(&self) -> ContentRef {
        let mut hasher = blake3::Hasher::new();
        for item in &self.items {
            hasher.update(&item.bytes);
        }
        ContentRef::from_bytes(*hasher.finalize().as_bytes())
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Reason [`assemble`] refused.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum AssemblyError {
    /// A declared Data-edge parent has no committed `result_ref` in the
    /// snapshot. Indicates the scheduler dispatched this Mote prematurely.
    #[error("declared Data-edge parent has no committed result_ref: {parent_mote_id:?}")]
    UpstreamNotCommitted {
        /// The parent that should be Committed but isn't.
        parent_mote_id: MoteId,
    },

    /// The content store doesn't have bytes for a `ContentRef` that the
    /// projection said exists. Indicates content-store inconsistency
    /// (operational issue).
    #[error("content store has no entry for ref: {content_ref:?}")]
    ContentStoreMiss {
        /// The missing ref.
        content_ref: ContentRef,
    },

    /// A granted tool failed to resolve through the registry. Surfaces both
    /// `NotFound` and `CapabilityExceedsWarrant` and `PendingHumanReview`
    /// from the registry layer.
    #[error("tool resolution failed: {reason}")]
    ToolNotResolvable {
        /// The tool grant that failed to resolve.
        grant: ToolGrant,
        /// The underlying registry error (typed; see `kx_tool_registry`).
        reason: ResolutionError,
    },

    /// The closure of resolvable content exceeds `window_bytes`. Carries the
    /// MEASURED closure size — the caller picks a deterministic ranking or
    /// summarization strategy (per D33 §5).
    #[error("context closure exceeds window: closure={closure_size_bytes} window={window_bytes}")]
    OverflowDecisionRequired {
        /// Measured total bytes that would be assembled.
        closure_size_bytes: usize,
        /// The window cap.
        window_bytes: usize,
    },
}

// ---------------------------------------------------------------------------
// The pure `assemble` function
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use kx_content::InMemoryContentStore;
    use kx_journal::{InMemoryJournal, Journal, JournalEntry, ParentEntry};
    use kx_mote::{
        derive_mote_id, EdgeMeta, EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId,
        MoteDef, MoteDefHash, NdClass, ParentRef, PromptTemplateHash, ToolName, ToolVersion,
    };
    use kx_projection::Projection;
    use kx_tool_registry::{InMemoryToolRegistry, ToolDef, ToolKind, ToolProvenance, ToolRegistry};
    use kx_warrant::{
        ExecutorClass, FsMode, FsScope, Host, ModelRoute, MoteClass, NetScope, ResourceCeiling,
        ToolGrant, ToolRequirement, WarrantSpec,
    };
    use smallvec::SmallVec;
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;

    // -----------------------------------------------------------------
    // Test helpers — build a fully-formed Mote, projection, store, registry
    // -----------------------------------------------------------------

    fn empty_def_hash() -> MoteDefHash {
        MoteDefHash([0; 32])
    }

    fn permissive_warrant() -> WarrantSpec {
        WarrantSpec {
            mote_class: MoteClass::Pure,
            nd_class: MoteClass::Pure,
            fs_scope: FsScope {
                mounts: BTreeMap::from([(PathBuf::from("/input"), FsMode::ReadOnly)]),
            },
            net_scope: NetScope::EgressAllowlist(BTreeSet::from([Host(
                "api.example.com:443".into(),
            )])),
            syscall_profile_ref: ContentRef::from_bytes([0; 32]),
            tool_grants: BTreeSet::new(),
            model_route: ModelRoute {
                model_id: ModelId("m".into()),
                max_input_tokens: 8000,
                max_output_tokens: 2000,
                max_calls: 10,
            },
            resource_ceiling: ResourceCeiling {
                cpu_milli: 2000,
                mem_bytes: 4 << 30,
                wall_clock_ms: 60_000,
                fd_count: 256,
                disk_bytes: 4 << 30,
            },
            environment_ref: None,
            executor_class: ExecutorClass::Bwrap,
        }
    }

    fn permissive_req() -> ToolRequirement {
        ToolRequirement {
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
        }
    }

    /// Build a Committed JournalEntry for the given MoteId. Each test parent
    /// must use a distinct `idempotency_key` (dedupe-by-key is on
    /// `(idempotency_key, kind=Committed)`); we derive the key from the
    /// MoteId's bytes so test parents naturally differ.
    fn build_committed_entry(
        mote_id: MoteId,
        result_ref: ContentRef,
        parents: SmallVec<[ParentEntry; 4]>,
    ) -> JournalEntry {
        JournalEntry::Committed {
            mote_id,
            idempotency_key: mote_id.0,
            seq: 0,
            nondeterminism: NdClass::Pure,
            result_ref,
            parents,
            warrant_ref: ContentRef::from_bytes([0xaa; 32]),
            mote_def_hash: empty_def_hash(),
        }
    }

    /// Make a Mote with given id, parents, and graph_position. The MoteDef
    /// is a minimal placeholder; the assembler only reads `mote.parents`.
    fn make_mote(
        mote_id: MoteId,
        parents: SmallVec<[ParentRef; 4]>,
        position: GraphPosition,
    ) -> Mote {
        Mote {
            id: mote_id,
            def: MoteDef {
                logic_ref: LogicRef([0; 32]),
                model_id: ModelId("m".into()),
                prompt_template_hash: PromptTemplateHash([0; 32]),
                tool_contract: BTreeMap::new(),
                nd_class: NdClass::Pure,
                config_subset: BTreeMap::new(),
                effect_pattern: EffectPattern::IdempotentByConstruction,
                critic_for: None,
                is_topology_shaper: false,
                schema_version: kx_mote::MOTE_DEF_SCHEMA_VERSION,
            },
            input_data_id: InputDataId([0; 32]),
            graph_position: position,
            parents,
        }
    }

    fn mid(seed: u8) -> MoteId {
        MoteId([seed; 32])
    }

    // -----------------------------------------------------------------
    // Happy path: 2 parents + 1 tool
    // -----------------------------------------------------------------

    #[test]
    fn assemble_two_parents_one_tool() {
        // Build content store with 2 parents' bytes.
        let store = InMemoryContentStore::new();
        let parent_a_bytes = b"output of parent A".to_vec();
        let parent_b_bytes = b"output of parent B".to_vec();
        let parent_a_ref = store.put(&parent_a_bytes).unwrap();
        let parent_b_ref = store.put(&parent_b_bytes).unwrap();

        // Build a journal with the two parent Motes committed.
        let parent_a_id = mid(1);
        let parent_b_id = mid(2);
        let journal = InMemoryJournal::new();
        let e_a = build_committed_entry(parent_a_id, parent_a_ref, SmallVec::new());
        let e_b = build_committed_entry(parent_b_id, parent_b_ref, SmallVec::new());
        let _ = journal.append(e_a).unwrap();
        let _ = journal.append(e_b).unwrap();

        // Fold into projection + snapshot.
        let proj = Projection::from_journal(&journal).unwrap();
        let snapshot = proj.snapshot();

        // Build the registry with one tool granted to the warrant.
        let mut registry = InMemoryToolRegistry::new();
        let tool = ToolDef {
            tool_id: ToolName("fs-read".into()),
            tool_version: ToolVersion("1".into()),
            kind: ToolKind::Builtin,
            required_capability: permissive_req(),
            description: "reads files".into(),
            idempotency_class: kx_tool_registry::IdempotencyClass::Readback,
        };
        let _ = registry
            .register(
                tool.clone(),
                ToolProvenance::HumanAuthored {
                    author: "ops".into(),
                },
            )
            .unwrap();

        // Build the child Mote referencing both parents on Data edges.
        let mut warrant = permissive_warrant();
        warrant.tool_grants = BTreeSet::from([ToolGrant {
            tool_id: tool.tool_id.clone(),
            tool_version: tool.tool_version.clone(),
        }]);
        let parents: SmallVec<[ParentRef; 4]> = SmallVec::from_vec(vec![
            ParentRef {
                parent_id: parent_a_id,
                edge: EdgeMeta::data(),
            },
            ParentRef {
                parent_id: parent_b_id,
                edge: EdgeMeta::data(),
            },
        ]);
        let child_id = derive_mote_id(
            &empty_def_hash(),
            &InputDataId([3; 32]),
            &GraphPosition(vec![0, 0]),
        );
        let child = make_mote(child_id, parents, GraphPosition(vec![0, 0]));

        // Assemble.
        let ctx = assemble(&child, &warrant, &snapshot, &store, &registry, usize::MAX).unwrap();

        // 3 items: 2 parents + 1 tool.
        assert_eq!(ctx.items.len(), 3);
        // Parents come first (sorted by MoteId bytes — A < B because 1 < 2).
        assert!(ctx.items[0].label.starts_with("parent."));
        assert!(ctx.items[1].label.starts_with("parent."));
        // Tool comes after.
        assert_eq!(ctx.items[2].label, "tool.fs-read@1");
        // Parent bytes are resolved content (NEVER hashes).
        assert_eq!(&ctx.items[0].bytes[..], parent_a_bytes);
        assert_eq!(&ctx.items[1].bytes[..], parent_b_bytes);
        // Tool item carries the description.
        assert_eq!(&ctx.items[2].bytes[..], b"reads files");
    }

    // -----------------------------------------------------------------
    // Deterministic parent ordering by MoteId bytes
    // -----------------------------------------------------------------

    #[test]
    fn parent_order_is_deterministic_by_mote_id_bytes() {
        let store = InMemoryContentStore::new();
        let r_5 = store.put(b"five").unwrap();
        let r_3 = store.put(b"three").unwrap();
        let r_7 = store.put(b"seven").unwrap();
        let id_5 = mid(5);
        let id_3 = mid(3);
        let id_7 = mid(7);

        let journal = InMemoryJournal::new();
        let e5 = build_committed_entry(id_5, r_5, SmallVec::new());
        let e3 = build_committed_entry(id_3, r_3, SmallVec::new());
        let e7 = build_committed_entry(id_7, r_7, SmallVec::new());
        // Append in arbitrary order — sort happens in the assembler.
        let _ = journal.append(e5).unwrap();
        let _ = journal.append(e3).unwrap();
        let _ = journal.append(e7).unwrap();

        let snapshot = Projection::from_journal(&journal).unwrap().snapshot();
        let registry = InMemoryToolRegistry::new();

        // Declare parents in non-sorted order — assembler must still sort.
        let parents: SmallVec<[ParentRef; 4]> = SmallVec::from_vec(vec![
            ParentRef {
                parent_id: id_7,
                edge: EdgeMeta::data(),
            },
            ParentRef {
                parent_id: id_3,
                edge: EdgeMeta::data(),
            },
            ParentRef {
                parent_id: id_5,
                edge: EdgeMeta::data(),
            },
        ]);
        let child = make_mote(mid(9), parents, GraphPosition(vec![0]));

        let ctx = assemble(
            &child,
            &permissive_warrant(),
            &snapshot,
            &store,
            &registry,
            usize::MAX,
        )
        .unwrap();

        assert_eq!(ctx.items.len(), 3);
        // Sorted: 3 < 5 < 7 in MoteId byte order.
        assert_eq!(&ctx.items[0].bytes[..], b"three");
        assert_eq!(&ctx.items[1].bytes[..], b"five");
        assert_eq!(&ctx.items[2].bytes[..], b"seven");
    }

    // -----------------------------------------------------------------
    // Control edges contribute NO content
    // -----------------------------------------------------------------

    #[test]
    fn control_edges_skipped() {
        let store = InMemoryContentStore::new();
        let r_data = store.put(b"data").unwrap();
        let r_ctrl = store.put(b"control output").unwrap();
        let id_d = mid(1);
        let id_c = mid(2);

        let journal = InMemoryJournal::new();
        let ed = build_committed_entry(id_d, r_data, SmallVec::new());
        let ec = build_committed_entry(id_c, r_ctrl, SmallVec::new());
        let _ = journal.append(ed).unwrap();
        let _ = journal.append(ec).unwrap();
        let snapshot = Projection::from_journal(&journal).unwrap().snapshot();

        let registry = InMemoryToolRegistry::new();
        let parents: SmallVec<[ParentRef; 4]> = SmallVec::from_vec(vec![
            ParentRef {
                parent_id: id_d,
                edge: EdgeMeta::data(),
            },
            ParentRef {
                parent_id: id_c,
                edge: EdgeMeta::control(),
            },
        ]);
        let child = make_mote(mid(9), parents, GraphPosition(vec![0]));

        let ctx = assemble(
            &child,
            &permissive_warrant(),
            &snapshot,
            &store,
            &registry,
            usize::MAX,
        )
        .unwrap();

        // Only the Data parent's bytes — Control is skipped.
        assert_eq!(ctx.items.len(), 1);
        assert_eq!(&ctx.items[0].bytes[..], b"data");
    }

    // -----------------------------------------------------------------
    // Error: parent not committed
    // -----------------------------------------------------------------

    #[test]
    fn missing_committed_parent_errors() {
        let store = InMemoryContentStore::new();
        let snapshot = Projection::new().snapshot(); // empty
        let registry = InMemoryToolRegistry::new();
        let parents: SmallVec<[ParentRef; 4]> = SmallVec::from_vec(vec![ParentRef {
            parent_id: mid(99),
            edge: EdgeMeta::data(),
        }]);
        let child = make_mote(mid(0), parents, GraphPosition(vec![0]));

        let err = assemble(
            &child,
            &permissive_warrant(),
            &snapshot,
            &store,
            &registry,
            usize::MAX,
        )
        .unwrap_err();

        match err {
            AssemblyError::UpstreamNotCommitted { parent_mote_id } => {
                assert_eq!(parent_mote_id, mid(99));
            }
            other => panic!("expected UpstreamNotCommitted, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // Error: content store miss
    // -----------------------------------------------------------------

    #[test]
    fn content_store_miss_errors() {
        // Commit a parent in the journal but DON'T put its bytes in the store.
        let store = InMemoryContentStore::new();
        let fake_ref = ContentRef::from_bytes([42; 32]);
        let id_a = mid(1);

        let journal = InMemoryJournal::new();
        let e = build_committed_entry(id_a, fake_ref, SmallVec::new());
        let _ = journal.append(e).unwrap();
        let snapshot = Projection::from_journal(&journal).unwrap().snapshot();

        let registry = InMemoryToolRegistry::new();
        let parents: SmallVec<[ParentRef; 4]> = SmallVec::from_vec(vec![ParentRef {
            parent_id: id_a,
            edge: EdgeMeta::data(),
        }]);
        let child = make_mote(mid(9), parents, GraphPosition(vec![0]));

        let err = assemble(
            &child,
            &permissive_warrant(),
            &snapshot,
            &store,
            &registry,
            usize::MAX,
        )
        .unwrap_err();

        assert!(matches!(err, AssemblyError::ContentStoreMiss { .. }));
    }

    // -----------------------------------------------------------------
    // Error: tool not resolvable
    // -----------------------------------------------------------------

    #[test]
    fn tool_not_resolvable_errors() {
        let store = InMemoryContentStore::new();
        let snapshot = Projection::new().snapshot();
        let registry = InMemoryToolRegistry::new(); // empty — no tools

        let mut warrant = permissive_warrant();
        warrant.tool_grants = BTreeSet::from([ToolGrant {
            tool_id: ToolName("nope".into()),
            tool_version: ToolVersion("1".into()),
        }]);

        let child = make_mote(mid(0), SmallVec::new(), GraphPosition(vec![0]));

        let err = assemble(&child, &warrant, &snapshot, &store, &registry, usize::MAX).unwrap_err();

        match err {
            AssemblyError::ToolNotResolvable { grant, .. } => {
                assert_eq!(grant.tool_id.0, "nope");
            }
            other => panic!("expected ToolNotResolvable, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // Overflow: closure size exceeds window
    // -----------------------------------------------------------------

    #[test]
    fn overflow_decision_required_when_window_too_small() {
        let store = InMemoryContentStore::new();
        let big_payload = vec![b'x'; 4096];
        let r = store.put(&big_payload).unwrap();
        let id = mid(1);

        let journal = InMemoryJournal::new();
        let e = build_committed_entry(id, r, SmallVec::new());
        let _ = journal.append(e).unwrap();
        let snapshot = Projection::from_journal(&journal).unwrap().snapshot();

        let registry = InMemoryToolRegistry::new();
        let parents: SmallVec<[ParentRef; 4]> = SmallVec::from_vec(vec![ParentRef {
            parent_id: id,
            edge: EdgeMeta::data(),
        }]);
        let child = make_mote(mid(9), parents, GraphPosition(vec![0]));

        // window_bytes = 100 — far less than 4096.
        let err = assemble(
            &child,
            &permissive_warrant(),
            &snapshot,
            &store,
            &registry,
            100,
        )
        .unwrap_err();

        match err {
            AssemblyError::OverflowDecisionRequired {
                closure_size_bytes,
                window_bytes,
            } => {
                assert_eq!(closure_size_bytes, 4096);
                assert_eq!(window_bytes, 100);
            }
            other => panic!("expected OverflowDecisionRequired, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // Empty assembly: no parents + no tool grants
    // -----------------------------------------------------------------

    #[test]
    fn empty_assembly_is_empty_context() {
        let store = InMemoryContentStore::new();
        let snapshot = Projection::new().snapshot();
        let registry = InMemoryToolRegistry::new();
        let child = make_mote(mid(0), SmallVec::new(), GraphPosition(vec![0]));
        let ctx = assemble(
            &child,
            &permissive_warrant(),
            &snapshot,
            &store,
            &registry,
            usize::MAX,
        )
        .unwrap();
        assert!(ctx.is_empty());
        assert_eq!(ctx.total_bytes(), 0);
        assert_eq!(ctx.len(), 0);
    }

    // -----------------------------------------------------------------
    // content_ref is byte-deterministic
    // -----------------------------------------------------------------

    #[test]
    fn content_ref_is_deterministic() {
        let ctx = AssembledContext {
            items: vec![
                AssembledItem {
                    label: "a".into(),
                    bytes: Bytes::from_static(b"alpha"),
                    source_ref: ContentRef::from_bytes([1; 32]),
                },
                AssembledItem {
                    label: "b".into(),
                    bytes: Bytes::from_static(b"beta"),
                    source_ref: ContentRef::from_bytes([2; 32]),
                },
            ],
        };
        assert_eq!(ctx.content_ref(), ctx.content_ref());
    }

    #[test]
    fn content_ref_ignores_labels_and_source_refs() {
        let ctx_a = AssembledContext {
            items: vec![AssembledItem {
                label: "label-a".into(),
                bytes: Bytes::from_static(b"same bytes"),
                source_ref: ContentRef::from_bytes([1; 32]),
            }],
        };
        let ctx_b = AssembledContext {
            items: vec![AssembledItem {
                label: "label-b".into(),
                bytes: Bytes::from_static(b"same bytes"),
                source_ref: ContentRef::from_bytes([99; 32]),
            }],
        };
        // Same bytes, different labels/source_refs → same content_ref.
        assert_eq!(ctx_a.content_ref(), ctx_b.content_ref());
    }

    #[test]
    fn content_ref_changes_with_bytes() {
        let ctx_a = AssembledContext {
            items: vec![AssembledItem {
                label: "a".into(),
                bytes: Bytes::from_static(b"alpha"),
                source_ref: ContentRef::from_bytes([0; 32]),
            }],
        };
        let ctx_b = AssembledContext {
            items: vec![AssembledItem {
                label: "a".into(),
                bytes: Bytes::from_static(b"alphb"),
                source_ref: ContentRef::from_bytes([0; 32]),
            }],
        };
        assert_ne!(ctx_a.content_ref(), ctx_b.content_ref());
    }
}
