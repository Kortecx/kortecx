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

/// The [`ConfigKey`] *name* under which an opt-in LLM-JUDGE critic Mote
/// (T-AGENT2) carries its RUBRIC — the grading instruction the judge model
/// evaluates the producer's output against. Delivered like [`PROMPT_KEY`]
/// (identity-bearing in `config_subset` ⇒ folds into `MoteId`), so two judges
/// with different rubrics are distinct Motes without a content-store read; the
/// `CheckSpec::LlmJudge` spec itself carries only the integer output bound.
///
/// Only the *string value* participates in identity; adding or referencing the
/// constant cannot move any digest (the canonical demo declares no judge).
pub const JUDGE_RUBRIC_KEY: &str = "kx.judge.rubric";

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
/// arm raw-commits like a leaf and additionally dead-letters malformed/
/// UNGRANTED tool-shaped output pre-commit (the decode fence), and the
/// coordinator's settle keys only off coordinator-written `ReactRound` facts —
/// the marker alone fires nothing.
pub const REACT_TURN_KEY: &str = "kx.react.turn";

/// The [`ConfigKey`] *name* under which a ReAct SEED Mote may carry its
/// instruction (PR-2d-2) — the `kx/recipes/react` free-param slot name (the
/// recipe binder writes a bound arg into `config_subset[<slot name>]`, so the
/// slot name IS the config key). The coordinator's seed-swap reads the
/// instruction from [`PROMPT_KEY`] first (the direct-submission contract,
/// PR-2d-1), then this key — both JSON-string-or-raw decoded (the
/// `prompt_from_config` precedent: a recipe-bound `Str` arrives JSON-quoted, a
/// directly-built seed carries raw bytes). Read off the SEED only — the seed is
/// validated then SWAPPED, so the key never reaches an admitted identity.
pub const REACT_INSTRUCTION_KEY: &str = "instruction";

/// The [`ConfigKey`] *name* under which a ReAct SEED Mote may carry its
/// per-run turn budget (PR-2d-2, the `kx/recipes/react` `max_turns` slot; a
/// canonical-JSON unsigned integer). Seed-only (see
/// [`REACT_INSTRUCTION_KEY`]); validated `0 < max_tool_calls < max_turns ≤ 8`
/// at the seed-swap, then recorded DURABLY on the turn-0 `ReactRound` anchor —
/// the admitted budget never depends on a default that could drift across
/// binary versions (red-team BLOCKER #4).
pub const REACT_MAX_TURNS_KEY: &str = "max_turns";

/// The [`ConfigKey`] *name* under which a ReAct SEED Mote may carry its per-run
/// tool-call (observation) budget (PR-2d-2, the `kx/recipes/react`
/// `max_tool_calls` slot). See [`REACT_MAX_TURNS_KEY`].
pub const REACT_MAX_TOOL_CALLS_KEY: &str = "max_tool_calls";

/// The [`ConfigKey`] *name* under which a ReAct SEED Mote may carry its per-run
/// HITL approval posture (D114, the `kx/recipes/react` `require_approval` slot; a
/// canonical-JSON boolean). Seed-only; the gateway resolves it from the recipe
/// param OR the serve default and bakes it here, and the coordinator records it
/// DURABLY on the turn-0 `ReactRound` anchor — so the approval gate survives
/// recovery (the seed config is dropped after recovery, exactly the reason the
/// budget caps are anchored). Absent ⇒ `false` (no gate; byte-identical to today).
pub const REACT_REQUIRE_APPROVAL_KEY: &str = "require_approval";

/// The [`ConfigKey`] *name* under which a ReAct Mote may OPT OUT of RC2
/// grammar-constrained tool-calling (the `kx/recipes/react` `unconstrained`
/// slot; a canonical-JSON boolean). ABSENT ⇒ `false` ⇒ grammar is armed on a
/// tool-eligible turn (the always-on default; byte-identical config to pre-RC2
/// since no prior Mote carries this key, so adding the constant moves no digest).
/// PRESENT + `true` ⇒ the executor skips the grammar derivation and decodes
/// unconstrained (relying on the `kx_toolcall` parser). The grammar itself is
/// derived OFF-MoteDef at dispatch (off-digest, D108.2) — only this OPT-OUT is an
/// identity-bearing config choice (a different posture ⇒ a different `MoteId`,
/// the right semantics).
pub const REACT_UNCONSTRAINED_KEY: &str = "unconstrained";

