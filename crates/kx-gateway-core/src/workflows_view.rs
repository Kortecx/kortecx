//! The Workflow-catalog seam behind `SaveWorkflow` / `ListWorkflows` /
//! `GetWorkflow` / `RunWorkflow` / `DeleteWorkflow`.
//!
//! A "workflow" is a `kortecx.workflow/v1` envelope — the same portable
//! blueprint an App carries, wrapped with the same by-reference rail — stored
//! as a first-class entity of its own. Spoken in gateway-core's own wire
//! vocabulary — **opaque envelope BYTES** + a host-derived [`WorkflowRecord`]
//! summary + a `[u8; 16]` ref. No envelope type crosses the seam, so
//! gateway-core never links `kx-app`; the host canonicalizes + validates and
//! derives the summary + `workflow_ref` (the App-catalog seam's posture,
//! verbatim).
//!
//! # Boundaries (load-bearing — the same wall as the App catalog)
//!
//! - **Off the truth path.** `workflows.db` is REBUILDABLE-TO-EMPTY: never
//!   journaled, never a `MoteId` input, never a digest input.
//! - **Carries NO authority.** `RunWorkflow` re-lowers the blueprint and the
//!   server re-resolves every warrant from the caller's OWN grants.
//! - **Server-derived id.** `workflow_ref = blake3("kx-workflow\0" ‖ handle ‖
//!   0 ‖ canonical(envelope))[..16]`; the host re-canonicalizes received bytes
//!   so client byte-ordering never affects identity.
//! - **Caller-scoped.** Every method takes the SERVER-RESOLVED `principal`;
//!   uniform not-found for absent OR not-owned.
//! - **`None` seam ⇒ degrade.** A host without the sidecar leaves the RPCs
//!   `unimplemented`.
//!
//! # Why every method is REQUIRED (no defaults)
//!
//! [`crate::AppCatalog`]'s `delete`/`set_lifecycle` are defaulted for
//! back-compat with impls that predate them — and a real store silently
//! inheriting a default is exactly how `DeleteApp` shipped broken. This trait
//! is BORN with its full surface: there are no pre-existing impls to protect,
//! so every method is required and a store that cannot delete simply does not
//! implement the trait.

use kx_content::ContentRef;

use crate::error::GatewayError;

/// Domain-separation tag for the handle-free Workflow identity
/// ([`workflow_digest_of`]). The same versioned-contract discipline as
/// [`crate::APP_DIGEST_DOMAIN`]: every producer computes it byte-for-byte
/// identically; an algorithm change bumps the `/vN` tag.
pub const WORKFLOW_DIGEST_DOMAIN: &[u8] = b"kortecx.workflow-digest/v1\0";

/// `workflow_digest = blake3(WORKFLOW_DIGEST_DOMAIN ‖ canonical_envelope)` —
/// the FULL 32-byte, HANDLE-FREE identity of a workflow (the portable "same
/// workflow" key; exact-equality only, never a similarity key).
#[must_use]
pub fn workflow_digest_of(canonical: &[u8]) -> [u8; 32] {
    let mut keyed = Vec::with_capacity(WORKFLOW_DIGEST_DOMAIN.len() + canonical.len());
    keyed.extend_from_slice(WORKFLOW_DIGEST_DOMAIN);
    keyed.extend_from_slice(canonical);
    ContentRef::of(&keyed).0
}

/// A stored workflow's summary — the catalog/display view. The envelope bytes
/// are opaque to gateway-core; the host derives every field from the canonical
/// JSON.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkflowRecord {
    /// 16-byte SERVER-DERIVED canonical-envelope hash (display + dedup signal).
    pub workflow_ref: [u8; 16],
    /// The canonical `namespace/collection/name` handle (the upsert key).
    pub handle: String,
    /// Envelope name.
    pub name: String,
    /// Envelope version.
    pub version: String,
    /// Advisory description (never parsed for enforcement).
    pub description: String,
    /// Advisory one-line statement of what a run PRODUCES (the
    /// [`crate::AppRecord::delivers`] posture — denormalized so one
    /// `ListWorkflows` yields every candidate's output line).
    pub delivers: String,
    /// Catalog tags.
    pub tags: Vec<String>,
    /// Blueprint step count (display only).
    pub step_count: u32,
    /// OPTIONAL 32-byte lineage hint — the `workflow_digest` this workflow was
    /// cloned from (`None` ⇒ authored-here). Off-identity, off-journal,
    /// off-digest. A provenance hint, never authenticity.
    pub source_digest: Option<Vec<u8>>,
    /// Advisory catalog lifecycle: `""` (active) or `"draft"` (authoring
    /// incomplete — finish or discard). CALLER-STATED at save: a workflow has
    /// no scaffold loop, the save IS the authoring act, so there is no second
    /// writer whose value a re-save could clobber (the reason
    /// `AppCatalog::save` must preserve does not arise here). Display, routing
    /// and trigger-registration refusal only; never run enforcement.
    pub lifecycle: String,
}

/// The [`WorkflowRecord::lifecycle`] value marking an authoring-incomplete workflow.
pub const WORKFLOW_LIFECYCLE_DRAFT: &str = "draft";

