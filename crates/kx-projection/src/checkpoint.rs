//! [`FoldCheckpoint`] — a **discardable** snapshot of the folded projection
//! state (D92(b), M2.2).
//!
//! ## What it is
//!
//! A projection is a deterministic left-fold over the seq-ordered journal, so
//! `fold(0,N] == fold(K,N] ∘ fold(0,K]`. A [`FoldCheckpoint`] stores `fold(0,K]`
//! materialized (a canonical-bincode encoding of the private [`State`]) plus a
//! `journal_offset = K` and an integrity `digest`. Cold recovery can then seed a
//! [`crate::Projection`] from the checkpoint and fold only the tail
//! `(K, current]` instead of re-folding `(0, current]` from scratch
//! ([`crate::Projection::from_journal_with_checkpoint`]).
//!
//! ## Hard contract (do not weaken)
//!
//! - **Never authoritative.** The journal is the only source of truth. A
//!   corrupt / stale / wrong-run checkpoint is **silently discarded** and the
//!   full fold runs — recovery is correct with or without it. Every fallible
//!   path in [`FoldCheckpoint::from_bytes`] returns `Err` (never panics), and
//!   [`crate::Projection::from_journal_with_checkpoint`] treats any anomaly as
//!   "fall back to the full fold".
//! - **Never journaled, never an identity input, never gates.** The checkpoint
//!   digest is a *full-state* integrity digest, **distinct** from the
//!   committed-facts product digest (`kx-runtime`'s `digest_projection`, the
//!   canonical `7d22d4bd…`). It is exact-equality only (SN-8); no fuzzy match.
//! - **Self-healing format.** [`CURRENT_FORMAT_VERSION`] + [`PAYLOAD_CODEC`] are
//!   checked on decode; a future format / codec (e.g. an rkyv zero-copy payload)
//!   bumps these and old checkpoints cleanly invalidate → full fold.
//!
//! ## Integrity → unforgeability (M2.2c, D103.2)
//!
//! The envelope `digest` alone proves **integrity against accidental corruption**
//! (bit-rot, torn write, truncation), not unforgeability — a writer of the
//! sidecar could craft a self-consistent blob seeding a *wrong* base state (the
//! D103.1 residual). **M2.2c closes that gap:** the runtime co-commits a
//! `DigestSealed{through_seq, state_digest}` entry _in the journal_ (the trust
//! root) at each checkpoint frontier, and
//! [`crate::Projection::from_journal_with_checkpoint`] trusts a seeded base only
//! if its reconstructed [`crate::Projection::state_digest`] matches that journaled
//! seal (gate 6 in `try_seed_state`; a missing/mismatched seal →
//! [`FullFoldReason::SealMissing`] / [`FullFoldReason::SealMismatch`] → full
//! fold). Forging a sidecar to seed a wrong state now requires forging the seal,
//! which requires forging the journal — so the read path is **unforgeable** under
//! the single-node trust model. (Distributed/untrusted-storage journals need
//! *signed* seals — the deferred coordinator-parity follow-on.)

use std::collections::{BTreeMap, BTreeSet};

use kx_content::ContentRef;
use kx_journal::{FailureReason, ParentEntry, ResolvedCapabilityRecord, INSTANCE_ID_LEN};
use kx_mote::{EdgeMeta, EffectPattern, MoteDefHash, MoteId, NdClass, ParentRef};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::state::{
    CommittedInfo, DeclaredInfo, MoteInfo, ReactRoundRecord, ReplanRoundRecord, RunRegistration,
    RunResolvedVersions, State,
};

/// The on-disk format version. Bump on **any** change to the envelope layout or
/// the `CheckpointState` payload encoding — old checkpoints then fail
/// [`FoldCheckpoint::from_bytes`] and recovery falls back to a full fold (self-healing).
///
/// `2` (PR-3, AL2): `MoteInfoDto` gained `failure_reason` so a checkpoint-seeded
/// recovery preserves the terminal `FailureReason` a model-driven re-plan reads.
///
/// `3` (PR-2c-2, re-plan-live): `CheckpointState` gained `replan_rounds` so a
/// checkpoint-seeded recovery preserves the durable re-plan-round records the live
/// coordinator re-derives its in-flight replan chain from. A grown payload shifts
/// the bincoded bytes AND the sealed `state_digest()` of every state, so the bump
/// is mandatory; a stale v2 sidecar is rejected and recovery full-folds (self-healing).
///
/// `4` (PR-2d-1, react-substrate): `CheckpointState` gained `react_rounds` so a
/// checkpoint-seeded recovery preserves the durable ReAct-turn records (anchor,
/// settled branches, budget caps) the live coordinator re-derives its in-flight
/// react chain + spent budget from. Same deliberate-break contract as v3: the
/// payload AND `state_digest()` move for every state, a stale v3 sidecar is
/// rejected, recovery full-folds and re-seals (self-healing). Only the PRODUCT
/// run-identity digest is invariant.
///
/// `5` (PR-9b-2b, deterministic-agentic step): `ReactRoundRecordDto` gained
/// `step_salt` (the per-step salt disjoining an agentic step's private chain) so a
/// checkpoint-seeded recovery preserves which chain each turn belongs to. Same
/// deliberate-break contract: the per-record payload grows by the `Option` tag,
/// shifting the bincoded bytes + `state_digest()` of any state THAT HAS react
/// records; a state with NO react rounds (the demo / a non-react run) encodes a
/// length-0 `react_rounds` Vec either way, so its `encode_state` /
/// `state_content_digest` are BYTE-UNCHANGED and the canonical PRODUCT digest
/// `7d22d4bd` is invariant. A stale v4 sidecar is rejected; recovery full-folds
/// from the v9 journal (which carries `step_salt`) and re-seals (self-healing).
///
/// `6` (PR-R1, per-invocation run identity): `ReactRoundRecordDto` gained
/// `is_agentic_launch` — the run-level/agentic-launch discriminator a salted
/// run-level chain needs (its `step_salt` is now `Some`). SAME deliberate-break
/// contract as v5: the per-record payload grows by the bool, shifting the bincoded
/// bytes + `state_digest()` of any state THAT HAS react records; a state with NO
/// react rounds (the demo / a non-react run) encodes a length-0 `react_rounds` Vec
/// either way, so its `encode_state` / `state_content_digest` are BYTE-UNCHANGED and
/// the canonical PRODUCT digest `7d22d4bd` is invariant. A stale v5 sidecar is
/// rejected; recovery full-folds from the journal (a byte-absent v10 `ReactRound`
/// up-converts `is_agentic_launch` to `step_salt.is_some()`) and re-seals.
///
/// `7` (PR-9d, per-turn upstream context-carry): `ReactRoundRecordDto` gained
/// `context_items_ref` (the run's encoded context-items bundle ref, recorded on the
/// turn-0 anchor). SAME deliberate-break contract: the per-record payload grows by an
/// `Option<ContentRef>` tag, shifting the bincoded bytes + `state_digest()` of any
/// state THAT HAS react records; a state with NO react rounds (the demo / a non-react
/// run) encodes a length-0 `react_rounds` Vec either way, so its `encode_state` /
/// `state_content_digest` are BYTE-UNCHANGED and the canonical PRODUCT digest
/// `7d22d4bd` is invariant. A stale v6 sidecar is rejected; recovery full-folds from
/// the v12 journal (a byte-absent v11 `ReactRound` up-converts `context_items_ref` to
/// `None`) and re-seals.
pub const CURRENT_FORMAT_VERSION: u16 = 7;

