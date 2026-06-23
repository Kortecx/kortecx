//! `JournalEntry` types + canonical byte encoding per `journal-entry.md` (P0.11, D19).
//!
//! The spec defines a fixed 74-byte header followed by a per-kind body. We model each
//! kind as a Rust variant and provide hand-rolled `encode` / `decode` functions that
//! match the spec byte-for-byte. (Bincode's `Vec` length prefix is u64 with fixint
//! encoding; the spec wants u16 for `parents`. Hand-rolling avoids the divergence.)
//!
//! ## Layout summary
//!
//! ```text
//! header (74 bytes, common to all kinds):
//!     kind             u8     (Proposed=0, Committed=1, Repudiated=2, Failed=3,
//!                              EffectStaged=4, RunRegistered=5,
//!                              RunVersionsResolved=6)
//!     mote_id          [u8;32]
//!     idempotency_key  [u8;32]
//!     seq              u64 LE
//!     nondeterminism   u8     (NdClass: Pure=0, ReadOnlyNondet=1, WorldMutating=2)
//!
//! body by kind:
//!   Proposed (16 bytes):
//!     placement_hint   u128 LE
//!
//!   Committed (34 + N*34 bytes, N = parent count):
//!     result_ref       [u8;32]
//!     parent_count     u16 LE
//!     parents          [ParentEntry; N]
//!
//!   Repudiated (57 bytes):
//!     target_mote_id        [u8;32]
//!     target_committed_seq  u64 LE
//!     reason_class          u8
//!     repudiator_id         u128 LE
//!
//!   Failed (17 bytes):
//!     reason_class     u8
//!     reporter_id      u128 LE
//!
//!   EffectStaged (0 bytes): header-only (v2, D38 §2b)
//!
//!   RunRegistered (56 bytes) (v3, M1.1, D63/D64):
//!     instance_id        [u8;16]
//!     recipe_fingerprint [u8;32]
//!     ts                 u64 LE   (audit-only; excluded from every hash)
//!
//!   RunVersionsResolved (variable) (v4, M1.2, D79): one entry per resolved
//!   capability (append-many; a zero-grant warrant emits one with has_cap=0):
//!     instance_id        [u8;16]
//!     warrant_ref        [u8;32]
//!     model_id_len       u16 LE
//!     model_id           [u8; model_id_len]  (UTF-8)
//!     has_cap            u8 (0 or 1)
//!     -- if has_cap == 1:
//!     tool_id_len        u16 LE
//!     tool_id            [u8; tool_id_len]   (UTF-8)
//!     tool_version_len   u16 LE
//!     tool_version       [u8; tool_version_len] (UTF-8)
//!     resolved_kind_tag  u8 (Builtin=0 .. SelfGenerated=4)
//!     resolved_def_hash  [u8;32]
//!     idempotency_class  u8 (Token=0, Readback=1, Staged=2, AtLeastOnce=3) (v6, M2.3b, D105.4)
//!
//! ParentEntry (34 bytes):
//!     parent_id        [u8;32]
//!     edge_kind        u8 (Data=0, Control=1)
//!     non_cascade      u8 (0 or 1; MUST be 0 when edge_kind == Data)
//! ```

use kx_content::ContentRef;
use kx_mote::{MoteId, NdClass};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

/// Canonical journal-file schema version. Bumped per `journal-entry.md` §10 when the
/// entry encoding changes. Readers refuse loudly on mismatch.
///
/// **v2 (PR 7) changes** vs v1:
/// - `warrant_ref: [u8; 32]` added to `Proposed` and `Committed` bodies (D36).
/// - New `EffectStaged` entry kind (=4); dedup-by-key index expands to
///   `{1, 2, 4}` (D38 §2b).
/// - `MAX_ENTRY_LEN` raised 4460 → 4500 (40-byte headroom for `warrant_ref`).
///
/// v2 readers refuse v1 files loudly (no in-place evolution; no production v1
/// journals exist per the corpus, acceptable).
///
/// **v3 (M1.1, D63/D64) changes** vs v2:
/// - New `RunRegistered` entry kind (=5) — the append-only, immutable
///   run-registration fact (the first entry of a run). Establishes the run's
///   identity root: a journaled `instance_id` (the cross-boundary
///   idempotency-token root) plus a `recipe_fingerprint` retained for
///   discovery/dedup only (NOT identity).
/// - `RunRegistered` does NOT participate in dedup-by-key (one run registers
///   exactly once by construction — one journal per run), so the dedup index
///   stays `{1, 2, 4}`. Strictly additive; `MAX_ENTRY_LEN` is unchanged.
///
/// v3 readers refuse v2 files loudly (no in-place evolution; no production v2
/// journals are retained across the bump per the corpus, acceptable).
///
/// **v4 (M1.2, D79) changes** vs v3:
/// - New `RunVersionsResolved` entry kind (=6) — an append-only, off-DAG
///   run-metadata fact capturing the resolved `(tool_id, tool_version,
///   resolved_kind, resolved_def_hash)` of a capability plus the run's
///   `warrant_ref` and resolved `model_id`. Audit/lineage **metadata, never
///   identity** (never folded into `MoteId`/`input_data_id`/any digest).
/// - One entry per resolved capability (append-many); a zero-grant warrant
///   emits one entry with no capability. Does NOT participate in dedup-by-key
///   (the dedup index stays `{1, 2, 4}`). Strictly additive; `MAX_ENTRY_LEN`
///   is unchanged (a single-capability body is small).
///
/// v4 readers refuse v3 files loudly (no in-place evolution; no production v3
/// journals are retained across the bump per the corpus, acceptable).
///
/// **v5 (M2.2c, D104) changes** vs v4:
/// - New `DigestSealed` entry kind (=7) — the journaled digest seal that anchors
///   the post-recovery `state_digest()` to the trust root, upgrading the M2.2b
///   checkpoint-sidecar trust model from integrity to **unforgeability** (D103.1
///   residual retired). Body layout: `through_seq(u64 LE) ‖ state_digest(32)` =
///   40 bytes (total entry 114 bytes). The header `mote_id` slot carries the
///   synthetic [`seal_root_id`] (anchored to the seq frontier, NOT a run id — a
///   single-node run has no `instance_id` in scope); the `idempotency_key` slot
///   is the all-zero sentinel.
/// - `DigestSealed` does NOT participate in dedup-by-key (the dedup index stays
///   `{1, 2, 4}`); it is an off-DAG metadata fact, never an identity input, never
///   folded into the run-identity product digest. Strictly additive;
///   `MAX_ENTRY_LEN` is unchanged.
///
/// v5 readers refuse v4 files loudly (no in-place evolution; no production v4
/// journals are retained across the bump per the corpus, acceptable).
///
/// **v6 (M2.3b, D105.4) changes** vs v5:
/// - `ResolvedCapabilityRecord` gains a trailing `idempotency_class` (one u8 tag,
///   [`IdempotencyClassTag`]) inside the `RunVersionsResolved` body's
///   capability-present arm. This makes the per-tool `IdempotencyClass` DURABLE
///   so crash recovery can pick the class-correct action (Redispatch /
///   CommitFromReadback / Compensate / Quarantine) for a staged-uncommitted
///   WORLD-MUTATING Mote, instead of relying on the Token-only idempotency-key
///   dedup. Body layout for a present capability becomes
///   `… ‖ resolved_kind_tag(u8) ‖ resolved_def_hash(32) ‖ idempotency_class_tag(u8)`
///   (+1 byte). Still off-DAG, never an identity input, never folded into any
///   digest; the dedup index stays `{1, 2, 4}`; `MAX_ENTRY_LEN` is unchanged.
/// - Two new terminal [`FailureReason`] variants (`CompensatedAtLeastOnce`,
///   `QuarantinedAtLeastOnce`) record the recovery outcome for an at-most-once
///   effect. No `Failed`-body layout change (`reason_class` is already a u8).
///
/// **v7 (PR-2c-2, re-plan-live) changes** vs v6:
/// - New `ReplanRound` entry kind (=8) — the durable record of a coordinator-driven
///   model re-plan round, so a crash-then-recover re-derives the in-flight (not-yet-
///   committed) replan shaper ONLY from committed journal facts (the durability
///   finding that split PR-2c). Carries the round index, the round's shaper `MoteId`
///   (the header slot), the `ContentRef`s of the immutable run base prompt + this
///   round's corrected prompt + the round's `warrant_ref`, the resolved `model_id`,
///   the failed step set that triggered the round, and an optional
///   `escalation_reason_ref` (flag-a-human). Body is variable-length. The header
///   `mote_id` slot carries the round's shaper id directly (a real Mote id, like
///   `Proposed`/`Committed`); the `idempotency_key` slot is the all-zero sentinel.
/// - `ReplanRound` does NOT participate in dedup-by-key (the dedup index stays
///   `{1, 2, 4}`); it is an off-DAG metadata fact (folded as a `last_seq` advance +
///   a `replan_rounds` record), never an identity input, never folded into the
///   run-identity product digest. Strictly additive; `MAX_ENTRY_LEN` is unchanged.
///
/// **v8 (PR-2d-1, react-substrate) changes** vs v7:
/// - New `ReactRound` entry kind (=9) — the durable record of a coordinator-driven
///   ReAct turn, so a crash-then-recover re-derives the in-flight (not-yet-committed)
///   turn AND the spent turn/tool budget ONLY from committed journal facts (the
///   `Committed` entry stores only `mote_def_hash`, never the turn's prompt — the
///   same durability finding that produced `ReplanRound`). Carries the turn index
///   (0 = the submit anchor), the turn's `MoteId` (the header slot), the run's
///   `instance_id` (the run-salt that keys every settle/recover query in serve's
///   SHARED journal — a deliberate difference from `ReplanRound`'s
///   shaper-id+round keying), the `ContentRef`s of the immutable base prompt +
///   the turn warrant, the resolved `model_id`, the turn's settled branch
///   ([`ReactBranch`]: `Answer` / `Tool` / `DeadLettered` / `Pending` /
///   `Rejected` (v10) — FROZEN at
///   append so recovery never re-decodes a re-sampled tail), and the run's durable
///   `max_turns`/`max_tool_calls` caps. Body is variable-length. The header
///   `mote_id` slot carries the turn's id directly (a real Mote id, like
///   `Proposed`/`Committed`); the `idempotency_key` slot is the all-zero sentinel.
/// - `ReactRound` does NOT participate in dedup-by-key (the dedup index stays
///   `{1, 2, 4}`); it is an off-DAG metadata fact (folded as a `last_seq` advance +
///   a `react_rounds` record), never an identity input, never folded into the
///   run-identity product digest. Strictly additive; `MAX_ENTRY_LEN` is unchanged.
///
/// **v9 (PR-9b-2a, deterministic-agentic substrate) changes** vs v8:
/// - `ReactRound` gains a trailing `step_salt: Option<[u8; 32]>` (encoded as a
///   presence byte + an optional 32-byte salt at the very end of the body). It is
///   the per-step salt that disjoins a deterministic-agentic step's private
///   reason→tool→observe chain from the run-level react chain (and from other
///   agentic steps in the same run); `None` ⇒ a run-level chain (every chain a v8
///   binary ever wrote). Still off-DAG metadata — never an identity input, never
///   folded into the run-identity product digest — so the canonical digest is
///   invariant. `blake3` is one-way, so the salt MUST be stored (recovery cannot
///   re-derive it), which is why this is a body change (not a recomputed field).
///   Strictly additive at the tail; `MAX_ENTRY_LEN` is unchanged (+33 bytes max,
///   far under the cap). The compound `(instance_id, step_salt)` chain key that
///   *reads* this field lands with the execution path (PR-9b-2b); v9 only persists
///   it (every v9 `ReactRound` written by this PR carries `None`).
///
/// The strict [`crate::SqliteJournal::open`] refuses any file whose
/// `schema_version` is not exactly the current version (the loud-refusal contract
/// is unchanged: an OLD binary refuses a newer journal rather than misreading it).
///
/// **Migration (IMP-2, M2.x-E).** As of the schema-migration work, an older
/// still-supported version is no longer a dead end: [`crate::migrate_entry`] /
/// [`crate::ReplayJournal`] up-convert old entries to the current shape for
/// replay/recovery, and [`crate::migrate_to`] rewrites an old journal into a fresh
/// v9 one for resume-and-append. v8 → v9 appends the safe-default `None` presence
/// byte to each `ReactRound` (kind 9) body — the lone v8→v9 delta, exactly the
/// v5→v6 trailing-byte shape — and is then v9-valid; v7 → v9 and v6 → v9 are pure
/// pass-throughs (kinds 0..8 are byte-identical and v7/v6 predate `ReactRound`, so
/// they carry no kind-9 entry to grow); v5 → v9 appends the safe-default
/// `idempotency_class` byte (the lone v5→v6 delta) and is then v9-valid.
/// The product identity digest is invariant across migration; see
/// [`crate::migrate_entry`] for the full contract and the supported version window
/// ([`crate::MIN_SUPPORTED_SCHEMA_VERSION`]..=v10).
///
/// **v9 → v10 (PR-3, A2 graceful tool-call recovery).** Adds the
/// [`ReactBranch::Rejected`] variant (branch tag 4, carrying a length-prefixed
/// `reason`) so a refused/invalid tool proposal becomes a NON-terminal round the
/// model reasons over (bounded by the turn/tool-call budget) instead of
/// dead-lettering the whole chain on the first imperfect proposal. A v9 → v10
/// migration is a pure pass-through (kinds 0..=9 are byte-identical and no v9
/// journal contains a tag-4 `ReactRound` — the variant is brand new), exactly like
/// v7/v6 → current. An OLD binary still refuses a v10 journal loudly. The product
/// identity digest is invariant (the PURE-8-mote demo never writes a `ReactRound`).
///
/// **v11 → v12 (PR-9d, per-turn upstream context-carry).** Adds a trailing
/// `context_items_ref: Option<ContentRef>` to each `ReactRound` (kind 9) body —
/// `present(u8: 0|1) ‖ [if 1: content_ref(32)]`, the same shape as the v8→v9
/// `step_salt` delta and recorded on the turn-0 anchor. A v11 body has no such
/// byte and up-converts on decode to `None`. A v11 → v12 migration appends the
/// safe-default `None` presence byte (`0`) to each kind-9 body; kinds 0..=8 are
/// byte-identical. An OLD binary still refuses a v12 journal loudly. The product
/// identity digest is invariant (the PURE-8-mote demo never writes a `ReactRound`).
///
/// **v12 → v13 (T-MULTI-ELEMENT-TOOLCALLS).** Adds a NEW [`ReactBranch`] tag `5`
/// (`ToolBatch`) carrying `count(u16 LE) ‖ count × (u16-prefixed tool_id ‖
/// u16-prefixed tool_version)` in the between-tag-and-caps slot the `Tool`/
/// `Rejected` fields occupy. A brand-new tag means **no v12 body can contain it**
/// (the exact v9→v10 `Rejected`=4 precedent), so the v12 → v13 migration is a
/// PURE pass-through — every v12 body (tags 0..=4) decodes byte-identically under
/// v13. An OLD binary still refuses a v13 journal loudly. The product identity
/// digest is invariant (the PURE-8-mote demo never writes a `ReactRound`).
pub const JOURNAL_SCHEMA_VERSION: u16 = 13;

/// Fixed entry-header length in bytes (`journal-entry.md` §3).
pub const HEADER_LEN: usize = 74;

/// Absolute per-entry size cap.
///
/// **Arithmetic correction note (v1):** `journal-entry.md` §8 originally quoted
/// `4304` but the stated inputs (128 parents × 34 bytes + 32-byte result_ref +
/// 2-byte u16 length prefix + 74-byte header) summed to `4460`. The
/// 128-parent promise was load-bearing; we matched the arithmetic.
///
/// **v2 (PR 7) raises this to 4500** for 40-byte headroom — `warrant_ref` adds
/// 32 bytes to Proposed and Committed bodies (D36). The corrected v2 inputs:
/// 128 parents × 34 + 32 result_ref + 32 warrant_ref + 2 parents-count + 74
/// header = 4492 bytes; the 8-byte buffer leaves room for a future per-body
/// addition without another cap bump.
pub const MAX_ENTRY_LEN: usize = 4500;

/// Maximum number of parents per Committed entry (per the size cap).
pub const MAX_PARENTS: usize = 128;

// ---------------------------------------------------------------------------
// Kind discriminants
// ---------------------------------------------------------------------------

/// `Proposed` entry-kind byte.
pub const KIND_PROPOSED: u8 = 0;
/// `Committed` entry-kind byte.
pub const KIND_COMMITTED: u8 = 1;
/// `Repudiated` entry-kind byte.
pub const KIND_REPUDIATED: u8 = 2;
/// `Failed` entry-kind byte.
pub const KIND_FAILED: u8 = 3;
/// `EffectStaged` entry-kind byte (NEW in v2; D38 §2b).
///
/// `EffectStaged` is the recovery hint that closes the WORLD-MUTATING
/// double-effect window: an effect was staged (intent durably recorded) but
/// not yet committed. On recovery, the projection's fold combines
/// `EffectStaged` with subsequent entries to decide whether re-dispatch is
/// safe (see the 9-cell cross-product in `journal-txn.md`). Body is
/// **header-only** (no payload bytes); dedup-by-key participates per the
/// expanded `{1, 2, 4}` index.
pub const KIND_EFFECT_STAGED: u8 = 4;
/// `RunRegistered` entry-kind byte (NEW in v3; M1.1, D63/D64).
///
/// `RunRegistered` is the append-only, immutable run-registration fact — the
/// FIRST entry of every run (`seq = 1` for a fresh run). It establishes the
/// run's identity root (`instance_id`, read on replay and never recomputed) and
/// carries the `recipe_fingerprint` for discovery/dedup only. Body layout:
/// `instance_id(16) ‖ recipe_fingerprint(32) ‖ ts(u64 LE)` = 56 bytes. Does NOT
/// participate in dedup-by-key (the dedup index stays `{1, 2, 4}`); the header
/// `idempotency_key` slot is the all-zero sentinel and the `mote_id` slot
/// carries the synthetic [`run_root_id`].
pub const KIND_RUN_REGISTERED: u8 = 5;

/// `RunVersionsResolved` entry-kind byte (NEW in v4; M1.2, D79).
///
/// An append-only, **off-DAG run-metadata** fact: the resolved `(tool_id,
/// tool_version, resolved_kind, resolved_def_hash)` of one capability plus the
/// run's `warrant_ref` and resolved `model_id`. These are audit/lineage
/// **metadata, never identity** (never folded into `MoteId`/`input_data_id`/any
/// content-addressed digest, per D64/D79/D70). One entry per resolved
/// capability (append-many); a zero-grant warrant emits a single entry with no
/// capability. Does NOT participate in dedup-by-key (the dedup index stays
/// `{1, 2, 4}`); the header `idempotency_key` slot is the all-zero sentinel and
/// the `mote_id` slot carries the run's synthetic [`run_root_id`].
pub const KIND_RUN_VERSIONS_RESOLVED: u8 = 6;

/// `DigestSealed` entry-kind byte (NEW in v5; M2.2c, D103.2/D104).
///
/// The journaled digest seal: a `through_seq(u64 LE) ‖ state_digest(32)` fact
/// asserting that a faithful fold of the journal through `through_seq` yields a
/// projection whose `state_digest()` equals `state_digest`. Committed *in* the
/// journal (the trust root), it anchors checkpoint-seeded recovery — a
/// forged-but-self-consistent sidecar (the M2.2b D103.1 residual) cannot seed a
/// wrong base state, because the seeded digest would not match the journaled
/// seal, and forging the seal requires forging the journal itself. This upgrades
/// the checkpoint trust model from integrity to **unforgeability**. Does NOT
/// participate in dedup-by-key (the dedup index stays `{1, 2, 4}`); the header
/// `idempotency_key` slot is the all-zero sentinel and the `mote_id` slot carries
/// the synthetic [`seal_root_id`]. Off-DAG metadata: never an identity input,
/// never folded into the run-identity product digest, never gates.
pub const KIND_DIGEST_SEALED: u8 = 7;

/// `ReplanRound` entry-kind byte (NEW in v7; PR-2c-2, re-plan-live).
///
/// The durable record of a coordinator-driven model re-plan round: enough to
/// re-derive the round's shaper Mote (its id, the round's corrected-prompt
/// `ContentRef`, `warrant_ref`, `model_id`) ONLY from committed journal facts on a
/// cold recovery, so an in-flight (not-yet-committed) replan round survives a
/// coordinator crash. Variable-length body:
/// `round(u32 LE) ‖ base_prompt_ref(32) ‖ corrected_prompt_ref(32) ‖
/// warrant_ref(32) ‖ u16-prefixed model_id ‖ failed_count(u16) ‖ failed_count*32 ‖
/// has_escalation(u8) ‖ [if 1: escalation_reason_ref(32)]`. The header `mote_id`
/// slot carries the round's shaper id directly (a real Mote id, like
/// `Proposed`/`Committed`); the `idempotency_key` slot is the all-zero sentinel.
/// Does NOT participate in dedup-by-key (the dedup index stays `{1, 2, 4}`);
/// off-DAG metadata, never an identity input, never folded into any digest.
pub const KIND_REPLAN_ROUND: u8 = 8;

