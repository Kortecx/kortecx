//! D38 §1 — derive the 32-byte cross-boundary idempotency token.
//!
//! The token roots in the **registered run** (`instance_id`, D64/M1.1) via
//! [`run_scoped_token`] (M1.2): the same Mote re-submitted as a *new* run gets a
//! *different* token, so cross-boundary exactly-once is **per run** — a fresh
//! run fires a fresh effect, while a recovery re-dispatch of the *same* run
//! re-derives the *same* token (the remote API dedups). [`idempotency_token_for`]
//! (MoteId-only) survives for callers with no run context.

use kx_mote::Mote;

/// Length of a run's `instance_id` (the registered run nonce). Kept as a local
/// literal so this crate stays **independent of `kx-journal`** (recovery-state
/// independence); it mirrors `kx_journal::INSTANCE_ID_LEN`.
pub const INSTANCE_ID_LEN: usize = 16;

/// Domain tag for run-scoped idempotency tokens — separates them from
/// `run_root_id` (`"kx-run-root"`) and any bare MoteId, so a token can never
/// collide with another 32-byte identity on a different derivation path.
const RUN_SCOPED_TOKEN_DOMAIN: &[u8] = b"kx-run-scoped-token";

/// Derive the 32-byte cross-boundary idempotency token for a [`Mote`] within a
/// registered run (M1.2, D64).
///
/// `token = blake3("kx-run-scoped-token" ‖ instance_id ‖ mote.id)`. Pure,
/// deterministic, and replay-stable: a recovery re-dispatch of the *same*
/// `(instance_id, mote)` re-derives the *same* token (the remote idempotency
/// header dedups → no double-effect), while the *same* Mote under a *different*
/// registered run produces a *different* token (a re-submitted recipe is a fresh
/// run that fires a fresh effect). The broker passes this through to the remote
/// tool's idempotency header (e.g. `Idempotency-Key: <hex>`); hex-encode per the
/// remote API's wire format (`blake3::Hash::from_bytes(token).to_hex()`).
#[inline]
#[must_use]
pub fn run_scoped_token(instance_id: &[u8; INSTANCE_ID_LEN], mote: &Mote) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(RUN_SCOPED_TOKEN_DOMAIN);
    hasher.update(instance_id);
    hasher.update(mote.id.as_bytes());
    *hasher.finalize().as_bytes()
}

/// Derive the 32-byte idempotency token from a [`Mote`]'s identity alone.
///
/// **Prefer [`run_scoped_token`]** for any caller that has the run's
/// `instance_id` (the distributed dispatch path does, M1.2): MoteId-only keying
/// means the *same* Mote re-submitted as a *new* run would collide on the
/// cross-boundary key. This MoteId-only form survives for callers with no run
/// context. Per D38 §1, the broker passes the token through to a remote tool's
/// idempotency header (e.g., `Idempotency-Key: <hex>`) so a recovery re-dispatch
/// of the same Mote produces the SAME token and the remote API returns the
/// cached response (no double-effect). The 32-byte form is the raw
/// [`MoteId`][kx_mote::MoteId] bytes; hex-encode per the remote API's wire
/// format (e.g., `blake3::Hash::from_bytes(token).to_hex()`).
///
/// # Example
///
/// ```
/// use kx_capability::idempotency_token_for;
/// use kx_mote::{
///     EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote,
///     MoteDef, NdClass, PromptTemplateHash,
/// };
/// use smallvec::SmallVec;
/// use std::collections::BTreeMap;
///
/// let def = MoteDef {
///     logic_ref: LogicRef::from_bytes([0u8; 32]),
///     model_id: ModelId("m".into()),
///     prompt_template_hash: PromptTemplateHash::from_bytes([0u8; 32]),
///     tool_contract: BTreeMap::new(),
///     nd_class: NdClass::Pure,
///     config_subset: BTreeMap::new(),
///     effect_pattern: EffectPattern::IdempotentByConstruction,
///     critic_for: None,
///     is_topology_shaper: false,
///     inference_params: kx_mote::InferenceParams::default(),
///     critic_check: None,
///     schema_version: kx_mote::MOTE_DEF_SCHEMA_VERSION,
/// };
/// let mote = Mote::new(
///     def,
///     InputDataId::from_bytes([0u8; 32]),
///     GraphPosition("/root".into()),
///     SmallVec::new(),
/// );
/// let token = idempotency_token_for(&mote);
/// assert_eq!(token.len(), 32);
/// assert_eq!(&token, mote.id.as_bytes());
/// ```
#[inline]
#[must_use]
pub fn idempotency_token_for(mote: &Mote) -> [u8; 32] {
    *mote.id.as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kx_mote::{
        EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, NdClass,
        PromptTemplateHash,
    };
    use smallvec::SmallVec;
    use std::collections::BTreeMap;

    fn sample_mote(pos: &str) -> Mote {
        let def = MoteDef {
            logic_ref: LogicRef::from_bytes([0u8; 32]),
            model_id: ModelId("m".into()),
            prompt_template_hash: PromptTemplateHash::from_bytes([0u8; 32]),
            tool_contract: BTreeMap::new(),
            nd_class: NdClass::WorldMutating,
            config_subset: BTreeMap::new(),
            effect_pattern: EffectPattern::StageThenCommit,
            critic_for: None,
            is_topology_shaper: false,
            inference_params: kx_mote::InferenceParams::default(),
            critic_check: None,
            schema_version: kx_mote::MOTE_DEF_SCHEMA_VERSION,
        };
        Mote::new(
            def,
            InputDataId::from_bytes([0u8; 32]),
            GraphPosition(pos.into()),
            SmallVec::new(),
        )
    }

    #[test]
    fn run_scoped_token_is_stable_within_a_run() {
        let mote = sample_mote("/root");
        let id = [0x11u8; INSTANCE_ID_LEN];
        // A recovery re-dispatch of the SAME (instance, mote) re-derives the
        // SAME token (the remote idempotency header dedups).
        assert_eq!(run_scoped_token(&id, &mote), run_scoped_token(&id, &mote));
    }

    #[test]
    fn run_scoped_token_differs_across_runs_for_same_mote() {
        let mote = sample_mote("/root");
        let run_a = [0x11u8; INSTANCE_ID_LEN];
        let run_b = [0x22u8; INSTANCE_ID_LEN];
        // The SAME Mote under a DIFFERENT registered run → DIFFERENT token, so a
        // re-submitted recipe is a fresh run that fires a fresh effect (D64).
        assert_ne!(
            run_scoped_token(&run_a, &mote),
            run_scoped_token(&run_b, &mote)
        );
    }

    #[test]
    fn run_scoped_token_is_domain_separated() {
        let mote = sample_mote("/root");
        let id = [0x33u8; INSTANCE_ID_LEN];
        let token = run_scoped_token(&id, &mote);
        // Never the bare MoteId (the old MoteId-only token).
        assert_ne!(&token, mote.id.as_bytes());
        // Never a bare blake3(instance_id ‖ mote_id) without the domain tag.
        let mut bare = blake3::Hasher::new();
        bare.update(&id);
        bare.update(mote.id.as_bytes());
        assert_ne!(&token, bare.finalize().as_bytes());
    }

    #[test]
    fn run_scoped_token_distinguishes_distinct_motes() {
        let id = [0x44u8; INSTANCE_ID_LEN];
        assert_ne!(
            run_scoped_token(&id, &sample_mote("/a")),
            run_scoped_token(&id, &sample_mote("/b"))
        );
    }
}