/// Payload codec tag. `0` = canonical-bincode (LE + fixed-int, the house
/// [`kx_mote::canonical_config`]). Reserved for a future rkyv zero-copy payload
/// (`1`, roadmap M2.2d) — an **additive** bump, never a breaking change.
pub const PAYLOAD_CODEC: u8 = 0;

/// Envelope header length: `version(2) ‖ codec(1) ‖ journal_offset(8) ‖ digest(32)`.
const HEADER_LEN: usize = 2 + 1 + 8 + 32;

// ---------------------------------------------------------------------------
// FoldCheckpoint — the public, durable artifact
// ---------------------------------------------------------------------------

/// A discardable, byte-serializable snapshot of the folded projection state at a
/// journal offset. See the module-level docs for the full contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldCheckpoint {
    format_version: u16,
    payload_codec: u8,
    journal_offset: u64,
    digest: [u8; 32],
    payload: Vec<u8>,
}

impl FoldCheckpoint {
    /// The journal `seq` this checkpoint folds **through** (inclusive). Recovery
    /// seeds from here and folds `(journal_offset, current]`.
    #[inline]
    #[must_use]
    pub fn journal_offset(&self) -> u64 {
        self.journal_offset
    }

    /// The full-state integrity digest (blake3 over the envelope header +
    /// payload). Exact-equality only.
    #[inline]
    #[must_use]
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }

    /// The **state-content digest** of this checkpoint — `blake3(payload)`.
    ///
    /// By construction the payload is the canonical `encode_state(state)` (see
    /// [`Self::from_state`]), so this equals [`state_content_digest`] of the
    /// decoded state AND equals the value the runtime journals in a
    /// `DigestSealed` seal at this frontier (the runtime seals
    /// `projection.state_digest()` == `blake3(encode_state(state))`). The M2.2c
    /// recovery gate compares this against the journaled seal **without
    /// re-encoding** the decoded state — and it is *stricter*: a non-canonical
    /// payload (one that decodes to a state but is not its canonical encoding)
    /// fails to match, so only a byte-exact canonical seed is ever trusted.
    #[inline]
    #[must_use]
    pub(crate) fn payload_state_digest(&self) -> [u8; 32] {
        *blake3::hash(&self.payload).as_bytes()
    }

    /// The payload codec tag ([`PAYLOAD_CODEC`] today).
    #[inline]
    #[must_use]
    pub fn payload_codec(&self) -> u8 {
        self.payload_codec
    }

    /// Build a checkpoint from a folded [`State`] (crate-internal; the public
    /// entry point is [`crate::Projection::fold_checkpoint`]).
    pub(crate) fn from_state(state: &State) -> Self {
        let payload = encode_state(state);
        let journal_offset = state.last_seq;
        let digest = envelope_digest(
            CURRENT_FORMAT_VERSION,
            PAYLOAD_CODEC,
            journal_offset,
            &payload,
        );
        Self {
            format_version: CURRENT_FORMAT_VERSION,
            payload_codec: PAYLOAD_CODEC,
            journal_offset,
            digest,
            payload,
        }
    }

    /// Recompute the integrity digest over this checkpoint's envelope + payload
    /// and compare it to the stored digest.
    ///
    /// Proves **internal integrity** (the bytes were not corrupted), not journal
    /// provenance — see the module-level docs. Cheap; no payload decode.
    #[must_use]
    pub fn verify(&self) -> bool {
        self.format_version == CURRENT_FORMAT_VERSION
            && self.payload_codec == PAYLOAD_CODEC
            && envelope_digest(
                self.format_version,
                self.payload_codec,
                self.journal_offset,
                &self.payload,
            ) == self.digest
    }

    /// Serialize to a durable byte blob: `version_le ‖ codec ‖ offset_le ‖ digest ‖ payload`.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + self.payload.len());
        out.extend_from_slice(&self.format_version.to_le_bytes());
        out.push(self.payload_codec);
        out.extend_from_slice(&self.journal_offset.to_le_bytes());
        out.extend_from_slice(&self.digest);
        out.extend_from_slice(&self.payload);
        out
    }

    /// Parse a durable byte blob. **Panic-free and fully validating**: every
    /// failure (short buffer, unknown version/codec, digest mismatch) is an
    /// [`CheckpointError`], so a malformed/truncated/hostile blob can only ever
    /// be discarded, never trusted.
    ///
    /// # Errors
    /// [`CheckpointError`] for any envelope or integrity failure.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CheckpointError> {
        // Length first — every field is read via `.get(..)` / `try_into`, never
        // by panicking index-slice, so a short buffer is an Err not a panic.
        if bytes.len() < HEADER_LEN {
            return Err(CheckpointError::TooShort {
                got: bytes.len(),
                need: HEADER_LEN,
            });
        }
        let version = u16::from_le_bytes(bytes.get(0..2).and_then(|s| s.try_into().ok()).ok_or(
            CheckpointError::TooShort {
                got: bytes.len(),
                need: HEADER_LEN,
            },
        )?);
        if version != CURRENT_FORMAT_VERSION {
            return Err(CheckpointError::UnsupportedVersion { got: version });
        }
        let codec = *bytes.get(2).ok_or(CheckpointError::TooShort {
            got: bytes.len(),
            need: HEADER_LEN,
        })?;
        if codec != PAYLOAD_CODEC {
            return Err(CheckpointError::UnsupportedCodec { got: codec });
        }
        let journal_offset =
            u64::from_le_bytes(bytes.get(3..11).and_then(|s| s.try_into().ok()).ok_or(
                CheckpointError::TooShort {
                    got: bytes.len(),
                    need: HEADER_LEN,
                },
            )?);
        let digest: [u8; 32] =
            bytes
                .get(11..43)
                .and_then(|s| s.try_into().ok())
                .ok_or(CheckpointError::TooShort {
                    got: bytes.len(),
                    need: HEADER_LEN,
                })?;
        let payload = bytes
            .get(HEADER_LEN..)
            .ok_or(CheckpointError::TooShort {
                got: bytes.len(),
                need: HEADER_LEN,
            })?
            .to_vec();

        if envelope_digest(version, codec, journal_offset, &payload) != digest {
            return Err(CheckpointError::DigestMismatch);
        }
        Ok(Self {
            format_version: version,
            payload_codec: codec,
            journal_offset,
            digest,
            payload,
        })
    }

    /// Decode the payload into a folded [`State`]. Re-validates structural
    /// invariants the encoder cannot (each parent edge via
    /// [`ParentEntry::to_parent_ref`]). Lazy — called only on resume after the
    /// envelope + digest checks pass.
    ///
    /// # Errors
    /// [`CheckpointError::Decode`] on a bincode failure or trailing bytes;
    /// [`CheckpointError::MalformedParent`] on an invalid parent edge.
    pub(crate) fn decode_state(&self) -> Result<State, CheckpointError> {
        let (dto, consumed): (CheckpointState, usize) =
            bincode::serde::decode_from_slice(&self.payload, kx_mote::canonical_config())
                .map_err(|e| CheckpointError::Decode(e.to_string()))?;
        if consumed != self.payload.len() {
            return Err(CheckpointError::Decode(format!(
                "trailing bytes: consumed {consumed} of {}",
                self.payload.len()
            )));
        }
        State::try_from(dto)
    }
}