/// `ReactRound` entry-kind byte (NEW in v8; PR-2d-1, react-substrate).
///
/// The durable record of a coordinator-driven **ReAct turn**: enough to re-derive
/// the turn's Mote (its id, the run's base prompt `ContentRef`, `warrant_ref`,
/// `model_id`, the run-salt `instance_id`) AND the spent turn/tool budget ONLY
/// from committed journal facts on a cold recovery. Variable-length body:
/// `turn(u32 LE) ‖ instance_id(16) ‖ base_prompt_ref(32) ‖ warrant_ref(32) ‖
/// u16-prefixed model_id ‖ branch_tag(u8) ‖ [if Tool: u16-prefixed tool_id ‖
/// u16-prefixed tool_version] ‖ max_turns(u32 LE) ‖ max_tool_calls(u32 LE)`.
/// The header `mote_id` slot carries the turn's id directly (a real Mote id, like
/// `Proposed`/`Committed`); the `idempotency_key` slot is the all-zero sentinel.
/// Does NOT participate in dedup-by-key (the dedup index stays `{1, 2, 4}`);
/// off-DAG metadata, never an identity input, never folded into any digest.
pub const KIND_REACT_ROUND: u8 = 9;

/// Length in bytes of a run's `instance_id` (the registered run nonce).
pub const INSTANCE_ID_LEN: usize = 16;

/// `RunRegistered` body length: `instance_id(16) + recipe_fingerprint(32) + ts(8)`.
const RUN_REGISTERED_BODY_LEN: usize = INSTANCE_ID_LEN + 32 + 8;

/// `DigestSealed` body length: `through_seq(u64 LE, 8) + state_digest(32)` = 40
/// bytes (total entry = `HEADER_LEN` + 40 = 114 bytes).
const DIGEST_SEALED_BODY_LEN: usize = 8 + 32;

/// `ReplanRound` body fixed-prefix length: `round(u32, 4) + base_prompt_ref(32) +
/// corrected_prompt_ref(32) + warrant_ref(32) + model_id_len(u16, 2) +
/// failed_count(u16, 2)` = 104 bytes (before the variable model_id, failed-step
/// ids, and optional escalation ref).
const REPLAN_ROUND_PREFIX_LEN: usize = 4 + 32 + 32 + 32 + 2 + 2;

/// Hard cap on a `ReplanRound`'s recorded failed-step ids — a `DoS` bound
/// independent of the size cap. A round's failed children are bounded by the
/// fan-out budget (`LoopBudget::max_children`, typically ≤ 8); 64 is generous
/// headroom and keeps the entry far under [`MAX_ENTRY_LEN`].
pub const MAX_REPLAN_FAILED_STEPS: usize = 64;

/// `ReactRound` body fixed-prefix length: `turn(u32, 4) + instance_id(16) +
/// base_prompt_ref(32) + warrant_ref(32) + model_id_len(u16, 2)` = 86 bytes
/// (before the variable model_id, the branch tag + its optional tool fields,
/// and the trailing budget caps).
const REACT_ROUND_PREFIX_LEN: usize = 4 + INSTANCE_ID_LEN + 32 + 32 + 2;

/// The settled branch of a ReAct turn, FROZEN into the durable [`JournalEntry::ReactRound`]
/// fact at append time (v8, PR-2d-1) so recovery re-reads the committed decision and
/// never re-decodes a tail a re-sampled model output could perturb.
///
/// Serde derives serve the checkpoint DTO only (the `ResolvedCapabilityRecord`
/// precedent); the journal's canonical on-disk encoding is the hand-rolled
/// [`Self::as_u8`] tag, never serde.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReactBranch {
    /// The turn's committed output was a final answer (no tool envelope) — the
    /// chain is terminal at this turn.
    Answer,
    /// The turn's committed output proposed a warrant-granted tool call —
    /// frozen AFTER the settle also resolved the tool against the registry and
    /// validated the args against its typed `inputSchema` (PR-2d-2), so a
    /// recorded `Tool` decision is always fireable. The coordinator then
    /// materializes the OBSERVATION Mote that fires it; the chain advances
    /// once the observation commits.
    Tool {
        /// The proposed tool's name (already grant-checked at decode).
        tool_id: String,
        /// The proposed tool's pinned version.
        tool_version: String,
    },
    /// The turn dead-lettered (an UNRECOVERABLE dispatch/execution failure, or
    /// the chain's turn/tool-call budget was exhausted) — terminal.
    DeadLettered,
    /// The turn is materialized but not yet settled (the anchor state of every
    /// turn). Recovery treats a trailing `Pending` as the work frontier.
    Pending,
    /// v10 (PR-3, A2): the turn proposed a tool call that was REFUSED at the
    /// decode/validate authority site (ungranted/deregistered name, args that
    /// fail the typed `inputSchema`, or a malformed proposal) — but the chain
    /// still has budget, so this is NON-terminal: the `reason` is rendered into
    /// the next turn's context so the model self-corrects (fixes its args, picks
    /// a granted tool, or answers directly). A `Rejected` round counts as one
    /// tool-call AND one turn against the budget, so the loop is bounded; the
    /// chain dead-letters loudly only once the budget is exhausted (BUG-27's
    /// "loud, never silent" terminal is preserved). The `reason` is a pure,
    /// deterministic function of the frozen turn output + the tool schema, so
    /// recovery/replay re-derive identical bytes.
    Rejected {
        /// The fail-closed refusal detail (bounded to [`MAX_REJECTED_REASON_LEN`]
        /// chars at construction); display + next-turn context only.
        reason: String,
    },
    /// v13 (T-MULTI-ELEMENT-TOOLCALLS): the turn's committed output proposed
    /// **N≥2 tool calls in one response** (OpenAI `tool_calls` array or repeated
    /// marked/native segments). Each `(tool_id, tool_version)` was grant-checked
    /// AND its args validated against the typed `inputSchema` at the settle
    /// authority site (all-or-nothing — any one call rejecting freezes the whole
    /// turn `Rejected`), so a recorded `ToolBatch` is always fireable. The
    /// coordinator materializes ONE call-indexed OBSERVATION Mote per call (the
    /// obs MoteId folds `call_index`); the chain advances (re-prompts ONCE with
    /// all N observations) only after EVERY observation commits — the requested
    /// back-pressure. A single-call turn stays [`Self::Tool`] (byte-identical
    /// back-compat); only a genuinely-multi output produces `ToolBatch`. Each
    /// call counts against `max_tool_calls`; the batch fires in full (bounded by
    /// [`MAX_TOOL_BATCH_CALLS`]) then dead-letters loudly if the cumulative
    /// budget is exhausted (BUG-27 "loud, never silent"). The calls are a pure,
    /// deterministic function of the frozen turn output, so recovery/replay
    /// re-derive identical bytes + identical observation ids.
    ToolBatch {
        /// The ordered `(tool_id, tool_version)` calls — the `Vec` index IS the
        /// `call_index` that disambiguates each observation Mote. Length is in
        /// `[2, MAX_TOOL_BATCH_CALLS]` (a 0/1-element decode is never produced;
        /// the parser yields a normal completion or [`Self::Tool`]).
        calls: Vec<(String, String)>,
    },
}

/// Hard cap on a [`ReactBranch::Rejected`] `reason`'s length (char count) — a
/// `DoS`/context-window bound. The coordinator truncates at a char boundary
/// before building the entry; [`MAX_ENTRY_LEN`] still fences the total body.
pub const MAX_REJECTED_REASON_LEN: usize = 512;

/// Hard cap on a [`ReactBranch::ToolBatch`]'s recorded calls — a `DoS` bound
/// independent of the size cap (v13, T-MULTI-ELEMENT-TOOLCALLS). The per-turn
/// batch is bounded by the react tool-call ceiling (`REACT_MAX_TOOL_CALLS`,
/// currently 20); this matches that ceiling so a single turn can fire up to the
/// full chain budget, and keeps the entry far under [`MAX_ENTRY_LEN`].
pub const MAX_TOOL_BATCH_CALLS: usize = 20;

impl ReactBranch {
    /// The branch's closed `u8` tag (the on-disk discriminant).
    #[must_use]
    pub fn as_u8(&self) -> u8 {
        match self {
            Self::Answer => 0,
            Self::Tool { .. } => 1,
            Self::DeadLettered => 2,
            Self::Pending => 3,
            Self::Rejected { .. } => 4,
            Self::ToolBatch { .. } => 5,
        }
    }
}

/// The kind of tool a capability resolved as, mirrored as a closed `u8`-tagged
/// enum so `kx-journal` need not depend on `kx-tool-registry` (the journal must
/// stay dependency-clean). Tags MUST mirror `kx_tool_registry::ToolKind`'s
/// variant order (Builtin=0, LocalScript=1, External=2, Mcp=3,
/// SelfGenerated=4); the coordinator maps `ToolKind → ResolvedKindTag` when
/// building the entry. Adding a variant is a `schema_version` bump.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum ResolvedKindTag {
    /// A built-in OSS tool (`kx_tool_registry::ToolKind::Builtin`).
    Builtin = 0,
    /// A registered local script (`kx_tool_registry::ToolKind::LocalScript`).
    LocalScript = 1,
    /// An external/URL-sourced tool (`kx_tool_registry::ToolKind::External`).
    External = 2,
    /// An MCP-exposed tool (`kx_tool_registry::ToolKind::Mcp`).
    Mcp = 3,
    /// A self-generated tool (`kx_tool_registry::ToolKind::SelfGenerated`).
    SelfGenerated = 4,
}

impl ResolvedKindTag {
    /// The tag's discriminant byte.
    #[must_use]
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// Parse a tag byte; `None` for an unknown discriminant (decoder rejects).
    #[must_use]
    pub fn from_u8(b: u8) -> Option<Self> {
        match b {
            0 => Some(Self::Builtin),
            1 => Some(Self::LocalScript),
            2 => Some(Self::External),
            3 => Some(Self::Mcp),
            4 => Some(Self::SelfGenerated),
            _ => None,
        }
    }
}

/// The resolved tool's idempotency class, mirrored as a closed `u8`-tagged enum
/// so `kx-journal` need not depend on `kx-tool-registry` (the journal must stay
/// dependency-clean, like [`ResolvedKindTag`]). Tags MUST mirror
/// `kx_tool_registry::IdempotencyClass`'s variant order (Token=0, Readback=1,
/// Staged=2, AtLeastOnce=3); the coordinator maps `IdempotencyClass →
/// IdempotencyClassTag` when building the entry. Adding a variant is a
/// `schema_version` bump.
///
/// Made durable by M2.3b (D105.4 Option A): the resolved `IdempotencyClass` is
/// otherwise transient (resolved at submit for the R-10 refusal, then dropped),
/// so crash recovery could only safely re-dispatch Token-class effects. This tag
/// lets recovery pick the class-correct action for every class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum IdempotencyClassTag {
    /// Tool accepts an idempotency token (`kx_tool_registry::IdempotencyClass::Token`).
    Token = 0,
    /// Deterministic read-back probe (`kx_tool_registry::IdempotencyClass::Readback`).
    Readback = 1,
    /// Staged-intent via the `EffectStaged` kind (`kx_tool_registry::IdempotencyClass::Staged`).
    Staged = 2,
    /// No closing mechanism; explicit author ack (`kx_tool_registry::IdempotencyClass::AtLeastOnce`).
    AtLeastOnce = 3,
}

impl IdempotencyClassTag {
    /// The tag's discriminant byte.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// Parse a tag byte; `None` for an unknown discriminant (decoder rejects).
    #[must_use]
    pub const fn from_u8(b: u8) -> Option<Self> {
        Some(match b {
            0 => Self::Token,
            1 => Self::Readback,
            2 => Self::Staged,
            3 => Self::AtLeastOnce,
            _ => return None,
        })
    }
}

/// One resolved capability captured in a [`JournalEntry::RunVersionsResolved`]
/// fact (M1.2, D79; `idempotency_class` added M2.3b/D105.4). Audit/lineage
/// metadata — never an identity input.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResolvedCapabilityRecord {
    /// The resolved tool's name (opaque identifier).
    pub tool_id: String,
    /// The resolved tool's pinned version.
    pub tool_version: String,
    /// Which tier served the resolution.
    pub resolved_kind: ResolvedKindTag,
    /// `blake3(canonical_bincode(ToolDef))` — pins the exact resolved `ToolDef`.
    pub resolved_def_hash: ContentRef,
    /// The resolved tool's durable idempotency class (M2.3b, D105.4). Carried
    /// explicitly because the resolved `ToolDef` is never content-stored, so the
    /// class cannot be recovered from `resolved_def_hash`. Drives the class-aware
    /// recovery decision for a staged-uncommitted WORLD-MUTATING Mote.
    pub idempotency_class: IdempotencyClassTag,
}

// ---------------------------------------------------------------------------
// Closed reason enums (D19; `journal-entry.md` §6.2 + §7.2)
// ---------------------------------------------------------------------------

/// Why a Mote was repudiated. Closed enum per D19 — adding variants is a
/// `schema_version` bump (`journal-entry.md` §10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum RepudiationReason {
    /// Operator explicitly invalidated this Mote.
    OperatorAction = 0,
    /// A critic Mote committed a `CriticVerdict::Invalid` and the operator chose to
    /// repudiate (`validate-then-commit.md` §9 + D22 §5).
    CriticInvalidated = 1,
    /// Batch repudiation by `mote_def_hash` — every Mote sharing the bug class
    /// (`verification.md` Scenario 10 + D22 §6).
    DefinitionLevelRepudiation = 2,
    /// An upstream parent was repudiated and the cascade reached this Mote
    /// (`repudiation.md` §4, D22).
    UpstreamCascade = 3,
    /// The runtime detected a safety-invariant breach (e.g., a dedupe-by-key collision
    /// surfaced post-commit).
    SafetyInvariantBreach = 4,
    /// An external system reported that the effect was wrong after the fact.
    ExternalSystemReportedFailure = 5,
}

impl RepudiationReason {
    /// Convert to the canonical u8 representation used in the entry body.
    #[inline]
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// Decode from the canonical u8 representation. Returns `None` for unknown values
    /// (forward-compat sentinel; readers refuse loudly on unknown discriminants).
    #[must_use]
    pub const fn from_u8(b: u8) -> Option<Self> {
        Some(match b {
            0 => Self::OperatorAction,
            1 => Self::CriticInvalidated,
            2 => Self::DefinitionLevelRepudiation,
            3 => Self::UpstreamCascade,
            4 => Self::SafetyInvariantBreach,
            5 => Self::ExternalSystemReportedFailure,
            _ => return None,
        })
    }
}

/// Why a Mote attempt landed `Failed`. Closed enum per D19 — same rules as
/// [`RepudiationReason`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum FailureReason {
    /// Worker did not commit before the stuck-vs-dead timeout (`stuck-vs-dead.md`, D21).
    TimedOut = 0,
    /// Executor refused dispatch (e.g., unsafe WORLD-MUTATING construction —
    /// `validate-then-commit.md` §7 / `mote.md` §4 anti-pattern).
    ExecutorRefused = 1,
    /// The critic Mote itself failed to perform validation. **Tightened by P0.8 + D20:**
    /// a clean `CriticVerdict::Invalid` is a *successful* critic commit, NOT a `Failed`
    /// entry. This variant is the worker-crashed / payload-malformed / evidence-missing
    /// case.
    ValidatorRejected = 2,
    /// Worker process died and was declared dead by the coordinator before committing.
    WorkerCrashed = 3,
    /// An upstream parent was repudiated; the cascade failed this Mote per the per-nd_class
    /// fail-vs-recompute policy (P0.7 / D22).
    UpstreamRepudiated = 4,
    /// Submission-time refusal: WORLD-MUTATING + no idempotency strategy + no critic
    /// (`mote.md` §4 anti-pattern; refusal predicate in `validate-then-commit.md` §7).
    UnsafeWorldMutatingConstruction = 5,
    /// **Recovery-time outcome (M2.3b, D105.4 / D65).** A staged-uncommitted
    /// at-most-once (`IdempotencyClass::AtLeastOnce`) WORLD-MUTATING effect could
    /// not be safely re-dispatched (that would double-fire), so recovery ran the
    /// capability's deterministic `compensate` (undo). The effect was reversed;
    /// the Mote is terminal. Distinct from `UnsafeWorldMutatingConstruction`
    /// (a *submission-time* refusal) — this is a *recovery-time* clean-up.
    CompensatedAtLeastOnce = 6,
    /// **Recovery-time outcome (M2.3b, D105.4 / D65).** Same situation as
    /// `CompensatedAtLeastOnce`, but the capability does NOT support compensation,
    /// so recovery quarantined the Mote rather than risk a double-fire: it is
    /// terminal (never re-dispatched) and surfaced via the projection's
    /// `AnomalyKind`/`anomaly_motes` for operator review.
    QuarantinedAtLeastOnce = 7,
    /// **Engine dead-letter outcome (F4).** The single-process drive loop gave up
    /// on a Mote: a *transient* infrastructure dispatch error that exhausted the
    /// [`crate`]-external `FailurePolicy` retry budget, OR a *terminal-logic*
    /// dispatch failure that cannot succeed on a retry. Classified **terminal** by
    /// [`is_pre_commit_crash`] (returns `false`), so the projection's `Failed` fold
    /// sets `terminal_failure_observed` → the Mote becomes terminal `Failed`, is
    /// never re-dispatched, and the loop terminates cleanly. **Distinct from
    /// [`TimedOut`](Self::TimedOut)** — `TimedOut` is a *liveness* pre-commit-crash
    /// (the worker died mid-flight; under an `EffectStaged` it *permits* re-dispatch
    /// because the broker's tool-boundary idempotency closes the window). Conflating
    /// the two is the F4 hang: a budget-exhausted dead-letter written as `TimedOut`
    /// under `EffectStaged` stays re-dispatchable forever (`run_with_seams` spins).
    DeadLettered = 8,
}

impl FailureReason {
    /// Convert to the canonical u8 representation.
    #[inline]
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// Decode from the canonical u8 representation.
    #[must_use]
    pub const fn from_u8(b: u8) -> Option<Self> {
        Some(match b {
            0 => Self::TimedOut,
            1 => Self::ExecutorRefused,
            2 => Self::ValidatorRejected,
            3 => Self::WorkerCrashed,
            4 => Self::UpstreamRepudiated,
            5 => Self::UnsafeWorldMutatingConstruction,
            6 => Self::CompensatedAtLeastOnce,
            7 => Self::QuarantinedAtLeastOnce,
            8 => Self::DeadLettered,
            _ => return None,
        })
    }
}

/// Canonical terminality classifier for `FailureReason`.
///
/// Returns `true` iff the failure represents a **pre-commit crash** — a
/// liveness-driven death of the worker between attempt-start and commit.
/// In the 9-cell recovery cross-product (`journal-txn.md` §"Recovery fold
/// semantics"), pre-commit-crash failures paired with an `EffectStaged`
/// entry **permit re-dispatch** (the executor's worker died, the broker's
/// tool-boundary idempotency closes the window).
///
/// Returns `false` for **terminal** failures (deliberate refusals, critic
/// rejections, cascade poisons, anti-pattern refusals). Terminal failures
/// paired with an `EffectStaged` entry **forbid re-dispatch** — the
/// executor declared a definite failure verdict; re-running a WM effect
/// would be the double-effect the seam exists to prevent.
///
/// **Single source of class truth.** Both production code (kx-projection's
/// fold; kx-executor's recovery predicate) AND tests (proptest sweeps via
/// `arbitrary_failure_reason`) call this function. No hardcoded list
/// anywhere. A new `FailureReason` variant must be classified here once,
/// and every consumer picks up the new behavior automatically.
///
/// Per STEP 5.2 + STEP 6.2 of PR 4.5.
///
/// # Examples
///
/// ```
/// use kx_journal::{is_pre_commit_crash, FailureReason};
///
/// // Pre-commit-crash class: liveness-driven, safe to re-dispatch.
/// assert!(is_pre_commit_crash(FailureReason::TimedOut));
/// assert!(is_pre_commit_crash(FailureReason::WorkerCrashed));
///
/// // Terminal class: deliberate, do NOT re-dispatch.
/// assert!(!is_pre_commit_crash(FailureReason::ExecutorRefused));
/// assert!(!is_pre_commit_crash(FailureReason::ValidatorRejected));
/// assert!(!is_pre_commit_crash(FailureReason::UpstreamRepudiated));
/// assert!(!is_pre_commit_crash(FailureReason::UnsafeWorldMutatingConstruction));
/// // The engine dead-letter (F4) is terminal — the loop gave up, never re-dispatch.
/// assert!(!is_pre_commit_crash(FailureReason::DeadLettered));
/// ```
#[must_use]
pub const fn is_pre_commit_crash(reason: FailureReason) -> bool {
    matches!(
        reason,
        FailureReason::TimedOut | FailureReason::WorkerCrashed
    )
}

