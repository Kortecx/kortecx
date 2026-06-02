//! [`ToolDef`] — the spec the registry stores. Plus the resolution-side
//! [`ToolResolutionEvent`] (content-addressed at resolution time; the resolved
//! versions are captured into run metadata by the coordinator in M1.2, D79 —
//! the event struct is not itself a journal entry) and [`ResolvedTool`] (the
//! rich return shape from [`crate::ToolRegistry::resolve`]).

use kx_content::ContentRef;
use kx_mote::{canonical_config, ToolName, ToolVersion};
use kx_warrant::ToolRequirement;
use serde::{Deserialize, Serialize};

use crate::idempotency_class::IdempotencyClass;
use crate::tool_kind::ToolKind;

/// A tool's full specification, content-addressed by its
/// [`canonical_bincode`][canonical_config] bytes.
///
/// The registry's primary record. Workflows reference by `(tool_id,
/// tool_version)`; the registry resolves to a `ToolDef`. The `description`
/// field is free-form human prose and is **NEVER parsed for enforcement** —
/// it's there for operator-readable inspection only.
///
/// # Canonical-bytes shift (PR 4.6 / D38 §2)
///
/// PR 4.6 added the required `idempotency_class` field. **This is a
/// canonical-bytes-shifting change**: `RegistrationToken` (the dedup primary
/// key) and `ToolResolutionEvent.resolved_def_hash` (the content-addressed
/// resolution event) for any given `ToolDef` now differ from what the
/// pre-PR-4.6 `ToolDef` would have produced. No production state exists at
/// either pin site at the time of the shift, so the canonical-bytes change
/// is bounded entirely to in-test fixtures + the built-ins computed at
/// `with_builtins()` time.
///
/// # No `Default` impl on this struct
///
/// The `idempotency_class` field is **required**, with no `#[serde(default)]`
/// fallback. Every tool MUST declare its class explicitly. The cost of this
/// (fixture rebase when adding the field) is the price of the safety: a
/// silent default would let a token-less WM tool be mis-classified as
/// something safer.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolDef {
    /// Tool name (the workflow's reference key, paired with `tool_version`).
    pub tool_id: ToolName,
    /// Pinned version of the tool. Different versions of the same `tool_id`
    /// are distinct registry entries.
    pub tool_version: ToolVersion,
    /// What kind of tool this is and where it lives.
    pub kind: ToolKind,
    /// The capability requirements this tool declares. Checked at resolution
    /// time against the Mote's warrant via
    /// [`kx_warrant::check_tool_requirement`].
    pub required_capability: ToolRequirement,
    /// Free-form human description. NEVER parsed for enforcement.
    pub description: String,
    /// Per-tool declared idempotency mechanism (D38 §2). Required field;
    /// no default. See [`IdempotencyClass`] for variant semantics.
    pub idempotency_class: IdempotencyClass,
}

/// The content-addressed fact that "tool X version Y was resolved as kind Z
/// from this registry at the resolution event corresponding to this `ContentRef`."
///
/// A pure resolution artifact — **not itself a journal entry**. The coordinator
/// captures the resolved `(tool_id, tool_version, resolved_kind,
/// resolved_def_hash)` as off-DAG run **metadata** (a `RunVersionsResolved`
/// journal fact, M1.2/D79); these versions are audit/lineage metadata, never an
/// identity input, and the executor never journals this struct (the coordinator
/// is the sole writer, D40). **Identity excludes wall-clock time** — including
/// time would break content-addressing (two runs would produce different refs
/// for the same resolution). Time, if needed for audit, lives in the
/// `RunVersionsResolved` entry's header.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolResolutionEvent {
    /// The tool that was resolved.
    pub tool_id: ToolName,
    /// The pinned version.
    pub tool_version: ToolVersion,
    /// What kind it resolved as (which tier served the resolution).
    pub resolved_kind: ToolKind,
    /// blake3 of the resolved `ToolDef`'s canonical bytes. Pins the exact
    /// `ToolDef` shape that was used for this resolution.
    pub resolved_def_hash: ContentRef,
    /// The resolved tool's declared idempotency mechanism (D38 §2 / D65).
    ///
    /// Carried explicitly (in addition to being implied by `resolved_def_hash`)
    /// because the resolved `ToolDef` is **never content-stored** — so the class
    /// could not otherwise be recovered from the hash. The coordinator persists it
    /// on the `RunVersionsResolved` metadata fact (M2.3b, D105.4 Option A) so crash
    /// recovery can pick the class-correct action (Redispatch / CommitFromReadback /
    /// Compensate / Quarantine) for a staged-uncommitted WORLD-MUTATING Mote.
    pub idempotency_class: IdempotencyClass,
}

impl ToolResolutionEvent {
    /// Compute the content-addressed `ContentRef` for this event.
    ///
    /// `event_ref = blake3(canonical_bincode(self))`. Deterministic and pure;
    /// recovery re-derives the same `ContentRef` bit-for-bit.
    ///
    /// # Example
    ///
    /// ```
    /// use kx_tool_registry::{IdempotencyClass, ToolKind, ToolResolutionEvent};
    /// use kx_mote::{ToolName, ToolVersion};
    /// use kx_content::ContentRef;
    ///
    /// let event = ToolResolutionEvent {
    ///     tool_id: ToolName("fs-read".into()),
    ///     tool_version: ToolVersion("1".into()),
    ///     resolved_kind: ToolKind::Builtin,
    ///     resolved_def_hash: ContentRef::from_bytes([0; 32]),
    ///     idempotency_class: IdempotencyClass::Readback,
    /// };
    /// // Same event → same ref (deterministic).
    /// assert_eq!(event.to_ref(), event.to_ref());
    /// ```
    #[must_use]
    pub fn to_ref(&self) -> ContentRef {
        let bytes = bincode::serde::encode_to_vec(self, canonical_config())
            .expect("canonical bincode encoding of ToolResolutionEvent cannot fail");
        ContentRef::of(&bytes)
    }
}

/// The result of [`crate::ToolRegistry::resolve`]: the resolved tool's definition,
/// the journaling-ready resolution event with its content-addressed ref, and
/// the post-check effective capability (which is the tool's
/// `required_capability` — same per-axis values, but pinned to this resolution).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTool {
    /// The resolved tool's spec.
    pub def: ToolDef,
    /// The resolution event (the executor writes its canonical bytes into the
    /// content store and journals `event_ref`).
    pub event: ToolResolutionEvent,
    /// `event.to_ref()` precomputed — the executor verifies this matches what
    /// the content store assigns.
    pub event_ref: ContentRef,
    /// The capability the tool will operate with after the subset check.
    /// Equal to `def.required_capability` on success.
    pub effective_capability: ToolRequirement,
}
