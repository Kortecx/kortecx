//! The on-disk JSONL wire shape for an [`crate::AuditEvent`].
//!
//! The typed [`crate::AuditEvent`] is time-free and id-typed; this module is the
//! ONE place that (a) renders ids/digests as lowercase hex strings (so the audit
//! log correlates by `grep` with the journal / CLI / `digest` output, which are
//! all hex — the default `serde` derive over a `[u8; 32]` newtype would emit a
//! 32-integer array that no SIEM keyed on hex ids could match), and (b) adds the
//! wall-clock stamp + monotonic sequence + optional principal envelope. Time and
//! "who" live here and ONLY here — never in the typed event, never near the digest.

use serde::Serialize;

use crate::event::AuditEvent;

/// One JSONL line: a stamped, sequenced envelope flattening the event body.
///
/// Serializes to e.g. `{"seq":0,"ts_ms":1717545600000,"type":"run_started","runnable":8}`.
#[derive(Debug, Serialize)]
pub(crate) struct AuditEventWire {
    /// Monotonic per-sink sequence number (a gap signals a dropped/removed line).
    seq: u64,
    /// Wall-clock stamp, epoch milliseconds. OFF the digest / identity path.
    ts_ms: u64,
    /// Optional run-scoped principal ("who"); absent unless the caller sets it.
    #[serde(skip_serializing_if = "Option::is_none")]
    principal: Option<String>,
    /// The internally-tagged event body, flattened onto the line.
    #[serde(flatten)]
    body: AuditBody,
}

impl AuditEventWire {
    /// Build a wire line from a typed event + the sink-supplied stamp/seq/principal.
    pub(crate) fn from_event(
        seq: u64,
        ts_ms: u64,
        event: &AuditEvent,
        principal: Option<String>,
    ) -> Self {
        Self {
            seq,
            ts_ms,
            principal,
            body: AuditBody::from_event(event),
        }
    }
}

/// The internally-tagged (`"type"`) body. New variants are additive for an
/// external JSONL parser (it ignores unknown `type` values).
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AuditBody {
    RunStarted {
        runnable: u32,
    },
    Recovered {
        committed_through: u32,
        folded_through: u64,
    },
    ChildrenDerived {
        shaper: String,
        children: u32,
    },
    MoteDispatched {
        mote_id: String,
        nd_class: &'static str,
        kind: &'static str,
    },
    MoteCommitted {
        mote_id: String,
        result_ref: String,
        nd_class: &'static str,
    },
    MoteFailed {
        mote_id: String,
    },
    MoteRepudiated {
        mote_id: String,
    },
    MoteInconsistent {
        mote_id: String,
    },
    RunCompleted {
        committed: u32,
        total: u32,
        digest: String,
    },
}

impl AuditBody {
    fn from_event(event: &AuditEvent) -> Self {
        match *event {
            AuditEvent::RunStarted { runnable } => Self::RunStarted { runnable },
            AuditEvent::Recovered {
                committed_through,
                folded_through,
            } => Self::Recovered {
                committed_through,
                folded_through,
            },
            AuditEvent::ChildrenDerived { shaper, children } => Self::ChildrenDerived {
                shaper: hex32(shaper.as_bytes()),
                children,
            },
            AuditEvent::MoteDispatched {
                mote_id,
                nd_class,
                kind,
            } => Self::MoteDispatched {
                mote_id: hex32(mote_id.as_bytes()),
                nd_class: nd_str(nd_class),
                kind: kind.as_str(),
            },
            AuditEvent::MoteCommitted {
                mote_id,
                result_ref,
                nd_class,
            } => Self::MoteCommitted {
                mote_id: hex32(mote_id.as_bytes()),
                result_ref: result_ref.to_hex(),
                nd_class: nd_str(nd_class),
            },
            AuditEvent::MoteFailed { mote_id } => Self::MoteFailed {
                mote_id: hex32(mote_id.as_bytes()),
            },
            AuditEvent::MoteRepudiated { mote_id } => Self::MoteRepudiated {
                mote_id: hex32(mote_id.as_bytes()),
            },
            AuditEvent::MoteInconsistent { mote_id } => Self::MoteInconsistent {
                mote_id: hex32(mote_id.as_bytes()),
            },
            AuditEvent::RunCompleted {
                committed,
                total,
                digest,
            } => Self::RunCompleted {
                committed,
                total,
                digest: hex32(&digest),
            },
        }
    }
}

/// Stable lowercase tag for a non-determinism class (matches the rest of the
/// codebase's snake_case wire vocabulary).
fn nd_str(nd: kx_mote::NdClass) -> &'static str {
    match nd {
        kx_mote::NdClass::Pure => "pure",
        kx_mote::NdClass::ReadOnlyNondet => "read_only_nondet",
        kx_mote::NdClass::WorldMutating => "world_mutating",
    }
}

/// Render 32 bytes as 64 lowercase hex chars (matches `MoteId`/`ContentRef`
/// `Display`/`to_hex`). Allocation-bounded, no `unwrap`.
fn hex32(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::DispatchKind;

    #[test]
    fn hex32_matches_lowercase_64char_hex() {
        let s = hex32(&[0xab; 32]);
        assert_eq!(s.len(), 64);
        assert_eq!(s, "ab".repeat(32));
        assert_eq!(hex32(&[0x00; 32]), "0".repeat(64));

        // First eight bytes carry every nibble; the rest are zero.
        let mut b = [0u8; 32];
        b[..8].copy_from_slice(&[0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef]);
        assert_eq!(hex32(&b), format!("0123456789abcdef{}", "00".repeat(24)));
    }

    #[test]
    fn nd_str_is_snake_case() {
        assert_eq!(nd_str(kx_mote::NdClass::Pure), "pure");
        assert_eq!(nd_str(kx_mote::NdClass::ReadOnlyNondet), "read_only_nondet");
        assert_eq!(nd_str(kx_mote::NdClass::WorldMutating), "world_mutating");
    }

    #[test]
    fn dispatch_kind_tags() {
        assert_eq!(DispatchKind::Pure.as_str(), "pure");
        assert_eq!(DispatchKind::Critic.as_str(), "critic");
        assert_eq!(DispatchKind::WmFresh.as_str(), "wm_fresh");
        assert_eq!(DispatchKind::WmRecovery.as_str(), "wm_recovery");
    }
}