// ---------------------------------------------------------------------------
// ParentEntry (the on-disk per-parent shape, D19 / journal-entry.md §5)
// ---------------------------------------------------------------------------

/// One parent's on-disk encoding inside a `Committed` body's `parents` array.
///
/// **Why a separate struct (and not `kx_mote::ParentRef` directly).** `ParentRef`
/// nests an `EdgeMeta` whose `EdgeKind` is a Rust enum; bincode would prepend a 4-byte
/// (fixint) variant tag, blowing the 34-byte-per-parent budget. `ParentEntry` uses raw
/// u8 fields so the on-disk byte count matches the spec exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ParentEntry {
    /// The parent Mote's identity.
    pub parent_id: MoteId,
    /// `EdgeKind` discriminant — `Data=0`, `Control=1`.
    pub edge_kind: u8,
    /// `non_cascade` flag — `0` or `1`. MUST be `0` when `edge_kind == 0` (Data);
    /// encoder asserts, decoder rejects (`journal-entry.md` §11 anti-pattern).
    pub non_cascade: u8,
}

impl ParentEntry {
    /// On-disk byte length per parent (`journal-entry.md` §5).
    pub const ENCODED_LEN: usize = 34;

    /// Construct from a `kx_mote::ParentRef` (the workflow-author-side type).
    #[must_use]
    pub fn from_parent_ref(p: &kx_mote::ParentRef) -> Self {
        Self {
            parent_id: p.parent_id,
            edge_kind: p.edge.kind.as_u8(),
            non_cascade: u8::from(p.edge.non_cascade),
        }
    }

    /// Convert back to a `kx_mote::ParentRef`. Returns `None` if the on-disk
    /// `edge_kind` value is unknown (forward-compat sentinel) or if the Data-edge
    /// `non_cascade` invariant is violated (per `journal-entry.md` §11 anti-pattern).
    #[must_use]
    pub fn to_parent_ref(self) -> Option<kx_mote::ParentRef> {
        use kx_mote::{EdgeKind, EdgeMeta};
        let kind = match self.edge_kind {
            0 => EdgeKind::Data,
            1 => EdgeKind::Control,
            _ => return None,
        };
        if kind == EdgeKind::Data && self.non_cascade != 0 {
            return None;
        }
        if self.non_cascade > 1 {
            return None;
        }
        Some(kx_mote::ParentRef {
            parent_id: self.parent_id,
            edge: EdgeMeta {
                kind,
                non_cascade: self.non_cascade == 1,
            },
        })
    }
}

// ---------------------------------------------------------------------------
// JournalEntry — the in-memory union over the four kinds
// ---------------------------------------------------------------------------

/// A journal entry — one atomic record of an attempt's outcome.
///
/// Four kinds, mirroring `journal-txn.md` §3 + `journal-entry.md` §4. The on-disk
/// encoding follows the spec byte-for-byte (see [`encode_entry`]); the Rust struct
/// carries some non-canonical metadata (e.g., `mote_def_hash` on `Committed` — used
/// for `list_committed_by_mote_def_hash` queries per D22 §6 — but NOT serialized in
/// the body, kept in a separate column instead).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalEntry {
    /// The scheduler selected a Mote attempt for placement. Many Proposed entries
    /// may exist for a single identity (re-scheduling after Failed; speculation).
    Proposed {
        /// The Mote's identity.
        mote_id: MoteId,
        /// The Mote's identity key per `idempotency.md`.
        idempotency_key: [u8; 32],
        /// Per-run monotonic sequence number assigned by the journal at commit time.
        seq: u64,
        /// The Mote's non-determinism tag.
        nondeterminism: NdClass,
        /// Opaque placement metadata (worker id, locality hint, etc.). Semantic
        /// interpretation is the coordinator's; the journal pins only the byte budget.
        placement_hint: u128,
        /// The `warrant_ref` (`blake3(canonical_bincode(WarrantSpec))`) of the
        /// warrant this attempt is being dispatched under. NEW in v2 (D36).
        /// Replay re-derives the warrant bit-for-bit from this ref + the content
        /// store; required for the executor's submission-time refusal predicates.
        warrant_ref: ContentRef,
    },

    /// A Mote attempt landed durably. The runtime treats this entry as truth for all
    /// downstream consumers until explicitly Repudiated. **Dedupe-by-key enforced**:
    /// at most one Committed per `idempotency_key`.
    Committed {
        /// The Mote's identity.
        mote_id: MoteId,
        /// The Mote's identity key per `idempotency.md`.
        idempotency_key: [u8; 32],
        /// Per-run monotonic sequence number.
        seq: u64,
        /// The Mote's non-determinism tag.
        nondeterminism: NdClass,
        /// The `ContentRef` of the result payload in the content store.
        result_ref: ContentRef,
        /// Declared parents with edge metadata. SmallVec inline up to 4; heap for 5+.
        parents: SmallVec<[ParentEntry; 4]>,
        /// The `warrant_ref` (`blake3(canonical_bincode(WarrantSpec))`) of the
        /// warrant this commit was performed under. NEW in v2 (D36). The
        /// durable fact carries the warrant identity; replay re-derives bit-for-bit.
        warrant_ref: ContentRef,
        /// **Non-canonical metadata** (NOT in the on-disk body bytes per
        /// `journal-entry.md` §4.2). The Mote's `mote_def_hash` — used by the
        /// `list_committed_by_mote_def_hash` query (`repudiation.md` §6, D22). The
        /// SQLite backend stores this in a separate indexed column.
        mote_def_hash: kx_mote::MoteDefHash,
    },

    /// A committed Mote was explicitly invalidated. The journal is append-only; the
    /// original `Committed` entry remains a historical fact. **Dedupe-by-target**:
    /// at most one Repudiated per `(target_mote_id, target_committed_seq)` pair via
    /// the derived `idempotency_key` (`journal-txn.md` §10, D15).
    Repudiated {
        /// The Mote whose committed entry is being invalidated. Also stored
        /// duplicated in `target_mote_id` inside the body for body-vs-header
        /// consistency checks (`journal-entry.md` §6).
        target_mote_id: MoteId,
        /// Derived key — `blake3("repudiation" ‖ target_mote_id ‖ target_committed_seq)`.
        idempotency_key: [u8; 32],
        /// Per-run monotonic sequence number.
        seq: u64,
        /// The `seq` of the Committed entry being repudiated.
        target_committed_seq: u64,
        /// Why this Mote is being repudiated.
        reason_class: RepudiationReason,
        /// UUID-shaped identifier of the operator or critic responsible.
        repudiator_id: u128,
    },

    /// A Mote attempt reached a terminal failure. NOT deduped: many Failed entries
    /// may exist for one identity (each retry is its own Failed). `Failed → Proposed
    /// → ...` is a valid `seq`-ordered sequence per `mote.md` §7.
    Failed {
        /// The Mote's identity.
        mote_id: MoteId,
        /// The Mote's identity key per `idempotency.md`.
        idempotency_key: [u8; 32],
        /// Per-run monotonic sequence number.
        seq: u64,
        /// Why this attempt failed.
        reason_class: FailureReason,
        /// UUID-shaped identifier of the worker / coordinator reporting the failure.
        reporter_id: u128,
    },

    /// A WORLD-MUTATING effect was staged (intent durably recorded) but not yet
    /// committed. NEW in v2 (D38 §2b). The recovery-hint kind that closes the
    /// WM double-effect window.
    ///
    /// **Body is header-only** — no payload bytes. The MoteId, idempotency_key,
    /// and seq in the header are the full carrying information; downstream
    /// consumers (kx-projection's fold) read presence to set
    /// `effect_staged_observed` on `MoteInfo`.
    ///
    /// **Dedup-by-key participates**: `(idempotency_key, kind = 4)` in the
    /// expanded dedup index `{1, 2, 4}`. Second-write of the same staged-intent
    /// is a no-op success.
    ///
    /// **Recovery-fold semantics**: see the 9-cell cross-product table in
    /// `journal-txn.md`. The interesting cells:
    /// - `EffectStaged` + `Committed` → done (cell 4); never re-dispatch.
    /// - `EffectStaged` + `Failed`(`is_pre_commit_crash`) → re-dispatch permitted
    ///   (cell 3); tool-boundary idempotency closes the window.
    /// - `EffectStaged` + `Failed`(terminal) → **terminal failure** (cell 5); do
    ///   NOT re-dispatch. The executor recorded a definite failure verdict;
    ///   re-running a WM effect here is the double-effect the seam exists to
    ///   prevent.
    /// - `EffectStaged` + `Repudiated` (no `Committed`) → **anomaly** (cell 8);
    ///   quarantine via `MoteState::Inconsistent`.
    EffectStaged {
        /// The Mote's identity.
        mote_id: MoteId,
        /// The Mote's identity key per `idempotency.md`. Participates in the
        /// expanded dedup index `{1, 2, 4}`.
        idempotency_key: [u8; 32],
        /// Per-run monotonic sequence number assigned by the journal at append time.
        seq: u64,
    },

    /// The append-only, immutable run-registration fact (v3, M1.1, D63/D64) —
    /// the FIRST entry of every run (`seq = 1` for a fresh run). Establishes the
    /// run's identity root.
    ///
    /// `instance_id` is the run's registered identity and the cross-boundary
    /// idempotency-token **root** (token derivation is wired in M1.2 — no
    /// `run_scoped_token` exists yet); it is **read on replay, never
    /// recomputed**. `recipe_fingerprint` is the
    /// content/def hash of the run's recipe, retained for **discovery/dedup
    /// only** — never an identity input. `ts` is audit-only and excluded from
    /// every hash.
    ///
    /// Does NOT participate in dedup-by-key: each run registers exactly once by
    /// construction (one journal per run). The header `mote_id` slot carries the
    /// synthetic [`run_root_id`]; the header `idempotency_key` slot is the
    /// all-zero sentinel (kind 5 is excluded from the dedup gate).
    RunRegistered {
        /// The per-run nonce — the registered run identity (and token root).
        instance_id: [u8; INSTANCE_ID_LEN],
        /// The recipe fingerprint (discovery/dedup only; never identity).
        recipe_fingerprint: [u8; 32],
        /// Wall-clock submission time (ms). Audit-only; excluded from every hash.
        ts: u64,
        /// Journal-assigned sequence (0 until appended). `= 1` for a fresh run.
        seq: u64,
    },

    /// An append-only, off-DAG **run-metadata** fact (v4, M1.2, D79): the
    /// resolved versions of a capability invoked under a run, captured at
    /// submit. **Metadata, never identity** — never folded into
    /// `MoteId`/`input_data_id`/any content-addressed digest (D64/D79/D70).
    ///
    /// One entry per resolved capability (append-many); a zero-grant warrant
    /// emits one entry with `capability == None`. Does NOT participate in
    /// dedup-by-key (the dedup index stays `{1, 2, 4}`). The header `mote_id`
    /// slot carries the run's synthetic [`run_root_id`]; the `idempotency_key`
    /// slot is the all-zero sentinel (kind 6 is excluded from the dedup gate).
    RunVersionsResolved {
        /// The run this metadata is attached to (its registered identity).
        instance_id: [u8; INSTANCE_ID_LEN],
        /// `blake3(canonical_bincode(WarrantSpec))` of the warrant resolved under.
        warrant_ref: ContentRef,
        /// The resolved model id (opaque identifier; audit metadata).
        model_id: String,
        /// The resolved capability, or `None` for a zero-grant warrant.
        capability: Option<ResolvedCapabilityRecord>,
        /// Journal-assigned sequence (0 until appended).
        seq: u64,
    },

    /// The journaled digest seal (v5, M2.2c, D103.2/D104) — an off-DAG metadata
    /// fact anchoring the recovered `state_digest()` to the trust root.
    ///
    /// Asserts: a faithful fold of the journal through `through_seq` yields a
    /// projection whose `kx_projection::Projection::state_digest()` equals
    /// `state_digest`. The runtime appends one at each checkpoint frontier `S`
    /// (single-writer ⇒ it lands at `seq = S + 1`), computed *before* the seal is
    /// appended so the sealed digest is the digest at frontier `S`. On recovery a
    /// checkpoint-seeded base at offset `S` is trusted only if the journaled seal
    /// at `through_seq == S` matches the seed's digest; a missing/mismatched seal
    /// discards the checkpoint and full-folds (fail-closed). Forging a sidecar to
    /// seed a wrong base state now requires forging the seal too, which requires
    /// forging the journal — the trust root. This is what upgrades the M2.2b
    /// checkpoint trust model from integrity to **unforgeability** (D103.1).
    ///
    /// **Body** = `through_seq(u64 LE) ‖ state_digest(32)` = 40 bytes. The header
    /// `mote_id` slot carries the synthetic [`seal_root_id`] (anchored to the seq
    /// frontier — a single-node run has no `instance_id`); the `idempotency_key`
    /// slot is the all-zero sentinel; `nondeterminism` is the 0 sentinel (a seal
    /// is not a Mote). Does NOT participate in dedup-by-key (the dedup index stays
    /// `{1, 2, 4}`). Never an identity input; never folded into the run-identity
    /// product digest; the projection folds it as a `last_seq`-only no-op.
    DigestSealed {
        /// The journal frontier this seal anchors — a faithful fold of `(0,
        /// through_seq]` has `state_digest()` equal to `state_digest`.
        through_seq: u64,
        /// `kx_projection::Projection::state_digest()` at frontier `through_seq`.
        state_digest: [u8; 32],
        /// Journal-assigned sequence (0 until appended); `= through_seq + 1` for a
        /// fresh seal under the single-writer discipline.
        seq: u64,
    },

    /// The durable record of a coordinator-driven model **re-plan round** (v7,
    /// PR-2c-2, re-plan-live) — an append-only, off-DAG coordinator-metadata fact.
    ///
    /// When a topology shaper's children settle with ≥1 failure, the live
    /// coordinator drives the next re-plan round: it materializes a round-namespaced
    /// correction shaper and (BEFORE that shaper commits its own `TopologyDecision`)
    /// appends this fact, so a crash between materialization and commit is
    /// recoverable. `recover()` re-derives the entire replan chain ONLY from
    /// committed facts: the round's shaper Mote is rebuilt from
    /// `(round, corrected_prompt_ref, warrant_ref, model_id)` and re-inserted into
    /// the dispatch admission set if it is not yet `Committed`. The `Committed` entry
    /// stores only `mote_def_hash`, not the shaper's `config_subset` (the corrected
    /// prompt), so without this fact a live-materialized round is lost on a crash —
    /// the durability finding that split PR-2c into focused PRs.
    ///
    /// **Metadata, never identity** — never folded into `MoteId`/`input_data_id`/any
    /// content-addressed digest; the projection folds it as a `last_seq`-advance plus
    /// a `replan_rounds` record. Does NOT participate in dedup-by-key (the dedup
    /// index stays `{1, 2, 4}`); the coordinator de-dups by `round`. The header
    /// `mote_id` slot carries `shaper_mote_id` directly; the `idempotency_key` slot
    /// is the all-zero sentinel; `nondeterminism` is the 0 sentinel.
    ReplanRound {
        /// The re-plan round index. `0` is the run's initial-plan anchor (records the
        /// immutable base prompt + the run-fixed warrant for later rounds); `1..` are
        /// the corrective rounds bounded by the coordinator's `MAX_SHAPER_ROUNDS`.
        round: u32,
        /// The round's shaper `MoteId` (also the header `mote_id` slot). Recovery
        /// looks up its `state_of` to decide whether to re-materialize it.
        shaper_mote_id: MoteId,
        /// `ContentRef` of the run's IMMUTABLE base planning prompt — the
        /// after-recovery chaining source for building the NEXT round's corrected
        /// prompt (the base is the same every round; only the failures differ).
        base_prompt_ref: ContentRef,
        /// `ContentRef` of THIS round's corrected planning prompt (base + the
        /// sorted, low-entropy failure tokens). Equals `base_prompt_ref` for round 0.
        corrected_prompt_ref: ContentRef,
        /// `blake3(canonical_bincode(WarrantSpec))` of the round's shaper warrant
        /// (the run-fixed ceiling). Replay re-derives the warrant bit-for-bit.
        warrant_ref: ContentRef,
        /// The resolved model id the round's shaper runs (audit + reconstruction).
        model_id: String,
        /// The step ids that failed and triggered this round, MoteId-byte-sorted and
        /// FROZEN at append (recovery rebuilds the corrected prompt from THIS, never
        /// a fresh `children_of` scan that a late sibling could perturb). Empty for
        /// round 0. Bounded by [`MAX_REPLAN_FAILED_STEPS`].
        failed_steps: SmallVec<[MoteId; 4]>,
        /// `Some(ref)` iff the model escalated (flag-a-human) for this round: the
        /// `ContentRef` of the bounded escalation reason. The run quiesces; recovery
        /// never feeds this into a planner prompt (a distinct field, not a corrected
        /// prompt).
        escalation_reason_ref: Option<ContentRef>,
        /// Journal-assigned sequence (0 until appended).
        seq: u64,
    },

    /// The durable record of a coordinator-driven **ReAct turn** (v8, PR-2d-1,
    /// react-substrate) — an append-only, off-DAG coordinator-metadata fact.
    ///
    /// The live ReAct chain (turn → settle → next turn) is coordinator-materialized;
    /// the `Committed` entry stores only `mote_def_hash`, never the turn's prompt or
    /// the run's budget, so without this fact an in-flight turn AND the spent budget
    /// are lost on a crash (the durability finding that produced `ReplanRound`).
    /// `recover()` re-derives the entire chain ONLY from committed facts: the latest
    /// turn's Mote is rebuilt from `(instance_id, turn, base_prompt_ref, warrant_ref,
    /// model_id)` and re-inserted into dispatch admission if not yet `Committed`;
    /// budget counters are re-derived by folding the recorded branches (never an
    /// in-memory count).
    ///
    /// Two fact shapes share the kind: the **anchor** (`turn = 0`, written at
    /// submit, `branch = Pending`) and per-turn **resolutions/advances** (a settled
    /// branch for turn N, or the next `Pending` turn N+1). The branch is FROZEN at
    /// append so recovery re-reads decisions, never re-decodes a re-sampled tail.
    ///
    /// **Keyed by `instance_id`** (the run-salt): serve's journal is SHARED across
    /// runs, so every settle/recover query scopes by the registered run identity —
    /// a deliberate difference from `ReplanRound`'s shaper-id+round keying.
    ///
    /// **Metadata, never identity** — never folded into `MoteId`/`input_data_id`/any
    /// content-addressed digest; the projection folds it as a `last_seq`-advance plus
    /// a `react_rounds` record. Does NOT participate in dedup-by-key (the dedup index
    /// stays `{1, 2, 4}`); the coordinator de-dups by `(instance_id, turn, branch)`.
    /// The header `mote_id` slot carries `turn_mote_id` directly; the
    /// `idempotency_key` slot is the all-zero sentinel; `nondeterminism` is the 0
    /// sentinel.
    ReactRound {
        /// The turn index. `0` is the run's submit anchor (records the immutable
        /// base prompt + the run-fixed warrant + the durable budget caps); settles
        /// and successor turns reference `1..` bounded by `max_turns`.
        turn: u32,
        /// The turn's `MoteId` (also the header `mote_id` slot) — the RUN-SALTED
        /// id (`blake3("kx-react-turn" ‖ instance_id ‖ turn)`). Recovery looks up
        /// its `state_of` to decide whether to re-materialize it.
        turn_mote_id: MoteId,
        /// The registered run identity (the run-salt). Keys every settle/recover
        /// query in the shared serve journal.
        instance_id: [u8; INSTANCE_ID_LEN],
        /// `ContentRef` of the run's IMMUTABLE base instruction prompt — the
        /// after-recovery source for rebuilding the in-flight turn's Mote (the
        /// trajectory itself is served from committed turn outputs via F-7).
        base_prompt_ref: ContentRef,
        /// `blake3(canonical_bincode(WarrantSpec))` of the run-fixed turn warrant.
        /// Replay re-derives the warrant bit-for-bit (the settle decode gates the
        /// proposal against THIS warrant's `tool_grants`).
        warrant_ref: ContentRef,
        /// The resolved model id the turns run (audit + reconstruction).
        model_id: String,
        /// The turn's settled branch, FROZEN at append. The anchor and a freshly
        /// advanced turn record [`ReactBranch::Pending`]; a settle appends a
        /// resolution fact with the terminal/advancing branch.
        branch: ReactBranch,
        /// The run's durable turn cap (mirrors the harness `ReactBudget`). Recorded
        /// on the anchor so a recovered coordinator enforces the SAME budget the
        /// run was admitted under (never a default that drifted across versions).
        max_turns: u32,
        /// The run's durable tool-call cap (see `max_turns`).
        max_tool_calls: u32,
        /// v9 (PR-9b-2a): the per-step salt that disjoins a deterministic-agentic
        /// step's PRIVATE reason→tool→observe chain from the run-level react chain
        /// (and from other agentic steps in the same run). `None` ⇒ a run-level
        /// chain (every chain v8 ever wrote up-converts to `None`). The execution
        /// path (PR-9b-2b) sets it to the launch step's `MoteId` and keys the
        /// chain by `(instance_id, step_salt)`. Off-DAG metadata — never an
        /// identity input, never folded into the run-identity product digest.
        step_salt: Option<[u8; 32]>,
        /// v11 (PR-R1): does this chain's anchor belong to a launched DETERMINISTIC-
        /// AGENTIC step (needs the launch-mote disposition on settle) or a RUN-LEVEL
        /// react chain (settles on its own terminal Answer)? Since PR-R1 a run-level
        /// chain is ALSO salted (by its seed `MoteId`, so distinct Invokes split), so
        /// `step_salt.is_some()` no longer means "agentic" — this explicit, recovery-
        /// stable flag is the discriminator. `false` for run-level; a v10 body
        /// up-converts to `step_salt.is_some()` (the OLD semantics, preserved). Off-DAG
        /// metadata — never an identity input, never a run-identity-digest input.
        is_agentic_launch: bool,
        /// v12 (PR-9d): `ContentRef` of the run's ENCODED context-items bundle (the
        /// `encode_context_items` bytes, staged in the content store), recorded on
        /// the turn-0 anchor so a recovered coordinator re-derives per-turn grounding
        /// context EDGE-FREE for turns ≥1 — the entry/seed Mote's identity-bearing
        /// `config_subset[CONTEXT_ITEMS_KEY]` is GONE after recovery (the projection
        /// retains only its `mote_def_hash`), exactly the reason `base_prompt_ref` /
        /// `warrant_ref` are anchored here too. `None` ⇒ no attached/retrieved context
        /// (every chain ≤v11 up-converts to `None`). Off-DAG metadata — never an
        /// identity input, never a run-identity-digest input (the canonical 8-Mote
        /// demo writes no `ReactRound`, so the projection identity digest is invariant).
        context_items_ref: Option<ContentRef>,
        /// Journal-assigned sequence (0 until appended).
        seq: u64,
    },
}