/// Errors from decoding / validating a [`FoldCheckpoint`]. Every variant is a
/// "discard the checkpoint and full-fold" signal — never fatal to recovery.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CheckpointError {
    /// The blob is shorter than the fixed envelope header.
    #[error("checkpoint too short: got {got} bytes, need at least {need}")]
    TooShort {
        /// Bytes available.
        got: usize,
        /// Bytes required for the header.
        need: usize,
    },
    /// The blob's format version is not [`CURRENT_FORMAT_VERSION`].
    #[error("unsupported checkpoint format version {got} (current is {CURRENT_FORMAT_VERSION})")]
    UnsupportedVersion {
        /// The version read from the blob.
        got: u16,
    },
    /// The blob's payload codec is not [`PAYLOAD_CODEC`].
    #[error("unsupported checkpoint payload codec {got} (current is {PAYLOAD_CODEC})")]
    UnsupportedCodec {
        /// The codec tag read from the blob.
        got: u8,
    },
    /// The recomputed integrity digest does not match the stored digest.
    #[error("checkpoint digest mismatch (corrupt or tampered)")]
    DigestMismatch,
    /// The payload failed bincode decode or carried trailing bytes.
    #[error("checkpoint payload decode failed: {0}")]
    Decode(String),
    /// A decoded parent edge violates the [`ParentEntry`] invariant
    /// (unknown `edge_kind`, or a Data edge with `non_cascade` set).
    #[error("checkpoint payload has a malformed parent edge (kind={edge_kind}, non_cascade={non_cascade})")]
    MalformedParent {
        /// The offending `edge_kind` byte.
        edge_kind: u8,
        /// The offending `non_cascade` byte.
        non_cascade: u8,
    },
}

// ---------------------------------------------------------------------------
// CheckpointOutcome — the structured, testable result of a checkpoint recovery
// ---------------------------------------------------------------------------

/// The outcome of a checkpoint-aware cold recovery — a structured record of
/// whether the discardable [`FoldCheckpoint`] seeded the fold or was discarded
/// (and which gate rejected it). Returned by
/// [`crate::Projection::from_journal_with_checkpoint_reported`] so the live
/// runtime can emit recovery observability and tests can assert on the reason
/// (rather than parsing log lines).
///
/// **Purely diagnostic.** The outcome never affects the folded state, which is
/// bit-identical to a full fold either way — it only reports *how much* of the
/// log was re-folded, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointOutcome {
    /// The checkpoint was usable: the fold was seeded at `offset` and only the
    /// tail `(offset, head]` — `tail_entries` entries — was re-folded.
    Seeded {
        /// The journal offset the checkpoint folded through (inclusive).
        offset: u64,
        /// The number of journal entries re-folded on top of the seed.
        tail_entries: u64,
    },
    /// No usable checkpoint — the full log `(0, head]` was folded. `reason`
    /// records why (none supplied, or which validation gate rejected it).
    FullFold {
        /// Why the full fold ran.
        reason: FullFoldReason,
    },
}

/// Why a checkpoint-aware recovery fell back to a full fold. Every non-`NoCheckpoint`
/// variant corresponds to exactly one validation gate in the seed path; **all are
/// safe** — recovery is bit-identical to a full fold regardless of which fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FullFoldReason {
    /// No checkpoint was supplied (`None`) — an ordinary cold start.
    NoCheckpoint,
    /// The envelope integrity digest did not verify (corrupt / truncated / tampered).
    IntegrityFailed,
    /// The checkpoint offset runs past the journal head (stale / truncated log).
    OffsetAheadOfHead,
    /// The payload failed to decode (a malformed / hostile blob).
    DecodeFailed,
    /// The decoded `last_seq` disagrees with the declared offset (inconsistent).
    OffsetMismatch,
    /// The checkpoint's run instance-id does not match the journal's (wrong run).
    WrongRun,
    /// **M2.2c (D103.2).** No journaled `DigestSealed{through_seq == offset}` seal
    /// was found to anchor the seeded state — the seed is un-anchored, so it is
    /// discarded (a sidecar without a co-committed seal, e.g. the M2.2b world, or
    /// a crash between the sidecar write and the seal append).
    SealMissing,
    /// **M2.2c (D103.2).** A journaled seal exists at the seed offset but its
    /// `state_digest` does not match the seeded state's digest — the seed is
    /// **forged or corrupt** (the D103.1 attack: a self-consistent sidecar seeding
    /// a wrong base state). Discarded; recovery full-folds the trust root.
    SealMismatch,
}