/// The Workflow-catalog store seam. Opaque envelope bytes cross the seam;
/// identity + summary are host-derived. A `None` seam on the service ⇒ the
/// RPCs return `unimplemented`. Every method is REQUIRED (see the module
/// header for why this trait has no defaults).
pub trait WorkflowCatalog: Send + Sync {
    /// Upsert the envelope bound to `(principal, handle)`. The host validates +
    /// canonicalizes `envelope_json`, derives `workflow_ref` + the summary, and
    /// stores the canonical bytes with the CALLER-STATED `lifecycle` (`""` or
    /// [`WORKFLOW_LIFECYCLE_DRAFT`]; the handler refuses anything else).
    /// `source_digest` is an OPTIONAL 32-byte off-identity lineage hint.
    /// Returns `(record, deduplicated)` where `deduplicated` is `true` iff an
    /// identical canonical envelope was already bound here WITH the same
    /// lifecycle (a lifecycle flip on identical bytes is a real write — a
    /// draft being finished must not read as a no-op).
    ///
    /// # Errors
    /// [`GatewayError::InvalidArgument`] if the envelope fails validation;
    /// [`GatewayError::Internal`] on a host write failure.
    fn save(
        &self,
        principal: &str,
        handle: &str,
        envelope_json: &[u8],
        source_digest: Option<&[u8]>,
        lifecycle: &str,
    ) -> Result<(WorkflowRecord, bool), GatewayError>;

    /// List `principal`'s workflows in deterministic handle order, paged.
    /// Returns `(records, has_more)`; `after_handle` is an exclusive cursor.
    ///
    /// # Errors
    /// A host read failure ([`GatewayError::Internal`]).
    fn list(
        &self,
        principal: &str,
        limit: usize,
        after_handle: Option<&str>,
    ) -> Result<(Vec<WorkflowRecord>, bool), GatewayError>;

    /// Fetch `(record, canonical_envelope_bytes)` bound to `(principal,
    /// handle)`, if any (caller-scoped; uniform not-found for absent OR
    /// not-owned).
    ///
    /// # Errors
    /// A host read failure ([`GatewayError::Internal`]).
    fn get(
        &self,
        principal: &str,
        handle: &str,
    ) -> Result<Option<(WorkflowRecord, Vec<u8>)>, GatewayError>;

    /// Drop the row bound to `(principal, handle)`. Returns `true` iff a row
    /// existed and was removed (`false` uniformly for absent OR not-owned — no
    /// existence oracle). Unbinds the POINTER only: content-addressed blobs
    /// and the definition branch's HISTORY stay (delete + restore is the
    /// recreate path). Cascading the things that merely REFERENCE the workflow
    /// — triggers, the definition-branch binding — is the caller's job.
    ///
    /// # Errors
    /// [`GatewayError::Internal`] on a host write failure.
    fn delete(&self, principal: &str, handle: &str) -> Result<bool, GatewayError>;

    /// Set `(principal, handle)`'s advisory lifecycle (`""` active /
    /// [`WORKFLOW_LIFECYCLE_DRAFT`]). Exists beside the caller-stated save
    /// lifecycle for the RESYNC path (a branch restore re-saves the restored
    /// definition and must carry the row's existing lifecycle through
    /// unchanged). Returns `true` iff a row existed and was updated (`false`
    /// uniformly for absent OR not-owned).
    ///
    /// # Errors
    /// [`GatewayError::Internal`] on a host write failure.
    fn set_lifecycle(
        &self,
        principal: &str,
        handle: &str,
        lifecycle: &str,
    ) -> Result<bool, GatewayError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps_view::{app_digest_of, APP_DIGEST_DOMAIN};
    use proptest::prelude::*;

    proptest! {
        /// `workflow_digest_of` is a PURE function matching the exact
        /// `blake3(WORKFLOW_DIGEST_DOMAIN ‖ bytes)` contract for ANY input.
        #[test]
        fn workflow_digest_of_is_pure_and_matches_the_contract(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
            prop_assert_eq!(workflow_digest_of(&bytes), workflow_digest_of(&bytes));
            let mut preimage = WORKFLOW_DIGEST_DOMAIN.to_vec();
            preimage.extend_from_slice(&bytes);
            prop_assert_eq!(workflow_digest_of(&bytes), ContentRef::of(&preimage).0);
        }

        /// Domain separation: the SAME canonical bytes never collide across the
        /// App and Workflow digest namespaces (a workflow can never impersonate
        /// an App by identity, or vice versa).
        #[test]
        fn workflow_digest_is_domain_separated_from_app_digest(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
            prop_assert_ne!(workflow_digest_of(&bytes), app_digest_of(&bytes));
        }
    }

    #[test]
    fn digest_domains_are_distinct_constants() {
        assert_ne!(WORKFLOW_DIGEST_DOMAIN, APP_DIGEST_DOMAIN);
        assert_ne!(WORKFLOW_DIGEST_DOMAIN, b"kx-workflow\0".as_slice());
        assert_eq!(workflow_digest_of(b"{}").len(), 32);
    }
}
