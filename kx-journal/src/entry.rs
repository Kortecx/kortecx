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
//!     kind             u8     (Proposed=0, Committed=1, Repudiated=2, Failed=3)
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
pub const JOURNAL_SCHEMA_VERSION: u16 = 2;

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
            | Self::EffectStaged { seq, .. } => *seq,
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
        }
    }

    /// The entry's primary `mote_id`. For `Repudiated` entries this is the
    /// `target_mote_id` (matches the header's `mote_id` per `journal-entry.md` §6).
    #[must_use]
    pub fn mote_id(&self) -> MoteId {
        match self {
            Self::Proposed { mote_id, .. }
            | Self::Committed { mote_id, .. }
            | Self::Failed { mote_id, .. }
            | Self::EffectStaged { mote_id, .. } => *mote_id,
            Self::Repudiated { target_mote_id, .. } => *target_mote_id,
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
    /// The kind discriminant byte is not one of the four known values.
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
        other => Err(DecodeError::UnknownKind(other)),
    }
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
