//! Model Control v2 — the active-default-model seam behind `SetActiveModel` +
//! `GetServerInfo.active_model_id` + `ModelSummary.active`.
//!
//! The active model is an OFF-JOURNAL advisory HINT (SN-8): it authorizes nothing and
//! the server NEVER silently re-routes `kx/recipes/chat`. It exists so the default a
//! client chats with is switchable from CLI/SDK (a client-local default cannot be
//! read by another surface). A chat turn still picks its model by the per-model chat
//! handle / recipe ENUM free-param; this is just the default that handle resolves to.
//! `None` ⇒ `SetActiveModel` returns `unimplemented` and `active_model_id` stays "".

use crate::error::GatewayError;

/// The host-side active-default-model control. The host impl holds an interior-mutable
/// id (off-journal RAM, rebuilt empty/primary on restart) + the served catalog handle
/// it validates against (fail-closed: an unknown id is refused).
pub trait ActiveModelControl: Send + Sync {
    /// The current active default model id; `None` ⇒ the primary (no override).
    fn get(&self) -> Option<String>;

    /// Set the active default. An empty `model_id` CLEARS the override (back to the
    /// primary). Fail-closed: a non-empty id MUST be in the served catalog, else
    /// [`GatewayError::NotFound`] (never an unrouteable active model). Returns the
    /// active id AFTER the op (`None` ⇒ cleared).
    ///
    /// # Errors
    /// [`GatewayError::NotFound`] when `model_id` is not in the served catalog.
    fn set(&self, model_id: &str) -> Result<Option<String>, GatewayError>;
}
