//! D114 (HITL approval) + M11 (cost readout): the [`ApprovalAdmin`] seam behind the
//! four autonomy-safety RPCs (`ListPendingApprovals` / `GrantApproval` /
//! `DenyApproval` / `GetRunCost`).
//!
//! The host impl owns the coordinator handle + the price-book; gateway-core stays a
//! translation/admission layer (the FE thesis test, D101.7 — it never writes the
//! journal). A `None` seam ⇒ the RPCs return `unimplemented` (the trigger/secret
//! forward-compatible-degrade precedent). Grant/Deny are OPERATOR decisions over a
//! SERVER-derived `request_id` (SN-8) — they release/reject a STAGED action, never
//! mint a client warrant.

/// A pending HITL approval, flattened for the operator inbox (display-only; no
/// authority). The `request_id` is the server-derived grant/deny key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingApprovalRow {
    /// 16-byte server-derived handshake handle.
    pub request_id: [u8; 16],
    /// 16-byte run awaiting approval.
    pub instance_id: [u8; 16],
    /// 32-byte proposing observation Mote.
    pub mote_id: [u8; 32],
    /// Proposed tool name (display).
    pub tool_id: String,
    /// Proposed tool version (display).
    pub tool_version: String,
    /// Proposed-action summary (display).
    pub intent: String,
    /// Approval deadline in unix-ms (`0` ⇒ operator-driven).
    pub deadline_unix_ms: u64,
    /// Request creation time in unix-ms (audit).
    pub created_unix_ms: u64,
}

/// A run's DISPLAY-ONLY local spend estimate (M11 — a budget guardrail readout, NOT
/// Cloud per-expert billing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunCostRow {
    /// The run.
    pub instance_id: [u8; 16],
    /// Committed model turns counted.
    pub turns: u64,
    /// Committed tool calls counted.
    pub tool_calls: u64,
    /// `turns*per_turn + tool_calls*per_tool_call` (micro-USD).
    pub estimated_micro_usd: u64,
    /// The run warrant's ceiling (`0` ⇒ no ceiling).
    pub ceiling_micro_usd: u64,
    /// Operator-configured per-turn rate (provenance).
    pub per_turn_micro_usd: u64,
    /// Operator-configured per-tool-call rate (provenance).
    pub per_tool_call_micro_usd: u64,
    /// `estimated >= ceiling` (display flag; only meaningful when `ceiling > 0`).
    pub over_ceiling: bool,
}

/// Errors surfaced by an [`ApprovalAdmin`] op.
#[derive(Debug, thiserror::Error)]
pub enum ApprovalAdminError {
    /// A bad argument (e.g. a malformed `request_id`/`instance_id`). Maps to
    /// `invalid_argument`.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    /// The coordinator was unreachable / a fold fault. Maps to `internal`.
    #[error("approval admin error: {0}")]
    Internal(String),
}

/// The autonomy-safety admin seam behind the four D114/M11 RPCs. Async (consistent
/// with the other admin seams — the ops dispatch coordinator commands). The host impl
/// owns the coordinator handle + the price-book.
#[tonic::async_trait]
pub trait ApprovalAdmin: Send + Sync {
    /// The operator's pending-approvals inbox (every withheld world-mutating action),
    /// clamped to `limit` (`0` ⇒ all).
    async fn list_pending(&self, limit: u32)
        -> Result<Vec<PendingApprovalRow>, ApprovalAdminError>;

    /// GRANT a pending approval (releases the staged action to fire exactly once).
    /// Returns `true` iff a decision was recorded (`false` ⇒ unknown/already-resolved).
    async fn grant(&self, request_id: [u8; 16], reason: &str) -> Result<bool, ApprovalAdminError>;

    /// DENY a pending approval (the gated chain dead-letters fail-closed). See
    /// [`Self::grant`].
    async fn deny(&self, request_id: [u8; 16], reason: &str) -> Result<bool, ApprovalAdminError>;

    /// The run's display-only local spend estimate (M11).
    async fn run_cost(&self, instance_id: [u8; 16]) -> Result<RunCostRow, ApprovalAdminError>;
}
