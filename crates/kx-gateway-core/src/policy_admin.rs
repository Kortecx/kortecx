//! The durable Policy/Role registry seam — `PutPolicyRole` / `ListPolicyRoles` /
//! `DeletePolicyRole` / `AssignPolicyRole`.
//!
//! Spoken entirely in gateway-core's OWN wire vocabulary (`String` / `u64`), the
//! [`crate::script_admin::ScriptAdmin`] pattern — no host type crosses the seam.
//! The host (`kx-gateway`) implements it over `policies.db`.
//!
//! # What a role IS, and what it can do
//!
//! A Policy/Role is a NAMED, reusable narrowing of tool authority. Assigning one
//! to a party makes that party's effective tool set the INTERSECTION of every
//! present authority leg — so a role can only ever take capability AWAY.
//!
//! This matters more than it sounds. The obvious alternative, "a role GRANTS the
//! tools it names", would make the registry an authority-minting surface: anyone
//! who could write a role could write themselves a capability. Intersection
//! makes the registry safe to expose at all, because the worst a malicious role
//! can do is refuse work.
//!
//! # Boundaries
//!
//! - **Naming is never granting.** A role may name a tool the party could not
//!   fire anyway; the intersection simply drops it. The registry never widens.
//! - **A party with no role assigned resolves EXACTLY as it does today.** That
//!   is not a convenience, it is the compatibility contract: the (no role, no
//!   asset allowlist) case must stay byte-identical or every running install
//!   changes behaviour on upgrade.
//! - **Caller-scoped.** Roles live in the calling principal's scope. A
//!   serve-wide role would be an authority a single-node operator cannot
//!   delegate away.
//! - **Off the truth path.** `policies.db` is off-journal and off-digest;
//!   nothing here can move the canonical digest.
//! - **Authored work.** The store opens `Durability::UserAuthored`, so a schema
//!   bump renames it aside and re-imports rather than rebuilding it empty. A
//!   role a user wrote cannot be regenerated from anything the runtime still
//!   has.
//! - **`None` seam ⇒ `unimplemented`.** A gateway without policies wired
//!   degrades forward-compatibly.

/// One `(tool_id, tool_version)` pair a role narrows TO.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PolicyRoleToolWire {
    /// The tool's grant-set key half.
    pub tool_id: String,
    /// The tool's version half.
    pub tool_version: String,
}

/// One stored role.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyRoleRow {
    /// Catalog key within the caller's scope.
    pub name: String,
    /// Free-form; NEVER parsed for enforcement.
    pub description: String,
    /// The allowlist this role narrows to. EMPTY is meaningful and legal: a role
    /// that names no tool refuses every tool, which is a coherent thing to want
    /// and is not the same as having no role at all.
    pub tools: Vec<PolicyRoleToolWire>,
    /// Creation wall-clock (audit).
    pub created_unix_ms: u64,
    /// Last-update wall-clock (audit).
    pub updated_unix_ms: u64,
}

/// Why a policy-registry call was refused.
#[derive(Debug, thiserror::Error)]
pub enum PolicyAdminError {
    /// A malformed field — an empty name, an oversized description, a tool pair
    /// with an empty half. Maps to `invalid_argument`.
    #[error("invalid policy role: {0}")]
    InvalidArgument(String),
    /// The named role does not exist (assignment only). Maps to `not_found`;
    /// assigning a role that is not there must not silently succeed, because a
    /// silent success reads as "narrowed" while the party stays unnarrowed.
    #[error("no such policy role: {0}")]
    NotFound(String),
    /// A durable-store failure. Maps to `internal`.
    #[error("policy storage error: {0}")]
    Storage(String),
}

/// The durable Policy/Role registry seam. A `None` seam ⇒ the RPCs return
/// `unimplemented`.
pub trait PolicyAdmin: Send + Sync {
    /// Create or update a role by exact name within `principal`'s scope. Returns
    /// `true` iff the role did not previously exist.
    ///
    /// # Errors
    /// [`PolicyAdminError`] on an invalid field or a storage failure.
    fn put(&self, principal: &str, role: PolicyRoleRow) -> Result<bool, PolicyAdminError>;

    /// One deterministic name-ordered page of `principal`'s roles. `limit` is
    /// pre-clamped by the service.
    ///
    /// # Errors
    /// [`PolicyAdminError`] on a read failure.
    fn list(&self, principal: &str, limit: usize) -> Result<Vec<PolicyRoleRow>, PolicyAdminError>;

    /// Delete a role by exact name. Returns `true` iff a row was removed.
    ///
    /// Deleting a role that parties are still assigned to WIDENS those parties
    /// back to their un-narrowed authority. That is the honest outcome — the
    /// alternative, refusing the delete, makes a role permanent the moment it is
    /// used — but it is a widening, so the host must record it.
    ///
    /// # Errors
    /// [`PolicyAdminError`] on a storage failure.
    fn delete(&self, principal: &str, name: &str) -> Result<bool, PolicyAdminError>;

    /// Assign `name` to `party`, or UNASSIGN when `name` is `None`. Returns
    /// `true` iff a role is assigned afterwards.
    ///
    /// # Errors
    /// [`PolicyAdminError::NotFound`] when `name` names no stored role, or
    /// [`PolicyAdminError::Storage`] on a storage failure.
    fn assign(
        &self,
        principal: &str,
        party: &str,
        name: Option<&str>,
    ) -> Result<bool, PolicyAdminError>;

    /// The tool allowlist a party's assigned role narrows to, if any.
    ///
    /// `None` means "this party expresses no per-tool narrowing" and MUST resolve
    /// exactly as a serve with no policy registry at all — it is the arm that
    /// keeps existing installs byte-identical. `Some(set)` is a strict
    /// allowlist, and `Some(empty)` legitimately means "nothing".
    ///
    /// # Errors
    /// [`PolicyAdminError`] on a read failure.
    fn allowlist_for(
        &self,
        principal: &str,
        party: &str,
    ) -> Result<Option<Vec<PolicyRoleToolWire>>, PolicyAdminError>;
}
