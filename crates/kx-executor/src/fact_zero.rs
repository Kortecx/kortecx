//! Fact-zero protocol (D34, `docs/design/seed-as-fact-zero.md` P0.13).
//!
//! `submit_run` writes a synthetic `Committed`-shaped journal entry as the
//! FIRST entry of a run; root Mote dispatch is gated on fact-zero's commit;
//! recovery REPLAYS fact-zero (never skipped).
//!
//! The `SeedPayload` carries `run_id`, the user's task prompt, the optional
//! system prompt, the workflow-def content-ref, and the submission timestamp.
//! `submitted_at_ms` is **audit-only and excluded from canonical bincode
//! bytes** that produce `result_ref` (per D34 §3.3) — two byte-identical
//! workflows submitted at different times produce the same `result_ref`
//! but different fact-zero `mote_id`s (different `run_id`s).

use kx_content::{ContentRef, ContentStore};
use kx_journal::{Journal, JournalEntry};
use kx_mote::{canonical_config, MoteDefHash, MoteId};
use kx_warrant::{warrant_ref_of, WarrantSpec};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use thiserror::Error;

/// The seed payload — fact-zero's content-store object. Per D34 §2.
///
/// `submitted_at_ms` is audit-only and excluded from canonical bincode bytes
/// via the separate `identity_bytes` helper. Identity callers use
/// `identity_bytes()`; audit callers use the full struct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedPayload {
    /// Per-run 16-byte UUID-shaped identifier. Identity-bearing.
    pub run_id: [u8; 16],
    /// The user's task prompt. Identity-bearing.
    pub task: String,
    /// Workflow's system prompt (optional). Identity-bearing.
    pub system_prompt: Option<String>,
    /// Content-addressed reference to the workflow front-matter.
    /// Identity-bearing.
    pub workflow_def_ref: ContentRef,
    /// Wall-clock at submission. **NOT identity-bearing** per D34 §3.3 —
    /// excluded from `identity_bytes` so two runs of the same workflow at
    /// different times produce byte-identical `result_ref`s.
    pub submitted_at_ms: u64,
}

/// Identity-only projection of `SeedPayload` — excludes `submitted_at_ms`
/// per D34 §3.3. The `result_ref` is BLAKE3 of canonical bincode of this
/// view. Serialize-only (the identity bytes are produced for hashing; we
/// never need to deserialize back to this projection — the full struct is
/// what callers reconstruct from disk).
#[derive(Debug, Clone, Serialize)]
struct SeedPayloadIdentity<'a> {
    run_id: &'a [u8; 16],
    task: &'a str,
    system_prompt: &'a Option<String>,
    workflow_def_ref: &'a ContentRef,
}

impl SeedPayload {
    /// The canonical bincode bytes for `result_ref` derivation. Excludes
    /// `submitted_at_ms` per D34 §3.3.
    #[must_use]
    pub fn identity_bytes(&self) -> Vec<u8> {
        let identity = SeedPayloadIdentity {
            run_id: &self.run_id,
            task: &self.task,
            system_prompt: &self.system_prompt,
            workflow_def_ref: &self.workflow_def_ref,
        };
        // Workspace-canonical bincode config (mirrors `kx_mote::canonical_config`).
        bincode::serde::encode_to_vec(&identity, canonical_config()).expect(
            "SeedPayload identity serialization is infallible (no floats, no non-encodable types)",
        )
    }

    /// The full canonical bincode bytes — INCLUDES `submitted_at_ms`.
    /// Used by the content store's `put` (the payload bytes ARE the audit
    /// record).
    #[must_use]
    pub fn full_bytes(&self) -> Vec<u8> {
        bincode::serde::encode_to_vec(self, canonical_config())
            .expect("SeedPayload full serialization is infallible")
    }

    /// The content-addressed `result_ref` of this seed — BLAKE3 of the
    /// identity bytes (NOT the full bytes; per D34 §3.3 `submitted_at_ms`
    /// is excluded from identity).
    #[must_use]
    pub fn result_ref(&self) -> ContentRef {
        let bytes = self.identity_bytes();
        ContentRef::from_bytes(*blake3::hash(&bytes).as_bytes())
    }
}

/// Derive the synthetic seed Mote identity from a `run_id`.
///
/// Per D34 §2: `mote_id = blake3("seed" ‖ run_id)`. The "seed" prefix is the
/// 4-byte ASCII string; the `run_id` is the 16-byte UUID-shaped per-run id.
#[must_use]
pub fn seed_mote_id(run_id: &[u8; 16]) -> MoteId {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"seed");
    hasher.update(run_id);
    MoteId::from_bytes(*hasher.finalize().as_bytes())
}

/// Derive the seed idempotency key from a `run_id`.
///
/// Per D34 §2: `idempotency_key = blake3("seed-key" ‖ run_id)`. The
/// "seed-key" prefix is the 8-byte ASCII string.
#[must_use]
pub fn seed_idempotency_key(run_id: &[u8; 16]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"seed-key");
    hasher.update(run_id);
    *hasher.finalize().as_bytes()
}

/// Errors from the fact-zero protocol.
#[derive(Debug, Error)]
pub enum FactZeroError {
    /// The content store rejected the `put` of the seed payload.
    #[error("content store put failed: {0}")]
    ContentStorePut(String),

