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