// ---------------------------------------------------------------------------
// Canonical encoding helpers (shared by the checkpoint + the state digest)
// ---------------------------------------------------------------------------

/// Canonical-bincode encoding of the full folded state (the checkpoint payload).
/// Infallible for [`CheckpointState`] (no custom `Serialize`); the `expect`
/// mirrors the house pattern (`kx_mote::def`, `kx_critic_types::verdict`).
// SAFETY (expect_used): `CheckpointState` has no `f32`/`f64`, no custom
// `Serialize`, and no non-encodable variant; bincode encode over it cannot fail
// (the only `EncodeError`s are I/O/size, neither reachable for an in-memory Vec
// sink). Documented-infallible production use per the workspace lint policy.
#[allow(clippy::expect_used)]
pub(crate) fn encode_state(state: &State) -> Vec<u8> {
    bincode::serde::encode_to_vec(CheckpointState::from(state), kx_mote::canonical_config())
        .expect("canonical bincode encode of CheckpointState is infallible")
}

/// The canonical **full-state** digest: `blake3(encode_state(state))`. A pure
/// function of the folded state content (includes `last_seq` + run registration,
/// so it is self-anchoring). Reused by [`crate::Projection::state_digest`] and by
/// the journaled digest seal (M2.2c) — the runtime seals this value at each
/// checkpoint frontier and recovery anchors the seeded state against it.
/// Distinct from the committed-facts product digest in `kx-runtime`.
pub(crate) fn state_content_digest(state: &State) -> [u8; 32] {
    *blake3::hash(&encode_state(state)).as_bytes()
}

/// The checkpoint envelope integrity digest:
/// `blake3(version_le ‖ codec ‖ offset_le ‖ payload)`. Binds the format header
/// to the payload so a header edit (e.g. a moved offset) is caught too.
fn envelope_digest(version: u16, codec: u8, offset: u64, payload: &[u8]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(&version.to_le_bytes());
    h.update(&[codec]);
    h.update(&offset.to_le_bytes());
    h.update(payload);
    *h.finalize().as_bytes()
}

// ---------------------------------------------------------------------------
// CheckpointState — the crate-private serde DTO mirroring `State`
// ---------------------------------------------------------------------------
//
// DTOs mirror the private `State` graph using only serde-friendly shapes (the
// one non-serde type, `ParentEntry`, becomes a `(MoteId,u8,u8)` triple). The
// `From`/`TryFrom` bodies DESTRUCTURE every source/target struct WITHOUT `..`,
// so adding a field to `State`/`MoteInfo`/`CommittedInfo`/`DeclaredInfo`/
// `RunRegistration`/`RunResolvedVersions` fails to COMPILE until the DTO is
// updated — "losslessly mirrors State" is enforced by the compiler, not review.

#[derive(Serialize, Deserialize)]
struct CheckpointState {
    motes: BTreeMap<MoteId, MoteInfoDto>,
    children: BTreeMap<MoteId, Vec<(MoteId, EdgeMeta)>>,
    last_seq: u64,
    run_registration: Option<RunRegistrationDto>,
    run_resolved_versions: Vec<RunResolvedVersionsDto>,
    replan_rounds: Vec<ReplanRoundRecordDto>,
    react_rounds: Vec<ReactRoundRecordDto>,
}

// Mirrors `MoteInfo`'s flags 1:1 — same `struct_excessive_bools` allow.
#[allow(clippy::struct_excessive_bools)]
#[derive(Serialize, Deserialize)]
struct MoteInfoDto {
    declared: Option<DeclaredInfoDto>,
    committed: Option<CommittedInfoDto>,
    has_proposed: bool,
    failed_pending_reattempt: bool,
    effect_staged_observed: bool,
    terminal_failure_observed: bool,
    inconsistent: bool,
    quarantined: bool,
    failure_reason: Option<FailureReason>,
}

#[derive(Serialize, Deserialize)]
struct DeclaredInfoDto {
    nd_class: NdClass,
    effect_pattern: EffectPattern,
    critic_for: Option<MoteId>,
    is_topology_shaper: bool,
    parents: Vec<ParentRef>,
    warrant_ref: ContentRef,
}

#[derive(Serialize, Deserialize)]
struct CommittedInfoDto {
    seq: u64,
    result_ref: ContentRef,
    nondeterminism: NdClass,
    /// `ParentEntry` is not serde-derived (it controls its on-disk journal
    /// width); mirror it as `(parent_id, edge_kind, non_cascade)`.
    parents_in_entry: Vec<(MoteId, u8, u8)>,
    warrant_ref: ContentRef,
    mote_def_hash: MoteDefHash,
    repudiated: bool,
}

#[derive(Serialize, Deserialize)]
struct RunRegistrationDto {
    instance_id: [u8; INSTANCE_ID_LEN],
    recipe_fingerprint: [u8; 32],
}

#[derive(Serialize, Deserialize)]
struct RunResolvedVersionsDto {
    instance_id: [u8; INSTANCE_ID_LEN],
    warrant_ref: ContentRef,
    model_id: String,
    capability: Option<ResolvedCapabilityRecord>,
}

#[derive(Serialize, Deserialize)]
struct ReplanRoundRecordDto {
    round: u32,
    shaper_mote_id: MoteId,
    base_prompt_ref: ContentRef,
    corrected_prompt_ref: ContentRef,
    warrant_ref: ContentRef,
    model_id: String,
    failed_steps: Vec<MoteId>,
    escalation_reason_ref: Option<ContentRef>,
    seq: u64,
}