/// The single canonical [`ConfigKey`] *name* under which a STANDALONE authored
/// `tool()` Mote (PR-6b-2) carries its tool-call argument object — ONE
/// canonical-JSON object (e.g. `{"q":"…"}`; `{}` when the call has no args),
/// serialized by the Chains-DSL lowering across all three SDK surfaces
/// (Py/TS/Rust, byte-identical, golden-pinned).
///
/// Unlike a ReAct OBSERVATION — whose args the coordinator RE-DERIVES from the
/// proposing model TURN's committed output ([`REACT_TURN_KEY`]) — an authored
/// tool node has NO model parent: its args are AUTHORED, so they are carried
/// HERE, in `config_subset`, where they fold into [`crate::MoteDef::hash`] →
/// `MoteId`. The args are therefore IDENTITY-BEARING ⇒ deterministic and
/// recovery-stable (a re-lease re-derives byte-identical args with nothing
/// staged), and they cannot be dropped in transit without changing the identity
/// the coordinator re-derives (structurally fail-closed, like [`REACT_TURN_KEY`]).
/// The coordinator's `is_authored_tool` gate keys on the PRESENCE of this key
/// (with `tool_contract` non-empty, `StageThenCommit`, and NOT a react
/// observation) to route the args-from-`config_subset` lease path; every Mote
/// without it leases exactly as before — so adding the constant moves no existing
/// digest (no prior Mote carries this key).
pub const TOOL_ARGS_KEY: &str = "kx.tool.args";

/// PR-7: the `config_subset` key under which the bind layer injects a run's
/// attached context-bundle items (canonical-encoded by
/// [`crate::encode_context_items`]). Present ONLY on an ENTRY Mote of a run that
/// attached `context_bundles`, so a different attached context yields a different
/// `MoteId` (exactly-once-per-`(input + context)`); every Mote without it is
/// byte-identical to pre-PR-7 (no prior Mote carries this key, so adding the
/// constant moves no existing digest). The context-assembler reads it to fetch +
/// label the items for the model.
pub const CONTEXT_ITEMS_KEY: &str = "kx.context.items";

/// AGENTIC-VISION: the `config_subset` key under which the bind layer injects a run's
/// grounding IMAGE as a content-store ref (a JSON string of 64 hex chars — the uploaded
/// blob's `PutContent` ref). It is the SAME public recipe slot name the vision + react
/// recipes publish, so the SDK/CLI/UI bind it through the form-gate. Read by BOTH the
/// gateway executor (turn-0 + the carried successor turns) and the coordinator anchor
/// (which records its `ContentRef` on the turn-0 `ReactRound` so a recovered chain
/// re-derives the image edge-free). Present ONLY on a Mote that bound an image, so every
/// Mote without it is byte-identical to pre-AGENTIC-VISION (no prior Mote carries this
/// key, so adding the constant moves no existing digest).
pub const IMAGE_REF_KEY: &str = "image_ref";

/// RC4c: the `config_subset` key under which an authored RAG `query` step records
/// its retrieval MODE (`"dense"` vs `"hybrid"`). The `rag_pipeline_hybrid` recipe
/// sets `"hybrid"` so its query Mote's `MoteId` is DISTINCT from the dense
/// `rag_pipeline`'s (a different retrieval is a different fact); the harness reads it
/// to choose `query_corpus` vs `query_corpus_hybrid`. Present ONLY on a hybrid query
/// step, so every existing Mote (incl. the dense `rag_pipeline`) is byte-identical —
/// no prior Mote carries this key, so adding the constant moves no existing digest.
pub const RETRIEVAL_MODE_KEY: &str = "kx.retrieval.mode";

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