/// The all-zero sentinel returned as the dedupe key of a `RunRegistered` entry,
/// which does not participate in dedup-by-key. A `static` so
/// [`JournalEntry::idempotency_key`] can hand back a `&[u8; 32]` uniformly.
static ZERO_IDEMPOTENCY_KEY: [u8; 32] = [0u8; 32];

/// Derive the synthetic 32-byte "run root" id that occupies a `RunRegistered`
/// entry's header `mote_id` slot, from the run's `instance_id`.
///
/// Domain-separated (`"kx-run-root"`) from real Mote ids so it can never collide
/// with one. Derived locally in `kx-journal` (no `kx-executor` dependency — the
/// journal must not depend on the executor).
#[must_use]
pub fn run_root_id(instance_id: &[u8; INSTANCE_ID_LEN]) -> MoteId {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"kx-run-root");
    hasher.update(instance_id);
    MoteId::from_bytes(*hasher.finalize().as_bytes())
}

/// Derive the synthetic 32-byte "seal root" id that occupies a `DigestSealed`
/// entry's header `mote_id` slot, from the sealed `through_seq` frontier.
///
/// Domain-separated (`"kx-digest-seal-root"`) from real Mote ids AND from
/// [`run_root_id`] so it can never collide with either. The seal anchors to the
/// seq frontier (not a run identity) because a single-node run has no journaled
/// `instance_id`; binding the header to `through_seq` gives the same body-header
/// consistency property `RunRegistered` has (the decoder re-derives it and
/// rejects a mismatch). Derived locally in `kx-journal` (no executor dependency).
#[must_use]
pub fn seal_root_id(through_seq: u64) -> MoteId {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"kx-digest-seal-root");
    hasher.update(&through_seq.to_le_bytes());
    MoteId::from_bytes(*hasher.finalize().as_bytes())
}

impl JournalEntry {
    /// The entry's `seq` value.
    #[must_use]
    pub fn seq(&self) -> u64 {
        match self {
            Self::Proposed { seq, .. }
            | Self::Committed { seq, .. }
            | Self::Repudiated { seq, .. }
            | Self::Failed { seq, .. }
            | Self::EffectStaged { seq, .. }
            | Self::RunRegistered { seq, .. }
            | Self::RunVersionsResolved { seq, .. }
            | Self::DigestSealed { seq, .. }
            | Self::ReplanRound { seq, .. }
            | Self::ReactRound { seq, .. } => *seq,
        }
    }

    /// The entry's `idempotency_key`.
    #[must_use]
    pub fn idempotency_key(&self) -> &[u8; 32] {
        match self {
            Self::Proposed {
                idempotency_key, ..
            }
            | Self::Committed {
                idempotency_key, ..
            }
            | Self::Repudiated {
                idempotency_key, ..
            }
            | Self::Failed {
                idempotency_key, ..
            }
            | Self::EffectStaged {
                idempotency_key, ..
            } => idempotency_key,
            // RunRegistered / RunVersionsResolved / DigestSealed / ReplanRound /
            // ReactRound do not dedup; return the all-zero sentinel (kinds 5, 6,
            // 7, 8, 9 are excluded from the dedup gate).
            Self::RunRegistered { .. }
            | Self::RunVersionsResolved { .. }
            | Self::DigestSealed { .. }
            | Self::ReplanRound { .. }
            | Self::ReactRound { .. } => &ZERO_IDEMPOTENCY_KEY,
        }
    }

    /// The entry's primary `mote_id`. For `Repudiated` entries this is the
    /// `target_mote_id` (matches the header's `mote_id` per `journal-entry.md` §6);
    /// for `RunRegistered` (which names no Mote) this is the synthetic
    /// [`run_root_id`] derived from the run's `instance_id`.
    #[must_use]
    pub fn mote_id(&self) -> MoteId {
        match self {
            Self::Proposed { mote_id, .. }
            | Self::Committed { mote_id, .. }
            | Self::Failed { mote_id, .. }
            | Self::EffectStaged { mote_id, .. } => *mote_id,
            Self::Repudiated { target_mote_id, .. } => *target_mote_id,
            Self::RunRegistered { instance_id, .. }
            | Self::RunVersionsResolved { instance_id, .. } => run_root_id(instance_id),
            Self::DigestSealed { through_seq, .. } => seal_root_id(*through_seq),
            // The header `mote_id` slot carries the round's shaper id directly (a
            // real Mote id, like Proposed/Committed) — recovery uses it to look up
            // the shaper's `state_of`.
            Self::ReplanRound { shaper_mote_id, .. } => *shaper_mote_id,
            // Same anchoring for a ReAct turn: the header slot IS the turn's
            // (run-salted) Mote id.
            Self::ReactRound { turn_mote_id, .. } => *turn_mote_id,
        }
    }

    /// The entry's kind discriminant byte.
    #[must_use]
    pub fn kind(&self) -> u8 {
        match self {
            Self::Proposed { .. } => KIND_PROPOSED,
            Self::Committed { .. } => KIND_COMMITTED,
            Self::Repudiated { .. } => KIND_REPUDIATED,
            Self::Failed { .. } => KIND_FAILED,
            Self::EffectStaged { .. } => KIND_EFFECT_STAGED,
            Self::RunRegistered { .. } => KIND_RUN_REGISTERED,
            Self::RunVersionsResolved { .. } => KIND_RUN_VERSIONS_RESOLVED,
            Self::DigestSealed { .. } => KIND_DIGEST_SEALED,
            Self::ReplanRound { .. } => KIND_REPLAN_ROUND,
            Self::ReactRound { .. } => KIND_REACT_ROUND,
        }
    }
}

// ---------------------------------------------------------------------------
// Canonical byte encoding — spec-exact per journal-entry.md
// ---------------------------------------------------------------------------

/// Errors raised when decoding a `JournalEntry` from canonical bytes.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DecodeError {
    /// The buffer is shorter than the fixed 74-byte header.
    #[error("input too short for header: {got} bytes < {} required", HEADER_LEN)]
    HeaderTooShort {
        /// Bytes actually available.
        got: usize,
    },
    /// The body is shorter than the kind's expected layout requires.
    #[error("body too short for kind {kind}: {got} bytes < {expected} required")]
    BodyTooShort {
        /// Discriminant byte from the header.
        kind: u8,
        /// Bytes available for the body.
        got: usize,
        /// Bytes the body's fixed prefix requires.
        expected: usize,
    },
    /// The entry exceeds the absolute size cap.
    #[error("entry exceeds size cap: {got} bytes > {} max", MAX_ENTRY_LEN)]
    TooLarge {
        /// Bytes actually present.
        got: usize,
    },
    /// The kind discriminant byte is not one of the known values
    /// (Proposed=0 .. ReplanRound=8).
    #[error("unknown kind discriminant: {0}")]
    UnknownKind(u8),
    /// The `nondeterminism` discriminant byte is not one of the three known values.
    #[error("unknown nondeterminism discriminant: {0}")]
    UnknownNdClass(u8),
    /// The `reason_class` byte (in a Repudiated or Failed entry) is not one of the
    /// six known values.
    #[error("unknown reason_class discriminant: {0}")]
    UnknownReason(u8),
    /// A `ParentEntry`'s `edge_kind` byte is not 0 or 1.
    #[error("unknown edge_kind discriminant: {0}")]
    UnknownEdgeKind(u8),
    /// A `ParentEntry`'s `non_cascade` byte is 1 on a Data edge — forbidden by
    /// `journal-entry.md` §11 anti-pattern (encoder MUST set 0; decoder rejects).
    #[error("non_cascade flag set on Data edge (anti-pattern §11)")]
    DataEdgeNonCascade,
    /// A `ParentEntry`'s `non_cascade` byte is neither 0 nor 1.
    #[error("non_cascade flag is not boolean: {0}")]
    NonBooleanNonCascade(u8),
    /// The `Repudiated` body's `target_mote_id` does not match the header's `mote_id`
    /// (`journal-entry.md` §6 + test #17).
    #[error("Repudiated body-header mote_id mismatch")]
    RepudiatedHeaderMismatch,
    /// A `RunRegistered` entry's header `mote_id` slot does not equal
    /// `run_root_id(instance_id)` (v3 body-header consistency; mirrors
    /// [`Self::RepudiatedHeaderMismatch`]).
    #[error("RunRegistered body-header run_root_id mismatch")]
    RunRegisteredHeaderMismatch,
    /// A `RunVersionsResolved` entry's header `mote_id` slot does not equal
    /// `run_root_id(instance_id)` (v4 body-header consistency; mirrors
    /// [`Self::RunRegisteredHeaderMismatch`]).
    #[error("RunVersionsResolved body-header run_root_id mismatch")]
    RunVersionsHeaderMismatch,
    /// A `DigestSealed` entry's header `mote_id` slot does not equal
    /// `seal_root_id(through_seq)` (v5 body-header consistency; mirrors
    /// [`Self::RunRegisteredHeaderMismatch`]).
    #[error("DigestSealed body-header seal_root_id mismatch")]
    DigestSealedHeaderMismatch,
    /// A `RunVersionsResolved` entry's `resolved_kind_tag` byte is not one of the
    /// five known [`ResolvedKindTag`] values.
    #[error("unknown resolved_kind tag: {0}")]
    UnknownResolvedKind(u8),
    /// A `RunVersionsResolved` entry's `idempotency_class` tag byte is not one of
    /// the four known [`IdempotencyClassTag`] values (v6, M2.3b). A v5-shaped body
    /// lacking the trailing byte fails the exact-cursor check as `TrailingBytes`
    /// before reaching here; this guards a present-but-corrupt tag.
    #[error("unknown idempotency_class tag: {0}")]
    UnknownIdempotencyClass(u8),
    /// A `RunVersionsResolved` entry's `has_cap` flag is neither 0 nor 1.
    #[error("has_cap flag is not boolean: {0}")]
    NonBooleanHasCap(u8),
    /// A `RunVersionsResolved` entry's UTF-8 string field is not valid UTF-8.
    #[error("RunVersionsResolved string field is not valid UTF-8")]
    RunVersionsInvalidUtf8,
    /// A `ReplanRound` entry declares more failed steps than
    /// [`MAX_REPLAN_FAILED_STEPS`] (v7, PR-2c-2) — a `DoS` bound.
    #[error(
        "ReplanRound failed-step count {got} exceeds max {}",
        MAX_REPLAN_FAILED_STEPS
    )]
    ReplanRoundTooManyFailedSteps {
        /// The declared failed-step count.
        got: usize,
    },
    /// A `ReplanRound` entry's `has_escalation` flag is neither 0 nor 1 (v7).
    #[error("ReplanRound has_escalation flag is not boolean: {0}")]
    NonBooleanHasEscalation(u8),
    /// A `ReactRound` entry's `branch` tag byte is not one of the known
    /// [`ReactBranch`] tags (v8, PR-2d-1; v13 added tag 5 `ToolBatch`).
    #[error("unknown ReactRound branch tag: {0}")]
    UnknownReactBranch(u8),
    /// A `ReactRound` entry's `ToolBatch` declares more calls than
    /// [`MAX_TOOL_BATCH_CALLS`] (v13, T-MULTI-ELEMENT-TOOLCALLS) — a `DoS` bound.
    #[error(
        "ReactRound ToolBatch call count {got} exceeds max {}",
        MAX_TOOL_BATCH_CALLS
    )]
    ReactRoundTooManyBatchCalls {
        /// The declared call count.
        got: usize,
    },
    /// A `ReactRound` entry's trailing `is_agentic_launch` tag byte is neither `0`
    /// (run-level) nor `1` (agentic launch) (v11, PR-R1).
    #[error("unknown ReactRound is_agentic_launch tag: {0}")]
    UnknownReactAgenticLaunchTag(u8),
    /// A `ReactRound` entry's trailing `context_items_ref` presence tag byte is not
    /// `0` (absent) or `1` (present) (v12, PR-9d).
    #[error("unknown ReactRound context_items_ref tag: {0}")]
    UnknownReactContextItemsTag(u8),
    /// A `ReactRound` entry's trailing `step_salt` presence tag byte is not `0`
    /// (absent) or `1` (present) (v9, PR-9b-2a).
    #[error("unknown ReactRound step_salt presence tag: {0}")]
    UnknownReactStepSaltTag(u8),
    /// Trailing bytes after a complete entry (§2 no-trailing-data rule).
    #[error("trailing bytes after entry: {0} extra")]
    TrailingBytes(usize),
}

/// Errors raised when encoding a `JournalEntry`.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EncodeError {
    /// More parents than the journal allows per entry (`journal-entry.md` §5
    /// per-entry max + §8 size cap).
    #[error("parent count {got} exceeds max {}", MAX_PARENTS)]
    TooManyParents {
        /// Parents the caller requested.
        got: usize,
    },
    /// A Data edge's `non_cascade` flag is `true` — the encoder rejects rather than
    /// silently coercing (`journal-entry.md` §11).
    #[error("non_cascade flag set on Data edge (anti-pattern §11)")]
    DataEdgeNonCascade,
    /// A `RunVersionsResolved` string field exceeds the `u16` length prefix
    /// (65535 bytes) — far beyond any real tool/model id.
    #[error("RunVersionsResolved field exceeds u16 length prefix: {got} bytes")]
    RunVersionsFieldTooLong {
        /// The over-long field's byte length.
        got: usize,
    },
    /// A `RunVersionsResolved` entry would exceed the absolute size cap
    /// ([`MAX_ENTRY_LEN`]) — a pathologically large model/tool id.
    #[error(
        "RunVersionsResolved entry exceeds size cap: {got} bytes > {} max",
        MAX_ENTRY_LEN
    )]
    RunVersionsTooLarge {
        /// The encoded entry's byte length.
        got: usize,
    },
    /// A `ReplanRound` entry declares more failed steps than
    /// [`MAX_REPLAN_FAILED_STEPS`] — a `DoS` bound independent of the size cap.
    #[error(
        "ReplanRound failed-step count {got} exceeds max {}",
        MAX_REPLAN_FAILED_STEPS
    )]
    ReplanRoundTooManyFailedSteps {
        /// The over-long failed-step count the caller requested.
        got: usize,
    },
    /// A `ReplanRound` entry would exceed the absolute size cap ([`MAX_ENTRY_LEN`])
    /// — a pathologically large model id.
    #[error(
        "ReplanRound entry exceeds size cap: {got} bytes > {} max",
        MAX_ENTRY_LEN
    )]
    ReplanRoundTooLarge {
        /// The encoded entry's byte length.
        got: usize,
    },
    /// A `ReactRound` entry would exceed the absolute size cap ([`MAX_ENTRY_LEN`])
    /// — a pathologically large model/tool id (v8, PR-2d-1).
    #[error(
        "ReactRound entry exceeds size cap: {got} bytes > {} max",
        MAX_ENTRY_LEN
    )]
    ReactRoundTooLarge {
        /// The encoded entry's byte length.
        got: usize,
    },
}