#[derive(Serialize, Deserialize)]
struct ReactRoundRecordDto {
    turn: u32,
    turn_mote_id: MoteId,
    instance_id: [u8; INSTANCE_ID_LEN],
    base_prompt_ref: ContentRef,
    warrant_ref: ContentRef,
    model_id: String,
    /// `kx_journal::ReactBranch` is serde-derived for exactly this DTO (the
    /// `ResolvedCapabilityRecord` precedent); the journal's canonical on-disk
    /// encoding stays the hand-rolled tag.
    branch: kx_journal::ReactBranch,
    max_turns: u32,
    max_tool_calls: u32,
    /// PR-9b-2 — the per-step salt (`None` ⇒ run-level). The format-version bump
    /// (v4→v5) covers this added field; absent in v4 sidecars (rejected → full-fold).
    step_salt: Option<[u8; 32]>,
    /// PR-R1 — the run-level/agentic-launch discriminator (the format-version bump
    /// v5→v6 covers it; absent in v5 sidecars → rejected → full-fold from the journal,
    /// which up-converts a byte-absent v10 `ReactRound` to `step_salt.is_some()`).
    is_agentic_launch: bool,
    /// PR-9d — the run's encoded context-items bundle ref (`None` ⇒ no attached/
    /// retrieved context). The format-version bump (v6→v7) covers this added field;
    /// absent in v6 sidecars (rejected → full-fold from the v12 journal).
    context_items_ref: Option<ContentRef>,
    seq: u64,
}

// ----- State -> DTO (infallible; destructure-without-`..` drift guard) -----

impl From<&State> for CheckpointState {
    fn from(state: &State) -> Self {
        // Destructure WITHOUT `..` — a new `State` field breaks this build.
        // `react_index` / `react_turn_motes` / `react_tool_round_of_turn` are
        // DERIVED views over `react_rounds` (PR-2d-2 / T-MULTI-ELEMENT-TOOLCALLS):
        // deliberately NOT serialized — the load path re-derives them, so the
        // checkpoint format (v4), `encode_state`, and the `state_content_digest`
        // are byte-unchanged by their existence.
        let State {
            motes,
            children,
            last_seq,
            run_registration,
            run_resolved_versions,
            replan_rounds,
            react_rounds,
            react_index: _,
            react_turn_motes: _,
            react_tool_round_of_turn: _,
        } = state;
        Self {
            motes: motes
                .iter()
                .map(|(id, mi)| (*id, MoteInfoDto::from(mi)))
                .collect(),
            children: children.clone(),
            last_seq: *last_seq,
            run_registration: run_registration.as_ref().map(RunRegistrationDto::from),
            run_resolved_versions: run_resolved_versions
                .iter()
                .map(RunResolvedVersionsDto::from)
                .collect(),
            replan_rounds: replan_rounds
                .iter()
                .map(ReplanRoundRecordDto::from)
                .collect(),
            react_rounds: react_rounds.iter().map(ReactRoundRecordDto::from).collect(),
        }
    }
}

impl From<&MoteInfo> for MoteInfoDto {
    fn from(mi: &MoteInfo) -> Self {
        let MoteInfo {
            declared,
            committed,
            has_proposed,
            failed_pending_reattempt,
            effect_staged_observed,
            terminal_failure_observed,
            inconsistent,
            quarantined,
            failure_reason,
        } = mi;
        Self {
            declared: declared.as_ref().map(DeclaredInfoDto::from),
            committed: committed.as_ref().map(CommittedInfoDto::from),
            has_proposed: *has_proposed,
            failed_pending_reattempt: *failed_pending_reattempt,
            effect_staged_observed: *effect_staged_observed,
            terminal_failure_observed: *terminal_failure_observed,
            inconsistent: *inconsistent,
            quarantined: *quarantined,
            failure_reason: *failure_reason,
        }
    }
}

impl From<&DeclaredInfo> for DeclaredInfoDto {
    fn from(d: &DeclaredInfo) -> Self {
        let DeclaredInfo {
            nd_class,
            effect_pattern,
            critic_for,
            is_topology_shaper,
            parents,
            warrant_ref,
        } = d;
        Self {
            nd_class: *nd_class,
            effect_pattern: *effect_pattern,
            critic_for: *critic_for,
            is_topology_shaper: *is_topology_shaper,
            parents: parents.to_vec(),
            warrant_ref: *warrant_ref,
        }
    }
}

impl From<&CommittedInfo> for CommittedInfoDto {
    fn from(c: &CommittedInfo) -> Self {
        let CommittedInfo {
            seq,
            result_ref,
            nondeterminism,
            parents_in_entry,
            warrant_ref,
            mote_def_hash,
            repudiated,
        } = c;
        Self {
            seq: *seq,
            result_ref: *result_ref,
            nondeterminism: *nondeterminism,
            parents_in_entry: parents_in_entry
                .iter()
                .map(|p| (p.parent_id, p.edge_kind, p.non_cascade))
                .collect(),
            warrant_ref: *warrant_ref,
            mote_def_hash: *mote_def_hash,
            repudiated: *repudiated,
        }
    }
}

impl From<&RunRegistration> for RunRegistrationDto {
    fn from(r: &RunRegistration) -> Self {
        let RunRegistration {
            instance_id,
            recipe_fingerprint,
        } = r;
        Self {
            instance_id: *instance_id,
            recipe_fingerprint: *recipe_fingerprint,
        }
    }
}

impl From<&RunResolvedVersions> for RunResolvedVersionsDto {
    fn from(r: &RunResolvedVersions) -> Self {
        let RunResolvedVersions {
            instance_id,
            warrant_ref,
            model_id,
            capability,
        } = r;
        Self {
            instance_id: *instance_id,
            warrant_ref: *warrant_ref,
            model_id: model_id.clone(),
            capability: capability.clone(),
        }
    }
}

impl From<&ReplanRoundRecord> for ReplanRoundRecordDto {
    fn from(r: &ReplanRoundRecord) -> Self {
        let ReplanRoundRecord {
            round,
            shaper_mote_id,
            base_prompt_ref,
            corrected_prompt_ref,
            warrant_ref,
            model_id,
            failed_steps,
            escalation_reason_ref,
            seq,
        } = r;
        Self {
            round: *round,
            shaper_mote_id: *shaper_mote_id,
            base_prompt_ref: *base_prompt_ref,
            corrected_prompt_ref: *corrected_prompt_ref,
            warrant_ref: *warrant_ref,
            model_id: model_id.clone(),
            failed_steps: failed_steps.clone(),
            escalation_reason_ref: *escalation_reason_ref,
            seq: *seq,
        }
    }
}

