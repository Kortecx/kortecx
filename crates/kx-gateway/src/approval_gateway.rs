//! D114/M11 host wiring: [`HostApprovalAdmin`] — the [`ApprovalAdmin`] seam impl over
//! the embedded coordinator + the env-resolved price-book.
//!
//! The coordinator stays the sole journal writer (this holds only its `Clone`able
//! service handle; no journal-writer dep). Grant/Deny dispatch coordinator commands
//! that append the durable decision fact; the gated react chain reads it on its next
//! settle pass. The operator id is a fixed OSS single-node stand-in (multi-tenant
//! principal attribution is deferred to Cloud, SN-8).

use kx_coordinator::CoordinatorService;
use kx_gateway_core::{ApprovalAdmin, ApprovalAdminError, PendingApprovalRow, RunCostRow};
use kx_pricing::PriceBook;

/// The configured operator id attributed to OSS single-node approval decisions.
/// Cloud resolves the real authenticated principal; OSS is single-operator.
const OSS_OPERATOR_ID: u64 = 1;

/// The host autonomy-safety admin: the coordinator handle + the env-resolved price-book.
pub(crate) struct HostApprovalAdmin {
    coordinator: CoordinatorService,
    price_book: PriceBook,
}

impl HostApprovalAdmin {
    pub(crate) fn new(coordinator: CoordinatorService) -> Self {
        Self {
            coordinator,
            // Resolve the operator-priced rates from the environment once at wiring.
            price_book: PriceBook::default().with_env_overrides(),
        }
    }
}

// By value so it composes directly as `.map_err(internal)` (map_err yields the error
// owned); the body only needs its Display.
#[allow(clippy::needless_pass_by_value)]
fn internal(e: kx_coordinator::CoordinatorError) -> ApprovalAdminError {
    ApprovalAdminError::Internal(e.to_string())
}

#[tonic::async_trait]
impl ApprovalAdmin for HostApprovalAdmin {
    async fn list_pending(
        &self,
        limit: u32,
    ) -> Result<Vec<PendingApprovalRow>, ApprovalAdminError> {
        let mut views = self
            .coordinator
            .list_pending_approvals()
            .await
            .map_err(internal)?;
        if limit > 0 && views.len() > limit as usize {
            views.truncate(limit as usize);
        }
        Ok(views
            .into_iter()
            .map(|v| PendingApprovalRow {
                request_id: v.request_id,
                instance_id: v.instance_id,
                mote_id: *v.mote_id.as_bytes(),
                tool_id: v.tool_id,
                tool_version: v.tool_version,
                intent: v.intent,
                deadline_unix_ms: v.deadline_unix_ms,
                created_unix_ms: v.created_unix_ms,
            })
            .collect())
    }

    async fn grant(&self, request_id: [u8; 16], reason: &str) -> Result<bool, ApprovalAdminError> {
        self.coordinator
            .grant_approval(request_id, OSS_OPERATOR_ID, reason.to_string())
            .await
            .map_err(internal)
    }

    async fn deny(&self, request_id: [u8; 16], reason: &str) -> Result<bool, ApprovalAdminError> {
        self.coordinator
            .deny_approval(request_id, OSS_OPERATOR_ID, reason.to_string())
            .await
            .map_err(internal)
    }

    async fn run_cost(&self, instance_id: [u8; 16]) -> Result<RunCostRow, ApprovalAdminError> {
        let (turns, tool_calls) = self
            .coordinator
            .run_cost_counts(instance_id)
            .await
            .map_err(internal)?;
        let estimated = self.price_book.estimate_spend(turns, tool_calls);
        // The readout ceiling is the DISPLAY view (the engine enforces the per-run
        // warrant ceiling itself); `0` ⇒ "no ceiling shown". A richer per-run ceiling
        // readout rides the per-run cost_ceiling request field (a follow-up).
        Ok(RunCostRow {
            instance_id,
            turns,
            tool_calls,
            estimated_micro_usd: estimated,
            ceiling_micro_usd: 0,
            per_turn_micro_usd: self.price_book.per_turn_micro_usd,
            per_tool_call_micro_usd: self.price_book.per_tool_call_micro_usd,
            over_ceiling: false,
        })
    }
}