/// Encode a `JournalEntry` to its canonical on-disk byte representation
/// (`journal-entry.md` §3-7).
///
/// # Examples
///
/// Round-trip a Failed entry through encode + decode:
///
/// ```
/// use kx_journal::{decode_entry, encode_entry, FailureReason, JournalEntry};
/// use kx_mote::MoteId;
///
/// let entry = JournalEntry::Failed {
///     mote_id: MoteId::from_bytes([7u8; 32]),
///     idempotency_key: [0xbb; 32],
///     seq: 5,
///     reason_class: FailureReason::WorkerCrashed,
///     reporter_id: 0xdead_beef,
/// };
/// let bytes = encode_entry(&entry).unwrap();
/// let decoded = decode_entry(&bytes).unwrap();
/// assert_eq!(decoded, entry);
/// ```
pub fn encode_entry(entry: &JournalEntry) -> Result<Vec<u8>, EncodeError> {
    // Reserve worst-case Committed-with-128-parents.
    let mut out = Vec::with_capacity(MAX_ENTRY_LEN);
    let kind = entry.kind();

    // -------------------- HEADER (74 bytes) --------------------
    out.push(kind);
    let (mote_id_for_header, idempotency_key, seq, nd_byte) = match entry {
        JournalEntry::Proposed {
            mote_id,
            idempotency_key,
            seq,
            nondeterminism,
            ..
        }
        | JournalEntry::Committed {
            mote_id,
            idempotency_key,
            seq,
            nondeterminism,
            ..
        } => (*mote_id, *idempotency_key, *seq, nondeterminism.as_u8()),
        JournalEntry::Repudiated {
            target_mote_id,
            idempotency_key,
            seq,
            ..
        } => (*target_mote_id, *idempotency_key, *seq, 0),
        JournalEntry::Failed {
            mote_id,
            idempotency_key,
            seq,
            ..
        }
        | JournalEntry::EffectStaged {
            mote_id,
            idempotency_key,
            seq,
        } => (*mote_id, *idempotency_key, *seq, 0),
        // v3 (M1.1): the header `mote_id` slot carries the synthetic run-root id;
        // the `idempotency_key` slot is the all-zero sentinel (kind 5 does not
        // dedup); `nondeterminism` is the 0 sentinel (a run is not a Mote).
        JournalEntry::RunRegistered {
            instance_id, seq, ..
        } => (run_root_id(instance_id), ZERO_IDEMPOTENCY_KEY, *seq, 0),
        // v4 (M1.2): same anchoring as RunRegistered — run-root id in the
        // mote_id slot, all-zero idempotency key (kind 6 does not dedup), 0 nd.
        JournalEntry::RunVersionsResolved {
            instance_id, seq, ..
        } => (run_root_id(instance_id), ZERO_IDEMPOTENCY_KEY, *seq, 0),
        // v5 (M2.2c): the header `mote_id` slot carries the synthetic seal-root
        // id derived from `through_seq`; all-zero idempotency key (kind 7 does
        // not dedup), 0 nd (a seal is not a Mote).
        JournalEntry::DigestSealed {
            through_seq, seq, ..
        } => (seal_root_id(*through_seq), ZERO_IDEMPOTENCY_KEY, *seq, 0),
        // v7 (PR-2c-2): the header `mote_id` slot carries the round's shaper id
        // directly; all-zero idempotency key (kind 8 does not dedup), 0 nd (a
        // re-plan-round fact is not a Mote).
        JournalEntry::ReplanRound {
            shaper_mote_id,
            seq,
            ..
        } => (*shaper_mote_id, ZERO_IDEMPOTENCY_KEY, *seq, 0),
        // v8 (PR-2d-1): the header `mote_id` slot carries the turn's (run-salted)
        // id directly; all-zero idempotency key (kind 9 does not dedup), 0 nd (a
        // react-round fact is not a Mote).
        JournalEntry::ReactRound {
            turn_mote_id, seq, ..
        } => (*turn_mote_id, ZERO_IDEMPOTENCY_KEY, *seq, 0),
    };
    out.extend_from_slice(mote_id_for_header.as_bytes());
    out.extend_from_slice(&idempotency_key);
    out.extend_from_slice(&seq.to_le_bytes());
    out.push(nd_byte);
    debug_assert_eq!(out.len(), HEADER_LEN);

    // -------------------- BODY (per kind) --------------------
    match entry {
        JournalEntry::Proposed {
            placement_hint,
            warrant_ref,
            ..
        } => {
            // v2 (D36): Proposed body is 16 bytes (placement_hint u128) + 32
            // bytes (warrant_ref ContentRef) = 48 bytes.
            out.extend_from_slice(&placement_hint.to_le_bytes());
            out.extend_from_slice(warrant_ref.as_bytes());
        }
        JournalEntry::Committed {
            result_ref,
            parents,
            warrant_ref,
            ..
        } => {
            if parents.len() > MAX_PARENTS {
                return Err(EncodeError::TooManyParents { got: parents.len() });
            }
            // v2 (D36): Committed body is 32 bytes (result_ref) + 32 bytes
            // (warrant_ref) + 2 bytes (parents count u16) + N * 34 bytes.
            out.extend_from_slice(result_ref.as_bytes());
            out.extend_from_slice(warrant_ref.as_bytes());
            let count = u16::try_from(parents.len()).expect("checked above");
            out.extend_from_slice(&count.to_le_bytes());
            for p in parents {
                // Anti-pattern guard: Data edges MUST have non_cascade == 0.
                if p.edge_kind == 0 && p.non_cascade != 0 {
                    return Err(EncodeError::DataEdgeNonCascade);
                }
                out.extend_from_slice(p.parent_id.as_bytes());
                out.push(p.edge_kind);
                out.push(p.non_cascade);
            }
        }
        JournalEntry::Repudiated {
            target_mote_id,
            target_committed_seq,
            reason_class,
            repudiator_id,
            ..
        } => {
            out.extend_from_slice(target_mote_id.as_bytes());
            out.extend_from_slice(&target_committed_seq.to_le_bytes());
            out.push(reason_class.as_u8());
            out.extend_from_slice(&repudiator_id.to_le_bytes());
        }
        JournalEntry::Failed {
            reason_class,
            reporter_id,
            ..
        } => {
            out.push(reason_class.as_u8());
            out.extend_from_slice(&reporter_id.to_le_bytes());
        }
        JournalEntry::EffectStaged { .. } => {
            // v2 (D38 §2b): EffectStaged body is HEADER-ONLY. No body bytes.
            // The full carrying information (mote_id + idempotency_key + seq)
            // is in the 74-byte header; the recovery fold reads presence to
            // set `effect_staged_observed` on `MoteInfo`.
        }
        JournalEntry::RunRegistered {
            instance_id,
            recipe_fingerprint,
            ts,
            ..
        } => {
            // v3 (M1.1): body = instance_id(16) ‖ recipe_fingerprint(32) ‖ ts(u64 LE).
            out.extend_from_slice(instance_id);
            out.extend_from_slice(recipe_fingerprint);
            out.extend_from_slice(&ts.to_le_bytes());
        }
        JournalEntry::RunVersionsResolved {
            instance_id,
            warrant_ref,
            model_id,
            capability,
            ..
        } => {
            // v4 (M1.2) + v6 (M2.3b): body = instance_id(16) ‖ warrant_ref(32) ‖
            // u16-prefixed model_id ‖ has_cap(u8) ‖ [if has_cap: u16-prefixed
            // tool_id ‖ u16-prefixed tool_version ‖ kind_tag(u8) ‖ def_hash(32) ‖
            // idempotency_class_tag(u8)].
            out.extend_from_slice(instance_id);
            out.extend_from_slice(warrant_ref.as_bytes());
            push_len_prefixed_str(&mut out, model_id)?;
            match capability {
                None => out.push(0u8),
                Some(cap) => {
                    out.push(1u8);
                    push_len_prefixed_str(&mut out, &cap.tool_id)?;
                    push_len_prefixed_str(&mut out, &cap.tool_version)?;
                    out.push(cap.resolved_kind.as_u8());
                    out.extend_from_slice(cap.resolved_def_hash.as_bytes());
                    out.push(cap.idempotency_class.as_u8());
                }
            }
            // Pathological-id guard (real-id bodies are ~100 B). Mirrors the
            // Committed TooManyParents bound — refuse rather than over-cap.
            if out.len() > MAX_ENTRY_LEN {
                return Err(EncodeError::RunVersionsTooLarge { got: out.len() });
            }
        }
        JournalEntry::DigestSealed {
            through_seq,
            state_digest,
            ..
        } => {
            // v5 (M2.2c): body = through_seq(u64 LE) ‖ state_digest(32) = 40 bytes.
            out.extend_from_slice(&through_seq.to_le_bytes());
            out.extend_from_slice(state_digest);
        }
        JournalEntry::ReplanRound {
            round,
            base_prompt_ref,
            corrected_prompt_ref,
            warrant_ref,
            model_id,
            failed_steps,
            escalation_reason_ref,
            ..
        } => {
            // v7 (PR-2c-2): body = round(u32 LE) ‖ base_prompt_ref(32) ‖
            // corrected_prompt_ref(32) ‖ warrant_ref(32) ‖ u16-prefixed model_id ‖
            // failed_count(u16) ‖ failed_count*32 ‖ has_escalation(u8) ‖ [ref(32)].
            if failed_steps.len() > MAX_REPLAN_FAILED_STEPS {
                return Err(EncodeError::ReplanRoundTooManyFailedSteps {
                    got: failed_steps.len(),
                });
            }
            out.extend_from_slice(&round.to_le_bytes());
            out.extend_from_slice(base_prompt_ref.as_bytes());
            out.extend_from_slice(corrected_prompt_ref.as_bytes());
            out.extend_from_slice(warrant_ref.as_bytes());
            push_len_prefixed_str(&mut out, model_id)?;
            let count = u16::try_from(failed_steps.len()).expect("checked above");
            out.extend_from_slice(&count.to_le_bytes());
            for id in failed_steps {
                out.extend_from_slice(id.as_bytes());
            }
            match escalation_reason_ref {
                None => out.push(0u8),
                Some(r) => {
                    out.push(1u8);
                    out.extend_from_slice(r.as_bytes());
                }
            }
            // Pathological-id guard (mirrors RunVersionsResolved) — refuse rather
            // than over-cap a single oversize entry.
            if out.len() > MAX_ENTRY_LEN {
                return Err(EncodeError::ReplanRoundTooLarge { got: out.len() });
            }
        }
        JournalEntry::ReactRound {
            turn,
            instance_id,
            base_prompt_ref,
            warrant_ref,
            model_id,
            branch,
            max_turns,
            max_tool_calls,
            step_salt,
            is_agentic_launch,
            context_items_ref,
            ..
        } => {
            // v9 (PR-9b-2a): body = turn(u32 LE) ‖ instance_id(16) ‖
            // base_prompt_ref(32) ‖ warrant_ref(32) ‖ u16-prefixed model_id ‖
            // branch_tag(u8) ‖ [if Tool: u16-prefixed tool_id ‖ u16-prefixed
            // tool_version] [if Rejected (v10): u16-prefixed reason] ‖
            // max_turns(u32 LE) ‖ max_tool_calls(u32 LE) ‖
            // step_salt_present(u8: 0|1) ‖ [if 1: step_salt(32)]. The step_salt
            // presence byte is the lone v8→v9 delta (a trailing additive byte,
            // exactly the v5→v6 shape); v8 bodies up-convert by appending `0`.
            // v10 (PR-3) adds branch tag 4 (Rejected) with its reason in the
            // between-tag-and-caps slot — a brand-new tag, so no v9 body grows.
            out.extend_from_slice(&turn.to_le_bytes());
            out.extend_from_slice(instance_id);
            out.extend_from_slice(base_prompt_ref.as_bytes());
            out.extend_from_slice(warrant_ref.as_bytes());
            push_len_prefixed_str(&mut out, model_id)?;
            out.push(branch.as_u8());
            match branch {
                ReactBranch::Tool {
                    tool_id,
                    tool_version,
                } => {
                    push_len_prefixed_str(&mut out, tool_id)?;
                    push_len_prefixed_str(&mut out, tool_version)?;
                }
                // v10 (PR-3): a Rejected round carries its u16-prefixed reason in
                // the same between-tag-and-caps slot the Tool fields occupy.
                ReactBranch::Rejected { reason } => {
                    push_len_prefixed_str(&mut out, reason)?;
                }
                // v13 (T-MULTI-ELEMENT-TOOLCALLS): a ToolBatch round carries its
                // ordered calls in the same between-tag-and-caps slot:
                // count(u16 LE) ‖ count × (u16-prefixed tool_id ‖ u16-prefixed
                // tool_version). A brand-new tag (5), so no v12 body grows.
                ReactBranch::ToolBatch { calls } => {
                    let count = u16::try_from(calls.len())
                        .map_err(|_| EncodeError::ReactRoundTooLarge { got: calls.len() })?;
                    out.extend_from_slice(&count.to_le_bytes());
                    for (tool_id, tool_version) in calls {
                        push_len_prefixed_str(&mut out, tool_id)?;
                        push_len_prefixed_str(&mut out, tool_version)?;
                    }
                }
                ReactBranch::Answer | ReactBranch::DeadLettered | ReactBranch::Pending => {}
            }
            out.extend_from_slice(&max_turns.to_le_bytes());
            out.extend_from_slice(&max_tool_calls.to_le_bytes());
            match step_salt {
                None => out.push(0u8),
                Some(salt) => {
                    out.push(1u8);
                    out.extend_from_slice(salt);
                }
            }
            // v11 (PR-R1): the trailing is_agentic_launch byte — a second additive
            // trailing byte after step_salt (same shape as the v8→v9 step_salt delta).
            // A v10 body has no such byte and up-converts on decode to
            // `step_salt.is_some()` (the OLD Some-means-agentic semantics).
            out.push(u8::from(*is_agentic_launch));
            // v12 (PR-9d): the trailing context_items_ref — `present(0|1) ‖ [if 1:
            // content_ref(32)]`, the same additive-trailing shape as the v8→v9
            // step_salt delta. A v11 body has no such byte and up-converts on decode
            // to `None`. Recorded on the turn-0 anchor (None on every other fact).
            match context_items_ref {
                None => out.push(0u8),
                Some(r) => {
                    out.push(1u8);
                    out.extend_from_slice(r.as_bytes());
                }
            }
            // Pathological-id guard (mirrors ReplanRound) — refuse rather than
            // over-cap a single oversize entry.
            if out.len() > MAX_ENTRY_LEN {
                return Err(EncodeError::ReactRoundTooLarge { got: out.len() });
            }
        }
    }

    debug_assert!(out.len() <= MAX_ENTRY_LEN);
    Ok(out)
}

/// Decode a `JournalEntry` from its canonical on-disk byte representation.
///
/// For `Committed` entries the `mote_def_hash` field is **not** in the canonical bytes
/// (per `journal-entry.md` §4.2); the caller (the journal backend) supplies it from
/// its own metadata column. We expose two decoders:
///   - [`decode_entry`] — for non-Committed kinds; returns the entry directly.
///   - [`decode_entry_with_def_hash`] — for Committed kinds; takes the metadata.
///
/// To keep one decoder signature, [`decode_entry`] returns `Committed` with a sentinel
/// `mote_def_hash` of all-zeros; callers MUST overwrite from their metadata column.
pub fn decode_entry(bytes: &[u8]) -> Result<JournalEntry, DecodeError> {
    decode_entry_with_def_hash(bytes, kx_mote::MoteDefHash::from_bytes([0u8; 32]))
}