impl From<&ReactRoundRecord> for ReactRoundRecordDto {
    fn from(r: &ReactRoundRecord) -> Self {
        let ReactRoundRecord {
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
        } = r;
        Self {
            turn: *turn,
            turn_mote_id: *turn_mote_id,
            instance_id: *instance_id,
            base_prompt_ref: *base_prompt_ref,
            warrant_ref: *warrant_ref,
            model_id: model_id.clone(),
            branch: branch.clone(),
            max_turns: *max_turns,
            max_tool_calls: *max_tool_calls,
            step_salt: *step_salt,
            is_agentic_launch: *is_agentic_launch,
            context_items_ref: *context_items_ref,
            seq: *seq,
        }
    }
}

// ----- DTO -> State (fallible: revalidates each parent edge) -----

impl TryFrom<CheckpointState> for State {
    type Error = CheckpointError;

    fn try_from(dto: CheckpointState) -> Result<Self, Self::Error> {
        // Destructure WITHOUT `..` — a new `CheckpointState` field breaks this build.
        let CheckpointState {
            motes,
            children,
            last_seq,
            run_registration,
            run_resolved_versions,
            replan_rounds,
            react_rounds,
        } = dto;
        let mut decoded_motes = BTreeMap::new();
        for (id, mi) in motes {
            decoded_motes.insert(id, MoteInfo::try_from(mi)?);
        }
        let mut state = State {
            motes: decoded_motes,
            children,
            last_seq,
            run_registration: run_registration.map(RunRegistration::from),
            run_resolved_versions: run_resolved_versions
                .into_iter()
                .map(RunResolvedVersions::from)
                .collect(),
            replan_rounds: replan_rounds
                .into_iter()
                .map(ReplanRoundRecord::from)
                .collect(),
            react_rounds: react_rounds
                .into_iter()
                .map(ReactRoundRecord::from)
                .collect(),
            react_index: BTreeMap::new(),
            react_turn_motes: BTreeSet::new(),
            react_tool_round_of_turn: BTreeMap::new(),
        };
        // PR-2d-2: RE-DERIVE the react index/turn-set from the deserialized
        // facts — the same shape the fold maintains incrementally, so both
        // construction paths produce identical derived state and
        // `State: PartialEq` holds between a folded and a checkpoint-loaded
        // projection.
        derive_react_index(&mut state);
        Ok(state)
    }
}

/// Re-derive the PR-2d-2 react index + turn-set (+ the T-MULTI-ELEMENT-TOOLCALLS
/// turn→tool-round map) over an already-populated `react_rounds` (the
/// checkpoint-load path; the fold path maintains them incrementally via
/// `State::index_last_react_round` — keep the two in lock-step).
fn derive_react_index(state: &mut State) {
    state.react_index.clear();
    state.react_turn_motes.clear();
    state.react_tool_round_of_turn.clear();
    for (idx, record) in state.react_rounds.iter().enumerate() {
        state
            .react_index
            .entry(record.instance_id)
            .or_default()
            .entry(record.step_salt)
            .or_default()
            .push(idx);
        state.react_turn_motes.insert(record.turn_mote_id);
        if matches!(
            record.branch,
            kx_journal::ReactBranch::Tool { .. } | kx_journal::ReactBranch::ToolBatch { .. }
        ) {
            state
                .react_tool_round_of_turn
                .insert(record.turn_mote_id, idx);
        }
    }
}

impl TryFrom<MoteInfoDto> for MoteInfo {
    type Error = CheckpointError;

    fn try_from(dto: MoteInfoDto) -> Result<Self, Self::Error> {
        let MoteInfoDto {
            declared,
            committed,
            has_proposed,
            failed_pending_reattempt,
            effect_staged_observed,
            terminal_failure_observed,
            inconsistent,
            quarantined,
            failure_reason,
        } = dto;
        Ok(MoteInfo {
            declared: declared.map(DeclaredInfo::from),
            committed: committed.map(CommittedInfo::try_from).transpose()?,
            has_proposed,
            failed_pending_reattempt,
            effect_staged_observed,
            terminal_failure_observed,
            inconsistent,
            quarantined,
            failure_reason,
        })
    }
}

impl From<DeclaredInfoDto> for DeclaredInfo {
    fn from(dto: DeclaredInfoDto) -> Self {
        let DeclaredInfoDto {
            nd_class,
            effect_pattern,
            critic_for,
            is_topology_shaper,
            parents,
            warrant_ref,
        } = dto;
        DeclaredInfo {
            nd_class,
            effect_pattern,
            critic_for,
            is_topology_shaper,
            parents: SmallVec::from_vec(parents),
            warrant_ref,
        }
    }
}

impl TryFrom<CommittedInfoDto> for CommittedInfo {
    type Error = CheckpointError;

    fn try_from(dto: CommittedInfoDto) -> Result<Self, Self::Error> {
        let CommittedInfoDto {
            seq,
            result_ref,
            nondeterminism,
            parents_in_entry,
            warrant_ref,
            mote_def_hash,
            repudiated,
        } = dto;
        let mut parents: SmallVec<[ParentEntry; 4]> = SmallVec::new();
        for (parent_id, edge_kind, non_cascade) in parents_in_entry {
            let pe = ParentEntry {
                parent_id,
                edge_kind,
                non_cascade,
            };
            // Re-validate the edge invariant the journal encoder enforces — a
            // hostile/corrupt blob cannot smuggle an illegal edge into the index.
            if pe.to_parent_ref().is_none() {
                return Err(CheckpointError::MalformedParent {
                    edge_kind,
                    non_cascade,
                });
            }
            parents.push(pe);
        }
        Ok(CommittedInfo {
            seq,
            result_ref,
            nondeterminism,
            parents_in_entry: parents,
            warrant_ref,
            mote_def_hash,
            repudiated,
        })
    }
}

impl From<RunRegistrationDto> for RunRegistration {
    fn from(dto: RunRegistrationDto) -> Self {
        let RunRegistrationDto {
            instance_id,
            recipe_fingerprint,
        } = dto;
        RunRegistration {
            instance_id,
            recipe_fingerprint,
        }
    }
}

impl From<RunResolvedVersionsDto> for RunResolvedVersions {
    fn from(dto: RunResolvedVersionsDto) -> Self {
        let RunResolvedVersionsDto {
            instance_id,
            warrant_ref,
            model_id,
            capability,
        } = dto;
        RunResolvedVersions {
            instance_id,
            warrant_ref,
            model_id,
            capability,
        }
    }
}