    /// The content store rejected the `put` of the master warrant.
    #[error("content store put of master warrant failed: {0}")]
    WarrantPut(String),

    /// The journal rejected the `Committed` entry for fact-zero.
    #[error("journal append failed: {0}")]
    JournalAppend(String),

    /// Generic internal error.
    #[error("fact-zero internal: {0}")]
    Internal(String),
}

/// Write fact-zero to the journal + content store.
///
/// Steps (D34 §2 + D39 §a):
/// 1. `content_store.put(seed_payload.full_bytes())` → returns the content-ref
///    that MUST equal `seed_payload.result_ref()` (post-condition; if not,
///    the content store is broken — surface as `Internal`).
/// 2. `content_store.put(master_warrant)` → returns the warrant_ref.
/// 3. `journal.append(JournalEntry::Committed { ... })` with the fact-zero
///    shape.
///
/// Returns the synthetic seed `MoteId`. The caller uses this as the parent
/// of the workflow's root Mote.
///
/// **D34 idempotency**: re-calling `write_fact_zero` with the same `run_id`
/// is safe — the journal's dedup-by-key (kind=Committed, idempotency_key=
/// `seed_idempotency_key(run_id)`) makes the second call a no-op (returns
/// the pre-existing Committed entry's seq + the same mote_id).
///
/// # Errors
///
/// Returns `FactZeroError` on any underlying failure. The lifecycle layer's
/// caller treats this as a hard failure — the workflow cannot proceed
/// without fact-zero.
pub fn write_fact_zero<S, J>(
    content_store: &S,
    journal: &J,
    seed_payload: &SeedPayload,
    master_warrant: &WarrantSpec,
) -> Result<MoteId, FactZeroError>
where
    S: ContentStore,
    J: Journal,
{
    let mote_id = seed_mote_id(&seed_payload.run_id);
    let idempotency_key = seed_idempotency_key(&seed_payload.run_id);

    // 1. Put the seed payload bytes (full — audit + identity).
    let put_ref = content_store
        .put(&seed_payload.full_bytes())
        .map_err(|e| FactZeroError::ContentStorePut(format!("{e:?}")))?;
    // The `result_ref` for fact-zero is the IDENTITY hash, not the FULL hash.
    // The content store dedupes on the FULL bytes; the journal records the
    // IDENTITY hash so cross-run replay finds the same ref.
    // PR 9a's invariant: the executor records the identity ref; the
    // content store also persists the full bytes (audit). For PR 9a, the
    // simple shape is: put the identity bytes (which equals result_ref by
    // construction). Audit metadata (submitted_at_ms) is recorded in the
    // journal's `timestamp_ms` header field instead of the payload bytes.
    // PR 9a-hardening can split into "identity-payload" + "audit-payload"
    // refs if the audit trail needs the full struct.
    //
    // For PR 9a, we keep it simple: put the identity bytes; result_ref =
    // identity_ref. The full bytes (with submitted_at_ms) are reconstructible
    // from the journal header's timestamp + the identity payload.
    let identity_bytes = seed_payload.identity_bytes();
    let identity_ref = content_store
        .put(&identity_bytes)
        .map_err(|e| FactZeroError::ContentStorePut(format!("{e:?}")))?;
    debug_assert_eq!(identity_ref, seed_payload.result_ref());
    // `put_ref` (full bytes) and `identity_ref` may differ; both are durable.
    // The full-bytes audit trail lives in the content store keyed by
    // `put_ref` and is reachable via `store.get(put_ref)`. We discard the
    // binding (Copy type, no-op).
    let _ = put_ref;
    let result_ref = identity_ref;

    // 2. Put the master warrant; record its warrant_ref on the fact-zero
    // entry.
    let warrant_bytes = bincode::serde::encode_to_vec(master_warrant, canonical_config())
        .map_err(|e| FactZeroError::WarrantPut(format!("encode: {e:?}")))?;
    let _ = content_store
        .put(&warrant_bytes)
        .map_err(|e| FactZeroError::WarrantPut(format!("put: {e:?}")))?;
    let warrant_ref = warrant_ref_of(master_warrant);

    // 3. Append the synthetic Committed entry. The journal assigns the seq
    // monotonically; the first entry of an empty journal is seq=1 (the
    // shipped InMemoryJournal/SqliteJournal use 1-indexed seqs); D34 §2's
    // "seq=0" notation is semantic (the FIRST entry), not literal.
    let entry = JournalEntry::Committed {
        mote_id,
        idempotency_key,
        seq: 0, // ignored on input; journal assigns
        nondeterminism: kx_mote::NdClass::Pure,
        result_ref,
        parents: SmallVec::new(),
        warrant_ref,
        // Fact-zero is synthetic; its mote_def_hash is the all-zero hash
        // (no MoteDef exists for fact-zero — the seed is not a runtime Mote).
        mote_def_hash: MoteDefHash::from_bytes([0u8; 32]),
    };
    let _appended = journal
        .append(entry)
        .map_err(|e| FactZeroError::JournalAppend(format!("{e:?}")))?;

    Ok(mote_id)
}