/// As [`decode_entry`], but supplies the `mote_def_hash` metadata for Committed entries
/// (no-op for the other three kinds).
pub fn decode_entry_with_def_hash(
    bytes: &[u8],
    mote_def_hash: kx_mote::MoteDefHash,
) -> Result<JournalEntry, DecodeError> {
    if bytes.len() > MAX_ENTRY_LEN {
        return Err(DecodeError::TooLarge { got: bytes.len() });
    }
    if bytes.len() < HEADER_LEN {
        return Err(DecodeError::HeaderTooShort { got: bytes.len() });
    }

    // -------------------- Header --------------------
    let kind = bytes[0];
    let mote_id = {
        let mut buf = [0u8; 32];
        buf.copy_from_slice(&bytes[1..33]);
        MoteId::from_bytes(buf)
    };
    let idempotency_key = {
        let mut buf = [0u8; 32];
        buf.copy_from_slice(&bytes[33..65]);
        buf
    };
    let seq = u64::from_le_bytes(bytes[65..73].try_into().expect("8 bytes"));
    let nd_byte = bytes[73];

    let body = &bytes[HEADER_LEN..];

    // -------------------- Body, by kind --------------------
    match kind {
        KIND_PROPOSED => {
            // v2 (D36): Proposed body is 48 bytes (16 placement_hint + 32 warrant_ref).
            if body.len() < 48 {
                return Err(DecodeError::BodyTooShort {
                    kind,
                    got: body.len(),
                    expected: 48,
                });
            }
            let nondeterminism = nd_class_from_byte(nd_byte)?;
            let placement_hint = u128::from_le_bytes(body[..16].try_into().expect("16 bytes"));
            let mut warrant_ref_bytes = [0u8; 32];
            warrant_ref_bytes.copy_from_slice(&body[16..48]);
            let warrant_ref = ContentRef::from_bytes(warrant_ref_bytes);
            if body.len() > 48 {
                return Err(DecodeError::TrailingBytes(body.len() - 48));
            }
            Ok(JournalEntry::Proposed {
                mote_id,
                idempotency_key,
                seq,
                nondeterminism,
                placement_hint,
                warrant_ref,
            })
        }
        KIND_COMMITTED => {
            // v2 (D36): Committed body is 32 (result_ref) + 32 (warrant_ref)
            // + 2 (parents count) + N * 34 bytes. Fixed prefix is 66 bytes.
            const COMMITTED_PREFIX_LEN: usize = 66;
            if body.len() < COMMITTED_PREFIX_LEN {
                return Err(DecodeError::BodyTooShort {
                    kind,
                    got: body.len(),
                    expected: COMMITTED_PREFIX_LEN,
                });
            }
            let nondeterminism = nd_class_from_byte(nd_byte)?;
            let mut result_ref_bytes = [0u8; 32];
            result_ref_bytes.copy_from_slice(&body[..32]);
            let result_ref = ContentRef::from_bytes(result_ref_bytes);
            let mut warrant_ref_bytes = [0u8; 32];
            warrant_ref_bytes.copy_from_slice(&body[32..64]);
            let warrant_ref = ContentRef::from_bytes(warrant_ref_bytes);
            let n = u16::from_le_bytes(body[64..66].try_into().expect("2 bytes")) as usize;
            let expected_parents_len = COMMITTED_PREFIX_LEN + n * ParentEntry::ENCODED_LEN;
            if body.len() < expected_parents_len {
                return Err(DecodeError::BodyTooShort {
                    kind,
                    got: body.len(),
                    expected: expected_parents_len,
                });
            }
            if body.len() > expected_parents_len {
                return Err(DecodeError::TrailingBytes(
                    body.len() - expected_parents_len,
                ));
            }
            let mut parents: SmallVec<[ParentEntry; 4]> = SmallVec::with_capacity(n);
            for i in 0..n {
                let base = COMMITTED_PREFIX_LEN + i * ParentEntry::ENCODED_LEN;
                let mut pid = [0u8; 32];
                pid.copy_from_slice(&body[base..base + 32]);
                let edge_kind = body[base + 32];
                let non_cascade = body[base + 33];
                if edge_kind > 1 {
                    return Err(DecodeError::UnknownEdgeKind(edge_kind));
                }
                if edge_kind == 0 && non_cascade != 0 {
                    return Err(DecodeError::DataEdgeNonCascade);
                }
                if non_cascade > 1 {
                    return Err(DecodeError::NonBooleanNonCascade(non_cascade));
                }
                parents.push(ParentEntry {
                    parent_id: MoteId::from_bytes(pid),
                    edge_kind,
                    non_cascade,
                });
            }
            Ok(JournalEntry::Committed {
                mote_id,
                idempotency_key,
                seq,
                nondeterminism,
                result_ref,
                parents,
                warrant_ref,
                mote_def_hash,
            })
        }
        KIND_REPUDIATED => {
            if body.len() != 57 {
                return Err(DecodeError::BodyTooShort {
                    kind,
                    got: body.len(),
                    expected: 57,
                });
            }
            let mut tgt = [0u8; 32];
            tgt.copy_from_slice(&body[..32]);
            let target_mote_id = MoteId::from_bytes(tgt);
            // Test #17: body's target_mote_id matches header's mote_id.
            if target_mote_id != mote_id {
                return Err(DecodeError::RepudiatedHeaderMismatch);
            }
            let target_committed_seq =
                u64::from_le_bytes(body[32..40].try_into().expect("8 bytes"));
            let reason_class =
                RepudiationReason::from_u8(body[40]).ok_or(DecodeError::UnknownReason(body[40]))?;
            let repudiator_id = u128::from_le_bytes(body[41..57].try_into().expect("16 bytes"));
            Ok(JournalEntry::Repudiated {
                target_mote_id,
                idempotency_key,
                seq,
                target_committed_seq,
                reason_class,
                repudiator_id,
            })
        }
        KIND_FAILED => {
            if body.len() != 17 {
                return Err(DecodeError::BodyTooShort {
                    kind,
                    got: body.len(),
                    expected: 17,
                });
            }
            let reason_class =
                FailureReason::from_u8(body[0]).ok_or(DecodeError::UnknownReason(body[0]))?;
            let reporter_id = u128::from_le_bytes(body[1..17].try_into().expect("16 bytes"));
            Ok(JournalEntry::Failed {
                mote_id,
                idempotency_key,
                seq,
                reason_class,
                reporter_id,
            })
        }
        KIND_EFFECT_STAGED => {
            // v2 (D38 §2b): EffectStaged body is HEADER-ONLY. Any body bytes
            // are a decoder-side error (trailing bytes per §2 no-trailing-data).
            if !body.is_empty() {
                return Err(DecodeError::TrailingBytes(body.len()));
            }
            Ok(JournalEntry::EffectStaged {
                mote_id,
                idempotency_key,
                seq,
            })
        }
        KIND_RUN_REGISTERED => {
            // v3 (M1.1): body = instance_id(16) ‖ recipe_fingerprint(32) ‖ ts(u64 LE).
            // Exact-length (mirrors Repudiated's `!= 57`): over-length surfaces as
            // BodyTooShort rather than a separate TrailingBytes path.
            if body.len() != RUN_REGISTERED_BODY_LEN {
                return Err(DecodeError::BodyTooShort {
                    kind,
                    got: body.len(),
                    expected: RUN_REGISTERED_BODY_LEN,
                });
            }
            let mut instance_id = [0u8; INSTANCE_ID_LEN];
            instance_id.copy_from_slice(&body[..INSTANCE_ID_LEN]);
            // Body-header consistency: the header `mote_id` slot MUST be the
            // synthetic run-root id derived from this `instance_id` (mirrors the
            // Repudiated body-vs-header check).
            if mote_id != run_root_id(&instance_id) {
                return Err(DecodeError::RunRegisteredHeaderMismatch);
            }
            let mut recipe_fingerprint = [0u8; 32];
            recipe_fingerprint.copy_from_slice(&body[INSTANCE_ID_LEN..INSTANCE_ID_LEN + 32]);
            let ts = u64::from_le_bytes(
                body[INSTANCE_ID_LEN + 32..RUN_REGISTERED_BODY_LEN]
                    .try_into()
                    .expect("8 bytes"),
            );
            Ok(JournalEntry::RunRegistered {
                instance_id,
                recipe_fingerprint,
                ts,
                seq,
            })
        }
        KIND_RUN_VERSIONS_RESOLVED => {
            // v4 (M1.2): variable-length body. Cursor-based parse with strict
            // bounds at every read; reject trailing bytes at the end.
            const PREFIX_LEN: usize = INSTANCE_ID_LEN + 32; // instance_id ‖ warrant_ref
            if body.len() < PREFIX_LEN + 2 + 1 {
                // PREFIX + model_id_len(u16) + has_cap(u8)
                return Err(DecodeError::BodyTooShort {
                    kind,
                    got: body.len(),
                    expected: PREFIX_LEN + 2 + 1,
                });
            }
            let mut instance_id = [0u8; INSTANCE_ID_LEN];
            instance_id.copy_from_slice(&body[..INSTANCE_ID_LEN]);
            // Body-header consistency (mirrors RunRegistered).
            if mote_id != run_root_id(&instance_id) {
                return Err(DecodeError::RunVersionsHeaderMismatch);
            }
            let mut warrant_ref_bytes = [0u8; 32];
            warrant_ref_bytes.copy_from_slice(&body[INSTANCE_ID_LEN..PREFIX_LEN]);
            let warrant_ref = ContentRef::from_bytes(warrant_ref_bytes);

            let mut cursor = PREFIX_LEN;
            let model_id = read_len_prefixed_str(body, &mut cursor, kind)?;
            let has_cap = read_u8(body, &mut cursor, kind)?;
            let capability = match has_cap {
                0 => None,
                1 => {
                    let tool_id = read_len_prefixed_str(body, &mut cursor, kind)?;
                    let tool_version = read_len_prefixed_str(body, &mut cursor, kind)?;
                    let tag_byte = read_u8(body, &mut cursor, kind)?;
                    let resolved_kind = ResolvedKindTag::from_u8(tag_byte)
                        .ok_or(DecodeError::UnknownResolvedKind(tag_byte))?;
                    let def_hash_bytes = read_array32(body, &mut cursor, kind)?;
                    // v6 (M2.3b): trailing idempotency_class tag byte.
                    let class_byte = read_u8(body, &mut cursor, kind)?;
                    let idempotency_class = IdempotencyClassTag::from_u8(class_byte)
                        .ok_or(DecodeError::UnknownIdempotencyClass(class_byte))?;
                    Some(ResolvedCapabilityRecord {
                        tool_id,
                        tool_version,
                        resolved_kind,
                        resolved_def_hash: ContentRef::from_bytes(def_hash_bytes),
                        idempotency_class,
                    })
                }
                other => return Err(DecodeError::NonBooleanHasCap(other)),
            };
            if cursor != body.len() {
                return Err(DecodeError::TrailingBytes(body.len() - cursor));
            }
            Ok(JournalEntry::RunVersionsResolved {
                instance_id,
                warrant_ref,
                model_id,
                capability,
                seq,
            })
        }
        KIND_DIGEST_SEALED => {
            // v5 (M2.2c): body = through_seq(u64 LE) ‖ state_digest(32) = 40 bytes.
            // Exact-length (mirrors RunRegistered's `!= RUN_REGISTERED_BODY_LEN`):
            // over-length surfaces as BodyTooShort rather than a separate
            // TrailingBytes path. Panic-free: every read is bounds-checked.
            if body.len() != DIGEST_SEALED_BODY_LEN {
                return Err(DecodeError::BodyTooShort {
                    kind,
                    got: body.len(),
                    expected: DIGEST_SEALED_BODY_LEN,
                });
            }
            let through_seq = u64::from_le_bytes(body[..8].try_into().expect("8 bytes"));
            // Body-header consistency: the header `mote_id` slot MUST be the
            // synthetic seal-root id derived from this `through_seq` (mirrors the
            // RunRegistered body-vs-header check). A mismatch is a tampered or
            // malformed seal — reject loudly (fail-closed).
            if mote_id != seal_root_id(through_seq) {
                return Err(DecodeError::DigestSealedHeaderMismatch);
            }
            let mut state_digest = [0u8; 32];
            state_digest.copy_from_slice(&body[8..DIGEST_SEALED_BODY_LEN]);
            Ok(JournalEntry::DigestSealed {
                through_seq,
                state_digest,
                seq,
            })
        }
        KIND_REPLAN_ROUND => {
            // v7 (PR-2c-2): variable-length body. Cursor-based parse with strict
            // bounds at every read; reject trailing bytes at the end. The header
            // `mote_id` slot IS the round's shaper id (read directly, like
            // Proposed/Committed — no synthetic root to re-derive).
            if body.len() < REPLAN_ROUND_PREFIX_LEN {
                return Err(DecodeError::BodyTooShort {
                    kind,
                    got: body.len(),
                    expected: REPLAN_ROUND_PREFIX_LEN,
                });
            }
            let shaper_mote_id = mote_id;
            let round = u32::from_le_bytes(body[..4].try_into().expect("4 bytes"));
            let mut cursor = 4usize;
            let base_prompt_ref = ContentRef::from_bytes(read_array32(body, &mut cursor, kind)?);
            let corrected_prompt_ref =
                ContentRef::from_bytes(read_array32(body, &mut cursor, kind)?);
            let warrant_ref = ContentRef::from_bytes(read_array32(body, &mut cursor, kind)?);
            let model_id = read_len_prefixed_str(body, &mut cursor, kind)?;
            let failed_count = read_u16(body, &mut cursor, kind)? as usize;
            if failed_count > MAX_REPLAN_FAILED_STEPS {
                return Err(DecodeError::ReplanRoundTooManyFailedSteps { got: failed_count });
            }
            let mut failed_steps: SmallVec<[MoteId; 4]> = SmallVec::with_capacity(failed_count);
            for _ in 0..failed_count {
                failed_steps.push(MoteId::from_bytes(read_array32(body, &mut cursor, kind)?));
            }
            let has_escalation = read_u8(body, &mut cursor, kind)?;
            let escalation_reason_ref = match has_escalation {
                0 => None,
                1 => Some(ContentRef::from_bytes(read_array32(
                    body,
                    &mut cursor,
                    kind,
                )?)),
                other => return Err(DecodeError::NonBooleanHasEscalation(other)),
            };
            if cursor != body.len() {
                return Err(DecodeError::TrailingBytes(body.len() - cursor));
            }
            Ok(JournalEntry::ReplanRound {
                round,
                shaper_mote_id,
                base_prompt_ref,
                corrected_prompt_ref,
                warrant_ref,
                model_id,
                failed_steps,
                escalation_reason_ref,
                seq,
            })
        }
        KIND_REACT_ROUND => {
            // v8 (PR-2d-1): variable-length body. Cursor-based parse with strict
            // bounds at every read; reject trailing bytes at the end. The header
            // `mote_id` slot IS the turn's (run-salted) id (read directly, like
            // ReplanRound — no synthetic root to re-derive).
            if body.len() < REACT_ROUND_PREFIX_LEN {
                return Err(DecodeError::BodyTooShort {
                    kind,
                    got: body.len(),
                    expected: REACT_ROUND_PREFIX_LEN,
                });
            }
            let turn_mote_id = mote_id;
            let turn = u32::from_le_bytes(body[..4].try_into().expect("4 bytes"));
            let mut cursor = 4usize;
            let mut instance_id = [0u8; INSTANCE_ID_LEN];
            instance_id.copy_from_slice(&body[cursor..cursor + INSTANCE_ID_LEN]);
            cursor += INSTANCE_ID_LEN;
            let base_prompt_ref = ContentRef::from_bytes(read_array32(body, &mut cursor, kind)?);
            let warrant_ref = ContentRef::from_bytes(read_array32(body, &mut cursor, kind)?);
            let model_id = read_len_prefixed_str(body, &mut cursor, kind)?;
            let branch = match read_u8(body, &mut cursor, kind)? {
                0 => ReactBranch::Answer,
                1 => {
                    let tool_id = read_len_prefixed_str(body, &mut cursor, kind)?;
                    let tool_version = read_len_prefixed_str(body, &mut cursor, kind)?;
                    ReactBranch::Tool {
                        tool_id,
                        tool_version,
                    }
                }
                2 => ReactBranch::DeadLettered,
                3 => ReactBranch::Pending,
                // v10 (PR-3): a Rejected round's u16-prefixed reason.
                4 => {
                    let reason = read_len_prefixed_str(body, &mut cursor, kind)?;
                    ReactBranch::Rejected { reason }
                }
                // v13 (T-MULTI-ELEMENT-TOOLCALLS): a ToolBatch round's count(u16)
                // ‖ count × (u16-prefixed tool_id ‖ u16-prefixed tool_version).
                5 => {
                    let count = read_u16(body, &mut cursor, kind)? as usize;
                    if count > MAX_TOOL_BATCH_CALLS {
                        return Err(DecodeError::ReactRoundTooManyBatchCalls { got: count });
                    }
                    let mut calls: Vec<(String, String)> = Vec::with_capacity(count);
                    for _ in 0..count {
                        let tool_id = read_len_prefixed_str(body, &mut cursor, kind)?;
                        let tool_version = read_len_prefixed_str(body, &mut cursor, kind)?;
                        calls.push((tool_id, tool_version));
                    }
                    ReactBranch::ToolBatch { calls }
                }
                other => return Err(DecodeError::UnknownReactBranch(other)),
            };
            let max_turns = read_u32(body, &mut cursor, kind)?;
            let max_tool_calls = read_u32(body, &mut cursor, kind)?;
            // v9 (PR-9b-2a): the trailing step_salt presence byte (a v8 body
            // up-converts by appending `0`). Strict: an unknown presence tag is a
            // decode error; the trailing-bytes check below still fences the end.
            let step_salt = match read_u8(body, &mut cursor, kind)? {
                0 => None,
                1 => Some(read_array32(body, &mut cursor, kind)?),
                other => return Err(DecodeError::UnknownReactStepSaltTag(other)),
            };
            // v11 (PR-R1): the trailing is_agentic_launch byte. A v10 body has no
            // such byte (cursor is already at the end) ⇒ up-convert to the OLD
            // semantics (`step_salt.is_some()` WAS the agentic discriminator). A v11
            // body carries an explicit `0|1` (a run-level chain is now Some-salted yet
            // `false`). The trailing-bytes check below still fences a malformed tail.
            let is_agentic_launch = if cursor < body.len() {
                match read_u8(body, &mut cursor, kind)? {
                    0 => false,
                    1 => true,
                    other => return Err(DecodeError::UnknownReactAgenticLaunchTag(other)),
                }
            } else {
                step_salt.is_some()
            };
            // v12 (PR-9d): the trailing context_items_ref — `present(0|1) ‖ [if 1:
            // content_ref(32)]`. A v11 (or older) body has no such byte (cursor is at
            // the end after is_agentic_launch) ⇒ up-convert to `None`. The trailing-
            // bytes check below still fences a malformed tail.
            let context_items_ref = if cursor < body.len() {
                match read_u8(body, &mut cursor, kind)? {
                    0 => None,
                    1 => Some(ContentRef::from_bytes(read_array32(
                        body,
                        &mut cursor,
                        kind,
                    )?)),
                    other => return Err(DecodeError::UnknownReactContextItemsTag(other)),
                }
            } else {
                None
            };
            if cursor != body.len() {
                return Err(DecodeError::TrailingBytes(body.len() - cursor));
            }
            Ok(JournalEntry::ReactRound {
                turn,
                turn_mote_id,
                instance_id,
                base_prompt_ref,
                warrant_ref,
                model_id,
                branch,
                max_turns,
                max_tool_calls,
                step_salt,
                is_agentic_launch,
                context_items_ref,
                seq,
            })
        }
        other => Err(DecodeError::UnknownKind(other)),
    }
}

/// Push a `u16`-length-prefixed UTF-8 string (v4 `RunVersionsResolved` bodies).
fn push_len_prefixed_str(out: &mut Vec<u8>, s: &str) -> Result<(), EncodeError> {
    let len = u16::try_from(s.len())
        .map_err(|_| EncodeError::RunVersionsFieldTooLong { got: s.len() })?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(s.as_bytes());
    Ok(())
}

/// Read a `u16`-length-prefixed UTF-8 string at `*cursor`, advancing it. Strict
/// bounds + UTF-8 validation; total (never panics).
fn read_len_prefixed_str(body: &[u8], cursor: &mut usize, kind: u8) -> Result<String, DecodeError> {
    if body.len() < *cursor + 2 {
        return Err(DecodeError::BodyTooShort {
            kind,
            got: body.len(),
            expected: *cursor + 2,
        });
    }
    let len = u16::from_le_bytes(body[*cursor..*cursor + 2].try_into().expect("2 bytes")) as usize;
    *cursor += 2;
    if body.len() < *cursor + len {
        return Err(DecodeError::BodyTooShort {
            kind,
            got: body.len(),
            expected: *cursor + len,
        });
    }
    let s = std::str::from_utf8(&body[*cursor..*cursor + len])
        .map_err(|_| DecodeError::RunVersionsInvalidUtf8)?
        .to_owned();
    *cursor += len;
    Ok(s)
}

/// Read a single byte at `*cursor`, advancing it.
fn read_u8(body: &[u8], cursor: &mut usize, kind: u8) -> Result<u8, DecodeError> {
    if body.len() < *cursor + 1 {
        return Err(DecodeError::BodyTooShort {
            kind,
            got: body.len(),
            expected: *cursor + 1,
        });
    }
    let b = body[*cursor];
    *cursor += 1;
    Ok(b)
}

/// Read a little-endian `u16` at `*cursor`, advancing it.
fn read_u16(body: &[u8], cursor: &mut usize, kind: u8) -> Result<u16, DecodeError> {
    if body.len() < *cursor + 2 {
        return Err(DecodeError::BodyTooShort {
            kind,
            got: body.len(),
            expected: *cursor + 2,
        });
    }
    let v = u16::from_le_bytes(body[*cursor..*cursor + 2].try_into().expect("2 bytes"));
    *cursor += 2;
    Ok(v)
}

/// Read a little-endian `u32` at `*cursor`, advancing it (v8 `ReactRound` bodies).
fn read_u32(body: &[u8], cursor: &mut usize, kind: u8) -> Result<u32, DecodeError> {
    if body.len() < *cursor + 4 {
        return Err(DecodeError::BodyTooShort {
            kind,
            got: body.len(),
            expected: *cursor + 4,
        });
    }
    let v = u32::from_le_bytes(body[*cursor..*cursor + 4].try_into().expect("4 bytes"));
    *cursor += 4;
    Ok(v)
}

/// Read a 32-byte array at `*cursor`, advancing it.
fn read_array32(body: &[u8], cursor: &mut usize, kind: u8) -> Result<[u8; 32], DecodeError> {
    if body.len() < *cursor + 32 {
        return Err(DecodeError::BodyTooShort {
            kind,
            got: body.len(),
            expected: *cursor + 32,
        });
    }
    let mut buf = [0u8; 32];
    buf.copy_from_slice(&body[*cursor..*cursor + 32]);
    *cursor += 32;
    Ok(buf)
}

fn nd_class_from_byte(b: u8) -> Result<NdClass, DecodeError> {
    match b {
        0 => Ok(NdClass::Pure),
        1 => Ok(NdClass::ReadOnlyNondet),
        2 => Ok(NdClass::WorldMutating),
        _ => Err(DecodeError::UnknownNdClass(b)),
    }
}

// ---------------------------------------------------------------------------
// Derived idempotency key for Repudiated entries (D15, journal-txn.md §10)
// ---------------------------------------------------------------------------