impl From<ReplanRoundRecordDto> for ReplanRoundRecord {
    fn from(dto: ReplanRoundRecordDto) -> Self {
        let ReplanRoundRecordDto {
            round,
            shaper_mote_id,
            base_prompt_ref,
            corrected_prompt_ref,
            warrant_ref,
            model_id,
            failed_steps,
            escalation_reason_ref,
            seq,
        } = dto;
        ReplanRoundRecord {
            round,
            shaper_mote_id,
            base_prompt_ref,
            corrected_prompt_ref,
            warrant_ref,
            model_id,
            failed_steps,
            escalation_reason_ref,
            seq,
        }
    }
}

impl From<ReactRoundRecordDto> for ReactRoundRecord {
    fn from(dto: ReactRoundRecordDto) -> Self {
        let ReactRoundRecordDto {
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
        } = dto;
        ReactRoundRecord {
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mid(b: u8) -> MoteId {
        MoteId::from_bytes([b; 32])
    }

    /// A folded state exercising **every** DTO arm + **every** `MoteInfo` flag,
    /// so the lossless round-trip is a comprehensive field-coverage oracle:
    /// declared (with `critic_for` + a parent), committed (WORLD-MUTATING, a
    /// control parent, `mote_def_hash`), `effect_staged_observed`, `has_proposed`,
    /// `failed_pending_reattempt`, `terminal_failure_observed`, `inconsistent`,
    /// committed+`repudiated`, run registration, and a resolved-versions record.
    fn sample_state() -> State {
        let mut s = State::default();
        // declared child 10 with parent 1 (data edge) + a critic relationship
        s.set_declared(
            mid(10),
            DeclaredInfo {
                nd_class: NdClass::Pure,
                effect_pattern: EffectPattern::IdempotentByConstruction,
                critic_for: Some(mid(7)),
                is_topology_shaper: false,
                parents: SmallVec::from_vec(vec![ParentRef {
                    parent_id: mid(1),
                    edge: EdgeMeta::data(),
                }]),
                warrant_ref: ContentRef::from_bytes([0xaa; 32]),
            },
        );
        // committed mote 1 with a control parent 2, plus effect_staged_observed
        let info = s.moteinfo_mut(&mid(1));
        info.committed = Some(CommittedInfo {
            seq: 3,
            result_ref: ContentRef::from_bytes([7; 32]),
            nondeterminism: NdClass::WorldMutating,
            parents_in_entry: SmallVec::from_vec(vec![ParentEntry {
                parent_id: mid(2),
                edge_kind: 1,
                non_cascade: 0,
            }]),
            warrant_ref: ContentRef::from_bytes([0xbb; 32]),
            mote_def_hash: MoteDefHash::from_bytes([9; 32]),
            repudiated: false,
        });
        info.effect_staged_observed = true;
        s.index_committed(mid(1), &[]);
        // committed + repudiated
        let r = s.moteinfo_mut(&mid(24));
        r.committed = Some(CommittedInfo {
            seq: 4,
            result_ref: ContentRef::from_bytes([8; 32]),
            nondeterminism: NdClass::Pure,
            parents_in_entry: SmallVec::new(),
            warrant_ref: ContentRef::from_bytes([0xcc; 32]),
            mote_def_hash: MoteDefHash::from_bytes([1; 32]),
            repudiated: true,
        });
        // the remaining per-flag motes
        s.moteinfo_mut(&mid(20)).has_proposed = true;
        s.moteinfo_mut(&mid(21)).failed_pending_reattempt = true;
        {
            // PR-3: a terminal-failure mote carries a retained `failure_reason`, so
            // the round-trip proves the v1→v2 format preserves it (the bump's point).
            // F4: use the canonical engine dead-letter reason `DeadLettered` (the new
            // discriminant 8), so the checkpoint serde is proven to round-trip it.
            let m = s.moteinfo_mut(&mid(22));
            m.terminal_failure_observed = true;
            m.failure_reason = Some(kx_journal::FailureReason::DeadLettered);
        }
        s.moteinfo_mut(&mid(23)).inconsistent = true;
        s.last_seq = 4;
        s.run_registration = Some(RunRegistration {
            instance_id: [5; INSTANCE_ID_LEN],
            recipe_fingerprint: [6; 32],
        });
        s.run_resolved_versions.push(RunResolvedVersions {
            instance_id: [5; INSTANCE_ID_LEN],
            warrant_ref: ContentRef::from_bytes([0xdd; 32]),
            model_id: "qwen-0.5b".to_string(),
            capability: None,
        });
        // PR-2c-2 (v3): a replan-round record, so the round-trip proves the v3
        // payload field is preserved (closes a coverage gap — the v3 bump's point).
        s.replan_rounds.push(ReplanRoundRecord {
            round: 1,
            shaper_mote_id: mid(30),
            base_prompt_ref: ContentRef::from_bytes([0xe1; 32]),
            corrected_prompt_ref: ContentRef::from_bytes([0xe2; 32]),
            warrant_ref: ContentRef::from_bytes([0xe3; 32]),
            model_id: "qwen-0.5b".to_string(),
            failed_steps: vec![mid(31)],
            escalation_reason_ref: None,
            seq: 4,
        });
        push_sample_react_rounds(&mut s);
        // PR-2d-2: a real State's derived react index/turn-set is ALWAYS
        // consistent with `react_rounds` (the fold maintains it; the load path
        // re-derives it) — the fixture must uphold the same invariant or the
        // lossless-roundtrip assert would compare an unindexed source against
        // a re-derived decode.
        derive_react_index(&mut s);
        s
    }

    /// PR-2d-1 (v4) + PR-9b-2b (v5) + PR-R1 (v6): the fixture's react-turn records — a
    /// RUN-LEVEL anchor (`step_salt None`, `is_agentic_launch false`) + an AGENTIC `Tool`
    /// settle (`step_salt Some`, `is_agentic_launch true`) — so the round-trip proves the
    /// v4 payload branch AND the v5 `step_salt` Option AND the v6 launch flag survive.
    fn push_sample_react_rounds(s: &mut State) {
        s.react_rounds.push(ReactRoundRecord {
            turn: 0,
            turn_mote_id: mid(40),
            instance_id: [5; INSTANCE_ID_LEN],
            base_prompt_ref: ContentRef::from_bytes([0xf1; 32]),
            warrant_ref: ContentRef::from_bytes([0xf2; 32]),
            model_id: "qwen-0.5b".to_string(),
            branch: kx_journal::ReactBranch::Pending,
            max_turns: 8,
            max_tool_calls: 8,
            step_salt: None,
            is_agentic_launch: false,
            context_items_ref: None,
            seq: 4,
        });
        s.react_rounds.push(ReactRoundRecord {
            turn: 0,
            turn_mote_id: mid(40),
            instance_id: [5; INSTANCE_ID_LEN],
            base_prompt_ref: ContentRef::from_bytes([0xf1; 32]),
            warrant_ref: ContentRef::from_bytes([0xf2; 32]),
            model_id: "qwen-0.5b".to_string(),
            branch: kx_journal::ReactBranch::Tool {
                tool_id: "mcp-echo".to_string(),
                tool_version: "1".to_string(),
            },
            max_turns: 8,
            max_tool_calls: 8,
            step_salt: Some([0x77; 32]),
            is_agentic_launch: true,
            context_items_ref: Some(ContentRef::from_bytes([0xf3; 32])),
            seq: 4,
        });
    }

    /// PR-9d: pin the checkpoint format version so the v6→v7 bump (the additive
    /// `ReactRoundRecordDto.context_items_ref` field) is an intentional, reviewable
    /// change — and so a v6 sidecar written by the previous binary is REFUSED
    /// (decode error → full-fold self-heal), never misread.
    #[test]
    fn format_version_is_v7_and_v6_blobs_are_refused() {
        assert_eq!(CURRENT_FORMAT_VERSION, 7);
        let mut bytes = FoldCheckpoint::from_state(&sample_state()).to_bytes();
        // Stamp the envelope version back to v6 (bytes 0..2, LE u16).
        bytes[0..2].copy_from_slice(&6u16.to_le_bytes());
        assert!(matches!(
            FoldCheckpoint::from_bytes(&bytes),
            // The version is part of the digest preimage, so a re-stamped v6
            // envelope fails as UnsupportedVersion or DigestMismatch — both are
            // fail-safe discards (full fold).
            Err(CheckpointError::UnsupportedVersion { got: 6 } | CheckpointError::DigestMismatch)
        ));
    }

    #[test]
    fn roundtrip_state_is_lossless() {
        let s = sample_state();
        let cp = FoldCheckpoint::from_state(&s);
        let decoded = cp.decode_state().expect("decode");
        assert_eq!(
            decoded, s,
            "decoded State must equal the source bit-for-bit"
        );
        assert_eq!(cp.journal_offset(), 4);
    }

    #[test]
    fn bytes_roundtrip_and_verify() {
        let cp = FoldCheckpoint::from_state(&sample_state());
        let bytes = cp.to_bytes();
        let back = FoldCheckpoint::from_bytes(&bytes).expect("from_bytes");
        assert_eq!(back, cp);
        assert!(back.verify());
    }

    #[test]
    fn from_bytes_rejects_short_buffers_without_panic() {
        for len in [0usize, 1, HEADER_LEN - 1] {
            let buf = vec![0u8; len];
            assert!(matches!(
                FoldCheckpoint::from_bytes(&buf),
                Err(CheckpointError::TooShort { .. })
            ));
        }
    }

    #[test]
    fn from_bytes_rejects_wrong_version() {
        let mut bytes = FoldCheckpoint::from_state(&sample_state()).to_bytes();
        bytes[0] = bytes[0].wrapping_add(1); // perturb version
        assert!(matches!(
            FoldCheckpoint::from_bytes(&bytes),
            // version is part of the digest preimage, so this is caught as a
            // version OR digest mismatch — both are fail-safe discards.
            Err(CheckpointError::UnsupportedVersion { .. } | CheckpointError::DigestMismatch)
        ));
    }

    #[test]
    fn from_bytes_rejects_unknown_codec() {
        let mut bytes = FoldCheckpoint::from_state(&sample_state()).to_bytes();
        bytes[2] = 9; // unknown codec
        assert!(matches!(
            FoldCheckpoint::from_bytes(&bytes),
            Err(CheckpointError::UnsupportedCodec { got: 9 } | CheckpointError::DigestMismatch)
        ));
    }

    #[test]
    fn from_bytes_detects_flipped_payload_byte() {
        let bytes = FoldCheckpoint::from_state(&sample_state()).to_bytes();
        let mut corrupt = bytes.clone();
        let last = corrupt.len() - 1;
        corrupt[last] ^= 0xff;
        assert_eq!(
            FoldCheckpoint::from_bytes(&corrupt),
            Err(CheckpointError::DigestMismatch)
        );
    }

    #[test]
    fn verify_detects_mutated_digest_field() {
        let mut cp = FoldCheckpoint::from_state(&sample_state());
        cp.digest[0] ^= 0xff;
        assert!(!cp.verify());
    }

    #[test]
    fn decode_rejects_malformed_parent_edge() {
        // Hand-build a state with an illegal Data edge that carries non_cascade=1
        // (the journal encoder would reject this; the checkpoint decoder must too).
        let mut s = State::default();
        let info = s.moteinfo_mut(&mid(1));
        info.committed = Some(CommittedInfo {
            seq: 1,
            result_ref: ContentRef::from_bytes([0; 32]),
            nondeterminism: NdClass::Pure,
            parents_in_entry: SmallVec::from_vec(vec![ParentEntry {
                parent_id: mid(2),
                edge_kind: 0,   // Data
                non_cascade: 1, // illegal on a Data edge
            }]),
            warrant_ref: ContentRef::from_bytes([0; 32]),
            mote_def_hash: MoteDefHash::from_bytes([0; 32]),
            repudiated: false,
        });
        s.last_seq = 1;
        let cp = FoldCheckpoint::from_state(&s);
        assert!(matches!(
            cp.decode_state(),
            Err(CheckpointError::MalformedParent {
                edge_kind: 0,
                non_cascade: 1
            })
        ));
    }

    #[test]
    fn state_digest_is_deterministic_and_content_sensitive() {
        let a = state_content_digest(&sample_state());
        let b = state_content_digest(&sample_state());
        assert_eq!(a, b, "same state -> same digest");
        let mut mutated = sample_state();
        mutated.last_seq = 5; // any content change moves the digest
        assert_ne!(a, state_content_digest(&mutated));
    }
}
