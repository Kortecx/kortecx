//! A captured step record for one atomic Mote (agent).

use kx_content::ContentRef;
use kx_mote::MoteId;

/// An OFF-TRUTH-PATH record of one Mote's (agent's) step. Every payload is
/// content-addressed in blob storage (the `kx-content` store); this record holds
/// only the 32-byte refs + the `MoteId` join key. Integer-only — no float on any
/// path. NEVER journaled, NEVER an identity input, NEVER gates execution.
///
/// `output_ref` (the **action**) is also the Mote's committed `result_ref` on the
/// journal — duplicated here only as a join key, so the capture projection can be
/// rebuilt from, and reconciled against, the journal truth.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepRecord {
    /// The atomic Mote (agent) this step belongs to.
    pub mote_id: MoteId,
    /// The assembled input/context the agent reasoned over (blob ref). Retained
    /// only under [`crate::CaptureScope::Full`].
    pub input_ref: Option<ContentRef>,
    /// The committed output — the **action**. Always retained (it is the truth's
    /// join key).
    pub output_ref: Option<ContentRef>,
    /// The agent's reasoning trace (blob ref). Retained only under
    /// [`crate::CaptureScope::Full`].
    pub reasoning_ref: Option<ContentRef>,
    /// The agent's thinking / scratch (blob ref). Retained only under
    /// [`crate::CaptureScope::Full`].
    pub thinking_ref: Option<ContentRef>,
}

impl StepRecord {
    /// A record holding only the committed action (the always-retained join key).
    #[must_use]
    pub fn action(mote_id: MoteId, output_ref: ContentRef) -> Self {
        Self {
            mote_id,
            input_ref: None,
            output_ref: Some(output_ref),
            reasoning_ref: None,
            thinking_ref: None,
        }
    }

    /// A full step record (input/output/reasoning/thinking). Retained as-authored
    /// only when the session consented to [`crate::CaptureScope::Full`]; under
    /// the default scope the store strips the opt-in fields.
    #[must_use]
    pub fn full(
        mote_id: MoteId,
        input_ref: Option<ContentRef>,
        output_ref: Option<ContentRef>,
        reasoning_ref: Option<ContentRef>,
        thinking_ref: Option<ContentRef>,
    ) -> Self {
        Self {
            mote_id,
            input_ref,
            output_ref,
            reasoning_ref,
            thinking_ref,
        }
    }

    /// This record with the opt-in (non-action) fields stripped — the projection
    /// under [`crate::CaptureScope::ActionsOnly`]. Keeps only the action join key.
    #[must_use]
    pub fn actions_only(&self) -> Self {
        Self {
            mote_id: self.mote_id,
            output_ref: self.output_ref,
            input_ref: None,
            reasoning_ref: None,
            thinking_ref: None,
        }
    }
}
