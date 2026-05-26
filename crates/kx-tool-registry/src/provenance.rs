//! [`ToolProvenance`] + [`RegistrationStatus`] — registration-side lifecycle
//! vocabulary. Drives the HumanAuthored-vs-SelfGenerated routing in
//! [`crate::ToolRegistry::register`].

use kx_mote::MoteId;
use kx_warrant::WarrantSpec;
use serde::{Deserialize, Serialize};

/// Who/what authored this tool — drives the registration lifecycle (D32 §7).
///
/// `HumanAuthored` → registration is immediately
/// [`RegistrationStatus::Approved`]. `SelfGenerated` → registration is
/// [`RegistrationStatus::PendingHumanReview`] until
/// [`crate::ToolRegistry::approve_registration`] is called with the lineage subset
/// check passing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolProvenance {
    /// Authored by a human (operator, workflow author, org maintainer).
    /// Approved on registration.
    HumanAuthored {
        /// Free-form author identifier (audit log only; not enforcement).
        author: String,
    },
    /// Emitted by a Mote. INERT until reviewed.
    SelfGenerated {
        /// The warrant in effect when the Mote emitted the tool. Used at
        /// approve time to enforce `def.required_capability ⊆
        /// generating_lineage_warrant`.
        generating_lineage_warrant: WarrantSpec,
        /// The Mote that emitted the tool.
        generating_mote: MoteId,
    },
}

/// Lifecycle state of a registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RegistrationStatus {
    /// Active and resolvable.
    Approved,
    /// Recorded but INERT — `resolve` refuses with `PendingHumanReview`.
    PendingHumanReview,
}
