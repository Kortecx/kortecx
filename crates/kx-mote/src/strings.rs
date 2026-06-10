//! String / byte newtypes: [`ModelId`], [`ToolName`], [`ToolVersion`],
//! [`ConfigKey`], [`ConfigVal`], [`GraphPosition`].

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Identifier newtypes (strings / bytes)
// ---------------------------------------------------------------------------

/// Pinned identity of an inference model, *inclusive of version and quantization*.
///
/// Workflow authors are responsible for packing version and quantization into
/// this identifier — two models with the same name but different quantizations
/// MUST produce different `ModelId`s, or behavior drifts silently across runs.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ModelId(pub String);

/// The name of a tool a Mote may call. Paired with [`ToolVersion`] in
/// [`crate::MoteDef::tool_contract`].
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ToolName(pub String);

/// The version of a tool a Mote may call. A version bump materially changes
/// what a Mote commits and so changes its identity.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ToolVersion(pub String);

/// A key in the curated `config_subset` allowlist of [`crate::MoteDef`].
///
/// **Discipline (closes I2 from `02-improvement-areas.md`).** Only
/// *behavior-affecting* keys belong here. Log-level, telemetry endpoints,
/// worker thread count, and other operational knobs MUST be excluded —
/// including them would re-fire the identity hash on operational tweaks
/// without any change to what the Mote commits. Maintaining this allowlist
/// is a deliberate, reviewed act; the workflow SDK (P4) will surface the
/// review point.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ConfigKey(pub String);

/// The byte-encoded value of a `config_subset` entry. Opaque to this crate;
/// the workflow author decides the encoding (typically a serialized scalar
/// or small struct). Bincode canonical-serializes the bytes verbatim.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ConfigVal(pub Vec<u8>);

/// The single canonical [`ConfigKey`] *name* under which a Mote's instruction
/// prompt is carried in its `config_subset`.
///
/// `MoteDef` has no prompt field — in the full runtime the prompt is assembled
/// at context-assembly time, but its identity-bearing text is carried here, in
/// `config_subset`, so the prompt folds into [`crate::MoteDef::hash`] (same
/// prompt ⇒ same `MoteId`, different prompt ⇒ different `MoteId`). This constant
/// is promoted to the shared substrate so every layer that writes or reads a
/// prompt (the workflow recipe library, the model harness, the planner)
/// references **one** source rather than hand-mirroring the literal `"prompt"`
/// across crates (closes the IMP-7 hand-mirrored-constant hazard).
///
/// Only the *string value* participates in identity (it is a [`ConfigKey`]'s
/// inner `String`); the constant binding itself is never serialized, so adding
/// or referencing it cannot move any digest.
pub const PROMPT_KEY: &str = "prompt";

/// The single canonical [`ConfigKey`] *name* marking a Mote as a live ReAct
/// TURN (PR-2d-1, react-substrate). The value is the run-salt (the registered
/// `instance_id`) — the same bytes salted into the turn's identity material.
///
/// Inserted ONLY by the run-salted react-turn builders (the harness's
/// `react_turn_salted` and the coordinator's `react_shape::build_react_turn`,
/// pinned byte-equivalent by frozen goldens); the unsalted harness builders
/// never write it, so every existing identity is byte-unchanged. The gateway's
/// `ModelRouterExecutor` routes on key PRESENCE to the react decode arm.
///
/// Because the key lives in `config_subset` it folds into [`crate::MoteDef::hash`]
/// → `MoteId` (D53): it cannot be dropped in transit without changing the
/// identity the coordinator re-derives — structurally fail-closed, unlike a
/// droppable wire flag. A client-crafted marker is STRICTLY STRICTER: the react
/// arm raw-commits like a leaf and additionally fences tool-shaped output (the
/// answer-only fence), and the coordinator's settle keys only off
/// coordinator-written `ReactRound` facts — the marker alone fires nothing.
pub const REACT_TURN_KEY: &str = "kx.react.turn";

/// The stable position of a Mote in its DAG.
///
/// Assigned at DAG-compile time (workflow SDK) or derived from a topology
/// shaper's `TopologyDecision` for shaper-spawned children (per `topology.md`
/// §7 / D23: child positions extend the shaper's by appending the child's
/// u32 index in `TopologyDecision.children`). Opaque bytes to this crate;
/// participates in [`crate::MoteId`] derivation alongside `mote_def_hash` and
/// `input_data_id`.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GraphPosition(pub Vec<u8>);