/// Derive the `idempotency_key` for a `Repudiated` entry. Two repudiations of the
/// same `(target_mote_id, target_committed_seq)` pair produce identical keys and
/// dedupe via the journal's standard dedupe-by-key path.
///
/// `blake3("repudiation" ‖ target_mote_id ‖ target_committed_seq_le)`
#[must_use]
pub fn repudiation_idempotency_key(target_mote_id: &MoteId, target_committed_seq: u64) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"repudiation");
    hasher.update(target_mote_id.as_bytes());
    hasher.update(&target_committed_seq.to_le_bytes());
    *hasher.finalize().as_bytes()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use kx_mote::{EdgeKind, MoteDefHash, NdClass};

    fn sample_committed() -> JournalEntry {
        JournalEntry::Committed {
            mote_id: MoteId::from_bytes([7u8; 32]),
            idempotency_key: [8u8; 32],
            seq: 42,
            nondeterminism: NdClass::ReadOnlyNondet,
            result_ref: ContentRef::from_bytes([9u8; 32]),
            parents: SmallVec::new(),
            warrant_ref: ContentRef::from_bytes([0xaa; 32]),
            mote_def_hash: MoteDefHash::from_bytes([10u8; 32]),
        }
    }

    #[test]
    fn header_is_74_bytes_for_every_kind() {
        let kinds = [
            JournalEntry::Proposed {
                mote_id: MoteId::from_bytes([1u8; 32]),
                idempotency_key: [2u8; 32],
                seq: 0,
                nondeterminism: NdClass::Pure,
                placement_hint: 0,
                warrant_ref: ContentRef::from_bytes([0xaa; 32]),
            },
            sample_committed(),
            JournalEntry::Repudiated {
                target_mote_id: MoteId::from_bytes([1u8; 32]),
                idempotency_key: [2u8; 32],
                seq: 0,
                target_committed_seq: 0,
                reason_class: RepudiationReason::OperatorAction,
                repudiator_id: 0,
            },
            JournalEntry::Failed {
                mote_id: MoteId::from_bytes([1u8; 32]),
                idempotency_key: [2u8; 32],
                seq: 0,
                reason_class: FailureReason::TimedOut,
                reporter_id: 0,
            },
            // v2 (D38 §2b): EffectStaged. Header-only; body is empty.
            JournalEntry::EffectStaged {
                mote_id: MoteId::from_bytes([1u8; 32]),
                idempotency_key: [2u8; 32],
                seq: 0,
            },
            // v3 (M1.1): RunRegistered. Header carries the synthetic run-root id.
            JournalEntry::RunRegistered {
                instance_id: [3u8; INSTANCE_ID_LEN],
                recipe_fingerprint: [4u8; 32],
                ts: 0,
                seq: 0,
            },
            // v5 (M2.2c): DigestSealed. Header carries the synthetic seal-root id.
            JournalEntry::DigestSealed {
                through_seq: 0,
                state_digest: [5u8; 32],
                seq: 0,
            },
        ];
        for e in &kinds {
            let bytes = encode_entry(e).unwrap();
            assert!(bytes.len() >= HEADER_LEN, "entry header too short");
            // The first byte is the kind discriminant.
            assert_eq!(bytes[0], e.kind());
        }
    }

    #[test]
    fn proposed_total_length_is_122() {
        // v2 (D36): 74 header + 16 placement_hint + 32 warrant_ref = 122 bytes.
        let e = JournalEntry::Proposed {
            mote_id: MoteId::from_bytes([1u8; 32]),
            idempotency_key: [2u8; 32],
            seq: 0,
            nondeterminism: NdClass::Pure,
            placement_hint: 0,
            warrant_ref: ContentRef::from_bytes([0xaa; 32]),
        };
        assert_eq!(encode_entry(&e).unwrap().len(), 122);
    }

    #[test]
    fn committed_zero_parents_is_140() {
        // v2 (D36): 74 header + 32 result_ref + 32 warrant_ref + 2 parents-count
        // + 0 parents = 140 bytes.
        let bytes = encode_entry(&sample_committed()).unwrap();
        assert_eq!(bytes.len(), 140);
    }

    #[test]
    fn committed_four_parents_is_276() {
        // v2 (D36): 140 (zero-parents baseline) + 4 * 34 = 276 bytes.
        let parents: SmallVec<[ParentEntry; 4]> = (0..4u8)
            .map(|i| ParentEntry {
                parent_id: MoteId::from_bytes([i; 32]),
                edge_kind: 1,
                non_cascade: 0,
            })
            .collect();
        let e = JournalEntry::Committed {
            mote_id: MoteId::from_bytes([7u8; 32]),
            idempotency_key: [8u8; 32],
            seq: 42,
            nondeterminism: NdClass::ReadOnlyNondet,
            result_ref: ContentRef::from_bytes([9u8; 32]),
            parents,
            warrant_ref: ContentRef::from_bytes([0xaa; 32]),
            mote_def_hash: MoteDefHash::from_bytes([10u8; 32]),
        };
        assert_eq!(encode_entry(&e).unwrap().len(), 276);
    }

    #[test]
    fn repudiated_total_length_is_131() {
        // Unchanged in v2 — Repudiated body has no warrant_ref (the Repudiated
        // references a Committed which already carries its own warrant_ref).
        let e = JournalEntry::Repudiated {
            target_mote_id: MoteId::from_bytes([1u8; 32]),
            idempotency_key: [2u8; 32],
            seq: 0,
            target_committed_seq: 0,
            reason_class: RepudiationReason::OperatorAction,
            repudiator_id: 0,
        };
        assert_eq!(encode_entry(&e).unwrap().len(), 131);
    }

    #[test]
    fn failed_total_length_is_91() {
        // Unchanged in v2 — Failed body has no warrant_ref (the per-attempt
        // failure references the prior Proposed which carries warrant_ref).
        let e = JournalEntry::Failed {
            mote_id: MoteId::from_bytes([1u8; 32]),
            idempotency_key: [2u8; 32],
            seq: 0,
            reason_class: FailureReason::TimedOut,
            reporter_id: 0,
        };
        assert_eq!(encode_entry(&e).unwrap().len(), 91);
    }

    #[test]
    fn effect_staged_total_length_is_header_only_74() {
        // v2 (D38 §2b): EffectStaged is header-only.
        let e = JournalEntry::EffectStaged {
            mote_id: MoteId::from_bytes([1u8; 32]),
            idempotency_key: [2u8; 32],
            seq: 0,
        };
        assert_eq!(encode_entry(&e).unwrap().len(), HEADER_LEN);
        assert_eq!(encode_entry(&e).unwrap().len(), 74);
    }

    #[test]
    fn absolute_cap_at_max_parents_is_4492() {
        // v2 (D36): 74 header + 32 result_ref + 32 warrant_ref + 2 parents-count
        // + 128 * 34 parent bytes = 4492 bytes. MAX_ENTRY_LEN is 4500, leaving
        // 8 bytes of headroom for a future per-body addition.
        let parents: SmallVec<[ParentEntry; 4]> = (0..MAX_PARENTS as u32)
            .map(|i| ParentEntry {
                parent_id: MoteId::from_bytes([(i & 0xff) as u8; 32]),
                edge_kind: 1,
                non_cascade: 0,
            })
            .collect();
        let e = JournalEntry::Committed {
            mote_id: MoteId::from_bytes([0u8; 32]),
            idempotency_key: [0u8; 32],
            seq: 0,
            nondeterminism: NdClass::ReadOnlyNondet,
            result_ref: ContentRef::from_bytes([0u8; 32]),
            parents,
            warrant_ref: ContentRef::from_bytes([0u8; 32]),
            mote_def_hash: MoteDefHash::from_bytes([0u8; 32]),
        };
        let bytes = encode_entry(&e).unwrap();
        assert_eq!(bytes.len(), 4492);
        assert!(bytes.len() <= MAX_ENTRY_LEN);
    }

    #[test]
    fn encode_rejects_over_max_parents() {
        let parents: SmallVec<[ParentEntry; 4]> = (0..MAX_PARENTS as u32 + 1)
            .map(|_| ParentEntry {
                parent_id: MoteId::from_bytes([0u8; 32]),
                edge_kind: 1,
                non_cascade: 0,
            })
            .collect();
        let e = JournalEntry::Committed {
            mote_id: MoteId::from_bytes([0u8; 32]),
            idempotency_key: [0u8; 32],
            seq: 0,
            nondeterminism: NdClass::ReadOnlyNondet,
            result_ref: ContentRef::from_bytes([0u8; 32]),
            parents,
            warrant_ref: ContentRef::from_bytes([0u8; 32]),
            mote_def_hash: MoteDefHash::from_bytes([0u8; 32]),
        };
        assert!(matches!(
            encode_entry(&e),
            Err(EncodeError::TooManyParents { .. })
        ));
    }

    #[test]
    fn encode_rejects_data_edge_with_non_cascade_set() {
        let parents: SmallVec<[ParentEntry; 4]> = smallvec::smallvec![ParentEntry {
            parent_id: MoteId::from_bytes([0u8; 32]),
            edge_kind: 0,   // Data
            non_cascade: 1, // forbidden
        }];
        let e = JournalEntry::Committed {
            mote_id: MoteId::from_bytes([0u8; 32]),
            idempotency_key: [0u8; 32],
            seq: 0,
            nondeterminism: NdClass::ReadOnlyNondet,
            result_ref: ContentRef::from_bytes([0u8; 32]),
            parents,
            warrant_ref: ContentRef::from_bytes([0u8; 32]),
            mote_def_hash: MoteDefHash::from_bytes([0u8; 32]),
        };
        assert_eq!(encode_entry(&e), Err(EncodeError::DataEdgeNonCascade));
    }

    #[test]
    fn round_trip_each_kind() {
        // Committed (re-decoded with the original def_hash)
        let c = sample_committed();
        let bytes = encode_entry(&c).unwrap();
        let def_hash = if let JournalEntry::Committed { mote_def_hash, .. } = &c {
            *mote_def_hash
        } else {
            unreachable!()
        };
        let decoded = decode_entry_with_def_hash(&bytes, def_hash).unwrap();
        assert_eq!(decoded, c);

        // Proposed
        let p = JournalEntry::Proposed {
            mote_id: MoteId::from_bytes([3u8; 32]),
            idempotency_key: [4u8; 32],
            seq: 7,
            nondeterminism: NdClass::WorldMutating,
            placement_hint: 0xDEAD_BEEF_CAFE_BABE,
            warrant_ref: ContentRef::from_bytes([0xbb; 32]),
        };
        assert_eq!(decode_entry(&encode_entry(&p).unwrap()).unwrap(), p);

        // Repudiated
        let r = JournalEntry::Repudiated {
            target_mote_id: MoteId::from_bytes([5u8; 32]),
            idempotency_key: repudiation_idempotency_key(&MoteId::from_bytes([5u8; 32]), 99),
            seq: 100,
            target_committed_seq: 99,
            reason_class: RepudiationReason::UpstreamCascade,
            repudiator_id: 0x1234,
        };
        assert_eq!(decode_entry(&encode_entry(&r).unwrap()).unwrap(), r);

        // Failed
        let f = JournalEntry::Failed {
            mote_id: MoteId::from_bytes([6u8; 32]),
            idempotency_key: [11u8; 32],
            seq: 50,
            reason_class: FailureReason::UnsafeWorldMutatingConstruction,
            reporter_id: 0xABCD,
        };
        assert_eq!(decode_entry(&encode_entry(&f).unwrap()).unwrap(), f);

        // v2 (D38 §2b): EffectStaged
        let es = JournalEntry::EffectStaged {
            mote_id: MoteId::from_bytes([12u8; 32]),
            idempotency_key: [13u8; 32],
            seq: 200,
        };
        assert_eq!(decode_entry(&encode_entry(&es).unwrap()).unwrap(), es);

        // v3 (M1.1): RunRegistered
        let rr = JournalEntry::RunRegistered {
            instance_id: [0x5a; INSTANCE_ID_LEN],
            recipe_fingerprint: [0x6b; 32],
            ts: 0x0123_4567_89ab_cdef,
            seq: 300,
        };
        assert_eq!(decode_entry(&encode_entry(&rr).unwrap()).unwrap(), rr);

        // v7 (PR-2c-2): ReplanRound — with failed steps + an escalation ref.
        let rp = JournalEntry::ReplanRound {
            round: 2,
            shaper_mote_id: MoteId::from_bytes([0x7c; 32]),
            base_prompt_ref: ContentRef::from_bytes([0x11; 32]),
            corrected_prompt_ref: ContentRef::from_bytes([0x22; 32]),
            warrant_ref: ContentRef::from_bytes([0x33; 32]),
            model_id: "kx-serve:qwen3-4b".to_string(),
            failed_steps: smallvec::smallvec![
                MoteId::from_bytes([0x44; 32]),
                MoteId::from_bytes([0x55; 32]),
            ],
            escalation_reason_ref: Some(ContentRef::from_bytes([0x66; 32])),
            seq: 400,
        };
        assert_eq!(decode_entry(&encode_entry(&rp).unwrap()).unwrap(), rp);

        // v7: ReplanRound — the round-0 anchor shape (no failures, no escalation).
        let anchor = JournalEntry::ReplanRound {
            round: 0,
            shaper_mote_id: MoteId::from_bytes([0x7d; 32]),
            base_prompt_ref: ContentRef::from_bytes([0x11; 32]),
            corrected_prompt_ref: ContentRef::from_bytes([0x11; 32]),
            warrant_ref: ContentRef::from_bytes([0x33; 32]),
            model_id: String::new(),
            failed_steps: smallvec::smallvec![],
            escalation_reason_ref: None,
            seq: 401,
        };
        assert_eq!(
            decode_entry(&encode_entry(&anchor).unwrap()).unwrap(),
            anchor
        );

        // v9 (PR-9b-2a): ReactRound — every branch shape round-trips, under both
        // step_salt absent (None, the run-level chain) and present (Some, an
        // agentic step's private chain). v10 (PR-3) adds the Rejected branch,
        // whose u16-prefixed reason must survive the round-trip too.
        for (branch, seq) in [
            (ReactBranch::Pending, 500u64),
            (ReactBranch::Answer, 501),
            (
                ReactBranch::Tool {
                    tool_id: "mcp-echo".to_string(),
                    tool_version: "1".to_string(),
                },
                502,
            ),
            (ReactBranch::DeadLettered, 503),
            (
                ReactBranch::Rejected {
                    reason: "args do not match mcp-echo/echo@1 inputSchema: unknown param `text`"
                        .to_string(),
                },
                504,
            ),
            // An empty reason is a valid edge (the encoder length-prefixes it).
            (
                ReactBranch::Rejected {
                    reason: String::new(),
                },
                505,
            ),
            // v13 (T-MULTI-ELEMENT-TOOLCALLS): a ToolBatch's ordered calls survive
            // the round-trip — including two calls to the SAME tool (the per-call
            // observation ids are disambiguated downstream by call_index, not here).
            (
                ReactBranch::ToolBatch {
                    calls: vec![
                        ("mcp-echo".to_string(), "1".to_string()),
                        ("fs-read".to_string(), "1".to_string()),
                    ],
                },
                506,
            ),
            (
                ReactBranch::ToolBatch {
                    calls: vec![
                        ("mcp-echo".to_string(), "1".to_string()),
                        ("mcp-echo".to_string(), "1".to_string()),
                    ],
                },
                507,
            ),
        ] {
            // v11 (PR-R1): every (step_salt, is_agentic_launch) combination round-trips
            // — run-level (None/false, Some/false) and agentic (Some/true).
            for step_salt in [None, Some([0x5a_u8; 32])] {
                for is_agentic_launch in [false, true] {
                    // v12 (PR-9d): every context_items_ref (absent + present) round-trips.
                    for context_items_ref in [None, Some(ContentRef::from_bytes([0x77; 32]))] {
                        let rt = JournalEntry::ReactRound {
                            turn: 2,
                            turn_mote_id: MoteId::from_bytes([0x8e; 32]),
                            instance_id: [0x4d; INSTANCE_ID_LEN],
                            base_prompt_ref: ContentRef::from_bytes([0x12; 32]),
                            warrant_ref: ContentRef::from_bytes([0x34; 32]),
                            model_id: "kx-serve:qwen3-4b".to_string(),
                            branch: branch.clone(),
                            max_turns: 8,
                            max_tool_calls: 8,
                            step_salt,
                            is_agentic_launch,
                            context_items_ref,
                            seq,
                        };
                        assert_eq!(decode_entry(&encode_entry(&rt).unwrap()).unwrap(), rt);
                    }
                }
            }
        }
    }

    /// v12 (PR-9d): the `context_items_ref` presence byte is the lone v11→v12 delta,
    /// stacked on the v10→v11 `is_agentic_launch` byte. For a None/false/None entry the
    /// body tail is `step_salt(0) ‖ is_agentic_launch(0) ‖ context_items_ref(0)`: a v11
    /// body is the v12 body minus its FINAL byte (context up-converts to None); a v10
    /// body is the v12 body minus its final TWO bytes (is_agentic up-converts to
    /// step_salt.is_some()). Pin both relationships so a drift in the encode tail or the
    /// migration up-converters fails CI.
    #[test]
    fn react_round_v11_and_v10_bodies_are_v12_minus_trailing_bytes() {
        let run_level = JournalEntry::ReactRound {
            turn: 3,
            turn_mote_id: MoteId::from_bytes([0x8e; 32]),
            instance_id: [0x4d; INSTANCE_ID_LEN],
            base_prompt_ref: ContentRef::from_bytes([0x12; 32]),
            warrant_ref: ContentRef::from_bytes([0x34; 32]),
            model_id: "m".to_string(),
            branch: ReactBranch::Answer,
            max_turns: 8,
            max_tool_calls: 6,
            step_salt: None,
            is_agentic_launch: false,
            context_items_ref: None,
            seq: 600,
        };
        let v12 = encode_entry(&run_level).unwrap();
        // Final body byte = context_items_ref present tag (`0`); dropping it yields a
        // valid v11 body whose byte-absent context up-converts to None.
        assert_eq!(*v12.last().unwrap(), 0u8);
        let v11_shape = v12[..v12.len() - 1].to_vec();
        assert_eq!(decode_entry(&v11_shape).unwrap(), run_level);
        // Dropping the final TWO bytes (context + is_agentic) yields a v10 body whose
        // byte-absent is_agentic up-converts to step_salt.is_some() == false.
        let v10_shape = v12[..v12.len() - 2].to_vec();
        assert_eq!(decode_entry(&v10_shape).unwrap(), run_level);
        // Re-appending the two safe-default `0` bytes recovers the v12 encoding exactly.
        let mut reappended = v10_shape;
        reappended.push(0u8); // is_agentic_launch
        reappended.push(0u8); // context_items_ref present
        assert_eq!(reappended, v12);
        // A present context_items_ref appends `present(1) ‖ ref(32)`; the entry round-
        // trips, and its v11-shape (those 33 bytes dropped) up-converts to None — an old
        // binary simply can't see the context (the deliberate one-way forward break).
        let with_ctx = JournalEntry::ReactRound {
            turn: 3,
            turn_mote_id: MoteId::from_bytes([0x8e; 32]),
            instance_id: [0x4d; INSTANCE_ID_LEN],
            base_prompt_ref: ContentRef::from_bytes([0x12; 32]),
            warrant_ref: ContentRef::from_bytes([0x34; 32]),
            model_id: "m".to_string(),
            branch: ReactBranch::Answer,
            max_turns: 8,
            max_tool_calls: 6,
            step_salt: None,
            is_agentic_launch: false,
            context_items_ref: Some(ContentRef::from_bytes([0x77; 32])),
            seq: 600,
        };
        let v12_ctx = encode_entry(&with_ctx).unwrap();
        assert_eq!(v12_ctx.len(), v12.len() + 32); // present byte same; +32 for the ref
        assert_eq!(decode_entry(&v12_ctx).unwrap(), with_ctx);
        assert_eq!(
            decode_entry(&v12_ctx[..v12_ctx.len() - 33]).unwrap(),
            run_level
        );
    }

    #[test]
    fn react_round_is_excluded_from_dedup_and_is_off_metadata() {
        // kind 9 is not in the dedup index {1,2,4}; its idempotency_key is the ZERO
        // sentinel and its mote_id slot carries the turn's (run-salted) id directly.
        let turn_id = MoteId::from_bytes([0xa1; 32]);
        let e = JournalEntry::ReactRound {
            turn: 0,
            turn_mote_id: turn_id,
            instance_id: [0x4d; INSTANCE_ID_LEN],
            base_prompt_ref: ContentRef::from_bytes([0u8; 32]),
            warrant_ref: ContentRef::from_bytes([0u8; 32]),
            model_id: "m".to_string(),
            branch: ReactBranch::Pending,
            max_turns: 8,
            max_tool_calls: 8,
            step_salt: None,
            is_agentic_launch: false,
            context_items_ref: None,
            seq: 11,
        };
        assert_eq!(e.kind(), KIND_REACT_ROUND);
        assert_eq!(e.idempotency_key(), &[0u8; 32]);
        assert_eq!(e.mote_id(), turn_id);
        assert_eq!(e.seq(), 11);
    }

    #[test]
    fn react_round_rejects_unknown_branch_tag_and_trailing_bytes() {
        let e = JournalEntry::ReactRound {
            turn: 1,
            turn_mote_id: MoteId::from_bytes([0xa2; 32]),
            instance_id: [0x4e; INSTANCE_ID_LEN],
            base_prompt_ref: ContentRef::from_bytes([1u8; 32]),
            warrant_ref: ContentRef::from_bytes([2u8; 32]),
            model_id: "m".to_string(),
            branch: ReactBranch::Answer,
            max_turns: 8,
            max_tool_calls: 8,
            step_salt: None,
            is_agentic_launch: false,
            context_items_ref: None,
            seq: 12,
        };
        let bytes = encode_entry(&e).unwrap();
        // Corrupt the branch tag (it sits right after the u16-prefixed model_id;
        // body offset = 4 + 16 + 32 + 32 + 2 + 1 = 87 from the body start).
        let tag_at = HEADER_LEN + REACT_ROUND_PREFIX_LEN + 1;
        let mut corrupt = bytes.clone();
        corrupt[tag_at] = 0xee;
        assert!(matches!(
            decode_entry(&corrupt),
            Err(DecodeError::UnknownReactBranch(0xee))
        ));
        // v12 (PR-9d): the FINAL body byte is now the context_items_ref present tag
        // (0|1 for a None-context entry); an unknown tag is fail-closed.
        let mut bad_context_tag = bytes.clone();
        *bad_context_tag.last_mut().unwrap() = 0xee;
        assert!(matches!(
            decode_entry(&bad_context_tag),
            Err(DecodeError::UnknownReactContextItemsTag(0xee))
        ));
        // v11 (PR-R1): the is_agentic_launch tag now sits SECOND-to-last (before the
        // v12 context byte) for a None entry; an unknown tag is fail-closed.
        let mut bad_launch_tag = bytes.clone();
        let launch_pos = bad_launch_tag.len() - 2;
        bad_launch_tag[launch_pos] = 0xee;
        assert!(matches!(
            decode_entry(&bad_launch_tag),
            Err(DecodeError::UnknownReactAgenticLaunchTag(0xee))
        ));
        // v9 (PR-9b-2a): the step_salt presence byte now sits THIRD-to-last (before
        // the is_agentic_launch + context bytes) for a None entry; fail-closed.
        let mut bad_salt_tag = bytes.clone();
        let salt_pos = bad_salt_tag.len() - 3;
        bad_salt_tag[salt_pos] = 0xee;
        assert!(matches!(
            decode_entry(&bad_salt_tag),
            Err(DecodeError::UnknownReactStepSaltTag(0xee))
        ));
        // Trailing garbage after a complete entry is fail-closed (§2).
        let mut trailing = bytes;
        trailing.push(0);
        assert!(matches!(
            decode_entry(&trailing),
            Err(DecodeError::TrailingBytes(1))
        ));
    }

    /// v13 (T-MULTI-ELEMENT-TOOLCALLS): a `ToolBatch` whose declared call count
    /// exceeds [`MAX_TOOL_BATCH_CALLS`] is fail-closed at decode (the `DoS` bound,
    /// independent of the size cap — mirrors `ReplanRound`'s failed-step bound).
    #[test]
    fn react_round_rejects_over_cap_tool_batch() {
        // A ToolBatch at exactly the cap encodes + round-trips.
        let at_cap: Vec<(String, String)> = (0..MAX_TOOL_BATCH_CALLS)
            .map(|i| (format!("tool-{i}"), "1".to_string()))
            .collect();
        let e = JournalEntry::ReactRound {
            turn: 0,
            turn_mote_id: MoteId::from_bytes([0xa2; 32]),
            instance_id: [0x4e; INSTANCE_ID_LEN],
            base_prompt_ref: ContentRef::from_bytes([1u8; 32]),
            warrant_ref: ContentRef::from_bytes([2u8; 32]),
            model_id: "m".to_string(),
            branch: ReactBranch::ToolBatch { calls: at_cap },
            max_turns: 8,
            max_tool_calls: 20,
            step_salt: None,
            is_agentic_launch: false,
            context_items_ref: None,
            seq: 13,
        };
        let bytes = encode_entry(&e).unwrap();
        assert_eq!(decode_entry(&bytes).unwrap(), e);
        // Hand-craft a body whose ToolBatch count is cap+1: bump the u16 count that
        // sits right after the branch tag. The branch tag is at
        // `prefix + model_id("m"=1)`; the count follows it (so `+ 2`).
        let count_at = HEADER_LEN + REACT_ROUND_PREFIX_LEN + 2;
        let mut over = bytes;
        let over_count = u16::try_from(MAX_TOOL_BATCH_CALLS + 1).unwrap();
        over[count_at..count_at + 2].copy_from_slice(&over_count.to_le_bytes());
        assert!(matches!(
            decode_entry(&over),
            Err(DecodeError::ReactRoundTooManyBatchCalls {
                got
            }) if got == MAX_TOOL_BATCH_CALLS + 1
        ));
    }

    #[test]
    fn replan_round_rejects_over_cap_failed_steps() {
        let failed: SmallVec<[MoteId; 4]> = (0..=MAX_REPLAN_FAILED_STEPS)
            .map(|i| MoteId::from_bytes([u8::try_from(i % 256).unwrap(); 32]))
            .collect();
        let e = JournalEntry::ReplanRound {
            round: 1,
            shaper_mote_id: MoteId::from_bytes([1u8; 32]),
            base_prompt_ref: ContentRef::from_bytes([0u8; 32]),
            corrected_prompt_ref: ContentRef::from_bytes([0u8; 32]),
            warrant_ref: ContentRef::from_bytes([0u8; 32]),
            model_id: "m".to_string(),
            failed_steps: failed,
            escalation_reason_ref: None,
            seq: 0,
        };
        assert!(matches!(
            encode_entry(&e),
            Err(EncodeError::ReplanRoundTooManyFailedSteps { .. })
        ));
    }

    #[test]
    fn replan_round_is_excluded_from_dedup_and_is_off_metadata() {
        // kind 8 is not in the dedup index {1,2,4}; its idempotency_key is the ZERO
        // sentinel and its mote_id slot carries the shaper id directly.
        let shaper = MoteId::from_bytes([0x9a; 32]);
        let e = JournalEntry::ReplanRound {
            round: 3,
            shaper_mote_id: shaper,
            base_prompt_ref: ContentRef::from_bytes([0u8; 32]),
            corrected_prompt_ref: ContentRef::from_bytes([0u8; 32]),
            warrant_ref: ContentRef::from_bytes([0u8; 32]),
            model_id: "m".to_string(),
            failed_steps: smallvec::smallvec![],
            escalation_reason_ref: None,
            seq: 9,
        };
        assert_eq!(e.kind(), KIND_REPLAN_ROUND);
        assert_eq!(e.idempotency_key(), &[0u8; 32]);
        assert_eq!(e.mote_id(), shaper);
        assert_eq!(e.seq(), 9);
    }

    #[test]
    fn run_registered_total_length_is_130() {
        // v3 (M1.1): 74 header + 56 body (16 instance_id + 32 recipe_fingerprint
        // + 8 ts) = 130 bytes.
        let e = JournalEntry::RunRegistered {
            instance_id: [9u8; INSTANCE_ID_LEN],
            recipe_fingerprint: [8u8; 32],
            ts: 42,
            seq: 1,
        };
        assert_eq!(encode_entry(&e).unwrap().len(), 130);
    }

    #[test]
    fn run_registered_header_carries_run_root_id_and_zero_idempotency_key() {
        let instance_id = [0x11u8; INSTANCE_ID_LEN];
        let e = JournalEntry::RunRegistered {
            instance_id,
            recipe_fingerprint: [0x22; 32],
            ts: 7,
            seq: 1,
        };
        let bytes = encode_entry(&e).unwrap();
        // Header mote_id slot = run_root_id(instance_id).
        assert_eq!(&bytes[1..33], run_root_id(&instance_id).as_bytes());
        // Header idempotency_key slot = the all-zero sentinel (kind 5 doesn't dedup).
        assert_eq!(&bytes[33..65], &[0u8; 32]);
        // The accessor agrees.
        assert_eq!(e.idempotency_key(), &[0u8; 32]);
        assert_eq!(e.mote_id(), run_root_id(&instance_id));
        assert_eq!(e.kind(), KIND_RUN_REGISTERED);
    }

    #[test]
    fn decode_rejects_run_registered_body_too_short() {
        let e = JournalEntry::RunRegistered {
            instance_id: [1u8; INSTANCE_ID_LEN],
            recipe_fingerprint: [2u8; 32],
            ts: 3,
            seq: 1,
        };
        let mut bytes = encode_entry(&e).unwrap();
        bytes.pop(); // drop one body byte
        assert!(matches!(
            decode_entry(&bytes),
            Err(DecodeError::BodyTooShort {
                kind: KIND_RUN_REGISTERED,
                ..
            })
        ));
    }

    #[test]
    fn decode_rejects_run_registered_trailing_bytes() {
        let e = JournalEntry::RunRegistered {
            instance_id: [1u8; INSTANCE_ID_LEN],
            recipe_fingerprint: [2u8; 32],
            ts: 3,
            seq: 1,
        };
        let mut bytes = encode_entry(&e).unwrap();
        bytes.push(0xff); // over-length body
                          // Exact-length check (mirrors Repudiated `!= 57`) surfaces as BodyTooShort.
        assert!(matches!(
            decode_entry(&bytes),
            Err(DecodeError::BodyTooShort {
                kind: KIND_RUN_REGISTERED,
                ..
            })
        ));
    }

    #[test]
    fn decode_rejects_run_registered_header_body_mismatch() {
        let e = JournalEntry::RunRegistered {
            instance_id: [0x33u8; INSTANCE_ID_LEN],
            recipe_fingerprint: [0x44; 32],
            ts: 5,
            seq: 1,
        };
        let mut bytes = encode_entry(&e).unwrap();
        // Corrupt the header mote_id slot so it no longer equals
        // run_root_id(instance_id).
        bytes[1] ^= 0xff;
        assert_eq!(
            decode_entry(&bytes).unwrap_err(),
            DecodeError::RunRegisteredHeaderMismatch
        );
    }

    #[test]
    fn run_root_id_is_deterministic_and_domain_separated() {
        let a = [0xaau8; INSTANCE_ID_LEN];
        let b = [0xbbu8; INSTANCE_ID_LEN];
        // Deterministic.
        assert_eq!(run_root_id(&a), run_root_id(&a));
        // Distinct instances → distinct roots.
        assert_ne!(run_root_id(&a), run_root_id(&b));
        // Domain-separated: NOT a bare blake3 of the instance_id (the "kx-run-root"
        // tag must participate), so a run-root id can never collide with any other
        // blake3-of-16-bytes identity.
        let bare = MoteId::from_bytes(*blake3::hash(&a).as_bytes());
        assert_ne!(run_root_id(&a), bare);
    }

    // -------------------- v5 (M2.2c): DigestSealed --------------------

    fn sample_digest_sealed(through_seq: u64) -> JournalEntry {
        JournalEntry::DigestSealed {
            through_seq,
            state_digest: [0x5au8; 32],
            seq: through_seq + 1,
        }
    }

    #[test]
    fn digest_sealed_total_length_is_114() {
        // v5 (M2.2c): 74 header + 40 body (8 through_seq + 32 state_digest) = 114 bytes.
        assert_eq!(encode_entry(&sample_digest_sealed(256)).unwrap().len(), 114);
    }

    #[test]
    fn digest_sealed_round_trips() {
        let e = JournalEntry::DigestSealed {
            through_seq: 0xdead_beef,
            state_digest: [0xa7u8; 32],
            seq: 0xdead_beef + 1,
        };
        let bytes = encode_entry(&e).unwrap();
        assert_eq!(decode_entry(&bytes).unwrap(), e);
    }

    #[test]
    fn digest_sealed_header_carries_seal_root_id_and_zero_idempotency_key() {
        let through_seq = 4096u64;
        let e = sample_digest_sealed(through_seq);
        let bytes = encode_entry(&e).unwrap();
        // Header mote_id slot = seal_root_id(through_seq).
        assert_eq!(&bytes[1..33], seal_root_id(through_seq).as_bytes());
        // Header idempotency_key slot = the all-zero sentinel (kind 7 doesn't dedup).
        assert_eq!(&bytes[33..65], &[0u8; 32]);
        // The nondeterminism slot is the 0 sentinel (a seal is not a Mote).
        assert_eq!(bytes[73], 0);
        // The accessors agree.
        assert_eq!(e.idempotency_key(), &[0u8; 32]);
        assert_eq!(e.mote_id(), seal_root_id(through_seq));
        assert_eq!(e.kind(), KIND_DIGEST_SEALED);
    }

    #[test]
    fn decode_rejects_digest_sealed_body_too_short() {
        let mut bytes = encode_entry(&sample_digest_sealed(7)).unwrap();
        bytes.pop(); // drop one body byte
        assert!(matches!(
            decode_entry(&bytes),
            Err(DecodeError::BodyTooShort {
                kind: KIND_DIGEST_SEALED,
                ..
            })
        ));
    }

    #[test]
    fn decode_rejects_digest_sealed_trailing_bytes() {
        let mut bytes = encode_entry(&sample_digest_sealed(7)).unwrap();
        bytes.push(0xff); // over-length body
                          // Exact-length check (mirrors RunRegistered) surfaces as BodyTooShort.
        assert!(matches!(
            decode_entry(&bytes),
            Err(DecodeError::BodyTooShort {
                kind: KIND_DIGEST_SEALED,
                ..
            })
        ));
    }

    #[test]
    fn decode_rejects_digest_sealed_header_body_mismatch() {
        let mut bytes = encode_entry(&sample_digest_sealed(99)).unwrap();
        // Corrupt the header mote_id slot so it no longer equals
        // seal_root_id(through_seq) — a tampered/forged seal must be rejected.
        bytes[1] ^= 0xff;
        assert_eq!(
            decode_entry(&bytes).unwrap_err(),
            DecodeError::DigestSealedHeaderMismatch
        );
    }

    #[test]
    fn decode_rejects_digest_sealed_inconsistent_through_seq() {
        // A seal whose through_seq is rewritten in the BODY (not the header) so the
        // header seal-root no longer matches → fail-closed. (Flipping the body's
        // through_seq breaks the seal_root_id(through_seq) == header invariant.)
        let mut bytes = encode_entry(&sample_digest_sealed(1234)).unwrap();
        bytes[HEADER_LEN] ^= 0xff; // first body byte = low byte of through_seq
        assert_eq!(
            decode_entry(&bytes).unwrap_err(),
            DecodeError::DigestSealedHeaderMismatch
        );
    }

    #[test]
    fn seal_root_id_is_deterministic_and_domain_separated() {
        // Deterministic.
        assert_eq!(seal_root_id(10), seal_root_id(10));
        // Distinct frontiers → distinct roots.
        assert_ne!(seal_root_id(10), seal_root_id(11));
        // Domain-separated from a bare blake3 of the seq bytes.
        let bare = MoteId::from_bytes(*blake3::hash(&10u64.to_le_bytes()).as_bytes());
        assert_ne!(seal_root_id(10), bare);
        // Domain-separated from run_root_id: a seal root can never collide with a
        // run root even if the byte inputs coincided.
        let inst = [0u8; INSTANCE_ID_LEN];
        assert_ne!(seal_root_id(0), run_root_id(&inst));
    }

    #[test]
    fn digest_sealed_encode_is_deterministic() {
        let e = sample_digest_sealed(2048);
        assert_eq!(encode_entry(&e).unwrap(), encode_entry(&e).unwrap());
    }

    // -------------------- v4 (M1.2): RunVersionsResolved --------------------

    fn sample_run_versions(capability: Option<ResolvedCapabilityRecord>) -> JournalEntry {
        JournalEntry::RunVersionsResolved {
            instance_id: [0x55u8; INSTANCE_ID_LEN],
            warrant_ref: ContentRef::from_bytes([0x66; 32]),
            model_id: "qwen2.5-0.5b".to_owned(),
            capability,
            seq: 7,
        }
    }

    fn sample_capability() -> ResolvedCapabilityRecord {
        ResolvedCapabilityRecord {
            tool_id: "fs-read".to_owned(),
            tool_version: "1.0.0".to_owned(),
            resolved_kind: ResolvedKindTag::Builtin,
            resolved_def_hash: ContentRef::from_bytes([0x77; 32]),
            idempotency_class: IdempotencyClassTag::Readback,
        }
    }

    #[test]
    fn run_versions_round_trips_with_and_without_capability() {
        for cap in [None, Some(sample_capability())] {
            let e = sample_run_versions(cap);
            let bytes = encode_entry(&e).unwrap();
            assert_eq!(decode_entry(&bytes).unwrap(), e);
        }
    }

    #[test]
    fn run_versions_header_carries_run_root_id_and_zero_idempotency_key() {
        let e = sample_run_versions(Some(sample_capability()));
        let instance_id = match &e {
            JournalEntry::RunVersionsResolved { instance_id, .. } => *instance_id,
            _ => unreachable!(),
        };
        let bytes = encode_entry(&e).unwrap();
        assert_eq!(&bytes[1..33], run_root_id(&instance_id).as_bytes());
        assert_eq!(&bytes[33..65], &[0u8; 32]);
        assert_eq!(e.idempotency_key(), &[0u8; 32]);
        assert_eq!(e.mote_id(), run_root_id(&instance_id));
        assert_eq!(e.kind(), KIND_RUN_VERSIONS_RESOLVED);
    }

    #[test]
    fn decode_rejects_run_versions_header_body_mismatch() {
        let e = sample_run_versions(Some(sample_capability()));
        let mut bytes = encode_entry(&e).unwrap();
        bytes[1] ^= 0xff; // corrupt the run-root id in the header
        assert_eq!(
            decode_entry(&bytes).unwrap_err(),
            DecodeError::RunVersionsHeaderMismatch
        );
    }

    #[test]
    fn decode_rejects_run_versions_trailing_bytes() {
        let e = sample_run_versions(Some(sample_capability()));
        let mut bytes = encode_entry(&e).unwrap();
        bytes.push(0xff);
        assert!(matches!(
            decode_entry(&bytes),
            Err(DecodeError::TrailingBytes(1))
        ));
    }

    #[test]
    fn decode_rejects_run_versions_body_too_short() {
        let e = sample_run_versions(Some(sample_capability()));
        let mut bytes = encode_entry(&e).unwrap();
        bytes.pop(); // truncate the def_hash
        assert!(matches!(
            decode_entry(&bytes),
            Err(DecodeError::BodyTooShort {
                kind: KIND_RUN_VERSIONS_RESOLVED,
                ..
            })
        ));
    }

    #[test]
    fn decode_rejects_run_versions_unknown_kind_tag() {
        let e = sample_run_versions(Some(sample_capability()));
        let bytes = encode_entry(&e).unwrap();
        // Body tail (v6): kind_tag(1) ‖ def_hash(32) ‖ class_tag(1). The kind tag
        // sits 34 bytes from the end (33 trailing bytes after it, plus itself).
        let tag_idx = bytes.len() - 34;
        let mut corrupted = bytes.clone();
        corrupted[tag_idx] = 0xff;
        assert_eq!(
            decode_entry(&corrupted).unwrap_err(),
            DecodeError::UnknownResolvedKind(0xff)
        );
    }

    #[test]
    fn decode_rejects_run_versions_unknown_idempotency_class() {
        // v6 (M2.3b): the idempotency_class tag is the LAST body byte.
        let e = sample_run_versions(Some(sample_capability()));
        let bytes = encode_entry(&e).unwrap();
        let class_idx = bytes.len() - 1;
        let mut corrupted = bytes.clone();
        corrupted[class_idx] = 0xff;
        assert_eq!(
            decode_entry(&corrupted).unwrap_err(),
            DecodeError::UnknownIdempotencyClass(0xff)
        );
    }

    #[test]
    fn decode_rejects_v5_shaped_run_versions_body_missing_class_byte() {
        // A v5-shaped capability body (no trailing class byte) is a body-too-short
        // under v6 — the decoder reads through def_hash then runs out of bytes
        // reading the class tag. Fail-closed; backstopped by the schema-version gate.
        let e = sample_run_versions(Some(sample_capability()));
        let mut bytes = encode_entry(&e).unwrap();
        bytes.pop(); // drop the trailing idempotency_class tag → v5 shape
        assert!(matches!(
            decode_entry(&bytes),
            Err(DecodeError::BodyTooShort {
                kind: KIND_RUN_VERSIONS_RESOLVED,
                ..
            })
        ));
    }

    #[test]
    fn idempotency_class_tag_round_trips_and_rejects_unknown() {
        for tag in [
            IdempotencyClassTag::Token,
            IdempotencyClassTag::Readback,
            IdempotencyClassTag::Staged,
            IdempotencyClassTag::AtLeastOnce,
        ] {
            assert_eq!(IdempotencyClassTag::from_u8(tag.as_u8()), Some(tag));
        }
        assert_eq!(IdempotencyClassTag::from_u8(4), None);
    }

    #[test]
    fn recovery_failure_reasons_round_trip_and_are_terminal() {
        for r in [
            FailureReason::CompensatedAtLeastOnce,
            FailureReason::QuarantinedAtLeastOnce,
        ] {
            assert_eq!(FailureReason::from_u8(r.as_u8()), Some(r));
            // Both recovery outcomes are TERMINAL — never re-dispatched.
            assert!(!is_pre_commit_crash(r));
        }
        assert_eq!(FailureReason::CompensatedAtLeastOnce.as_u8(), 6);
        assert_eq!(FailureReason::QuarantinedAtLeastOnce.as_u8(), 7);
        // F4: the engine dead-letter variant is discriminant 8 (terminal), and it
        // round-trips. The unknown-sentinel boundary moves to 9.
        assert_eq!(FailureReason::DeadLettered.as_u8(), 8);
        assert_eq!(FailureReason::from_u8(8), Some(FailureReason::DeadLettered));
        assert!(!is_pre_commit_crash(FailureReason::DeadLettered));
        assert_eq!(FailureReason::from_u8(9), None);
    }

    #[test]
    fn decode_rejects_run_versions_non_boolean_has_cap() {
        // has_cap sits right after instance_id(16) ‖ warrant_ref(32) ‖
        // u16-prefixed model_id, all inside the body (after the 74-byte header).
        let e = sample_run_versions(None);
        let bytes = encode_entry(&e).unwrap();
        let has_cap_idx = bytes.len() - 1; // None body ends at has_cap
        let mut corrupted = bytes.clone();
        corrupted[has_cap_idx] = 2;
        assert_eq!(
            decode_entry(&corrupted).unwrap_err(),
            DecodeError::NonBooleanHasCap(2)
        );
    }

    #[test]
    fn resolved_kind_tag_round_trips_and_rejects_unknown() {
        for tag in [
            ResolvedKindTag::Builtin,
            ResolvedKindTag::LocalScript,
            ResolvedKindTag::External,
            ResolvedKindTag::Mcp,
            ResolvedKindTag::SelfGenerated,
        ] {
            assert_eq!(ResolvedKindTag::from_u8(tag.as_u8()), Some(tag));
        }
        assert_eq!(ResolvedKindTag::from_u8(5), None);
    }

    #[test]
    fn is_pre_commit_crash_classifies_canonically() {
        // Pre-commit-crash class (re-dispatch permitted under EffectStaged).
        assert!(is_pre_commit_crash(FailureReason::TimedOut));
        assert!(is_pre_commit_crash(FailureReason::WorkerCrashed));
        // Terminal class (re-dispatch FORBIDDEN under EffectStaged — cell 5).
        assert!(!is_pre_commit_crash(FailureReason::ExecutorRefused));
        assert!(!is_pre_commit_crash(FailureReason::ValidatorRejected));
        assert!(!is_pre_commit_crash(FailureReason::UpstreamRepudiated));
        assert!(!is_pre_commit_crash(
            FailureReason::UnsafeWorldMutatingConstruction
        ));
        // F4: the engine dead-letter is terminal — the loop gave up; NEVER
        // re-dispatch (writing it under EffectStaged must NOT spin the loop).
        assert!(!is_pre_commit_crash(FailureReason::DeadLettered));
    }

    #[test]
    fn decode_rejects_repudiated_with_header_body_mismatch() {
        // Hand-craft a Repudiated entry whose body's target_mote_id differs from the
        // header's. The encoder always sets them equal; we corrupt by byte twiddling.
        let r = JournalEntry::Repudiated {
            target_mote_id: MoteId::from_bytes([5u8; 32]),
            idempotency_key: [0u8; 32],
            seq: 1,
            target_committed_seq: 0,
            reason_class: RepudiationReason::OperatorAction,
            repudiator_id: 0,
        };
        let mut bytes = encode_entry(&r).unwrap();
        // Body starts at HEADER_LEN. target_mote_id is the first 32 bytes of the body.
        // Flip one byte to break body-header consistency.
        bytes[HEADER_LEN] ^= 0xff;
        assert_eq!(
            decode_entry(&bytes).unwrap_err(),
            DecodeError::RepudiatedHeaderMismatch
        );
    }

    #[test]
    fn decode_rejects_too_large() {
        let mut bytes = vec![0u8; MAX_ENTRY_LEN + 1];
        bytes[0] = KIND_PROPOSED;
        assert!(matches!(
            decode_entry(&bytes),
            Err(DecodeError::TooLarge { .. })
        ));
    }

    #[test]
    fn decode_rejects_unknown_kind() {
        let mut bytes = vec![0u8; HEADER_LEN + 16];
        bytes[0] = 0xff;
        assert!(matches!(
            decode_entry(&bytes),
            Err(DecodeError::UnknownKind(0xff))
        ));
    }

    #[test]
    fn byte_level_determinism_two_encodes_match() {
        let c = sample_committed();
        let a = encode_entry(&c).unwrap();
        let b = encode_entry(&c).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn repudiation_key_is_deterministic_and_dedupes_by_target() {
        let mid = MoteId::from_bytes([0xab; 32]);
        let k1 = repudiation_idempotency_key(&mid, 42);
        let k2 = repudiation_idempotency_key(&mid, 42);
        assert_eq!(k1, k2);

        let k_other = repudiation_idempotency_key(&mid, 43);
        assert_ne!(k1, k_other);
    }

    #[test]
    fn parent_entry_round_trips_through_parent_ref() {
        use kx_mote::{EdgeMeta, ParentRef};
        let pr = ParentRef {
            parent_id: MoteId::from_bytes([0xcd; 32]),
            edge: EdgeMeta {
                kind: EdgeKind::Control,
                non_cascade: true,
            },
        };
        let pe = ParentEntry::from_parent_ref(&pr);
        assert_eq!(pe.edge_kind, 1);
        assert_eq!(pe.non_cascade, 1);
        assert_eq!(pe.to_parent_ref(), Some(pr));
    }

    #[test]
    fn small_vec_inline_discipline_0_to_4() {
        // Up to 4 parents should fit inline (no heap allocation in SmallVec).
        for n in 0..=4 {
            let parents: SmallVec<[ParentEntry; 4]> = (0..n)
                .map(|i| ParentEntry {
                    parent_id: MoteId::from_bytes([i as u8; 32]),
                    edge_kind: 1,
                    non_cascade: 0,
                })
                .collect();
            assert!(!parents.spilled(), "parents of len {n} must stay inline");
        }
    }

    #[test]
    fn small_vec_spills_at_5_plus() {
        let parents: SmallVec<[ParentEntry; 4]> = (0..5u8)
            .map(|i| ParentEntry {
                parent_id: MoteId::from_bytes([i; 32]),
                edge_kind: 1,
                non_cascade: 0,
            })
            .collect();
        assert!(parents.spilled(), "5+ parents must spill to heap");
    }
}
