// SPDX-License-Identifier: Apache-2.0
//! The grant-ledger seam (M7.2, D86): the backend-agnostic [`GrantLedger`]
//! trait, its append-only fact vocabulary ([`LedgerFact`] / [`AssetBinding`]),
//! the query result types ([`EffectiveGrants`] / [`GrantWarrant`]), and the
//! [`AppendOutcome`] / [`LedgerError`] outcomes. The in-memory reference backend
//! is [`crate::InMemoryGrantLedger`].
//!
//! ## "Journaled" = the D-LOCK-4 discipline, applied HERE
//!
//! Like the M7.1 [`crate::CatalogRegistry`], this is a **separate truth**: it is
//! authoritative for *what grants exist*, and does NOT depend on `kx-journal`
//! (the dependency direction is the wall). D86's "journaled fact" is realized as
//! the DISCIPLINE — append-only, content-addressed, immutable, idempotent,
//! revoke-by-new-fact — inside the ledger. A durable / cloud backend (D94)
//! implements the SAME trait; the in-memory backend is rebuildable, not durable.
//!
//! ## Authorization is the fold, never the fact
//!
//! Appends are minimal and unchecked: a grant or revocation is recorded as a
//! fact regardless of whether it conveys anything. Authority is computed by the
//! fail-closed fold ([`GrantLedger::effective_grants`] /
//! [`GrantLedger::resolve_effective_warrant_for`]) — a forged, widening,
//! laundering, off-owner, or over-deep grant conveys nothing. The fold reuses
//! the FROZEN `kx_warrant::intersect`, so a runtime warrant can never widen.

use std::sync::Arc;

use kx_warrant::{NarrowingError, WarrantSpec};
use serde::{Deserialize, Serialize};

use crate::action::{CatalogAction, CatalogActionSet};
use crate::grant::{Grant, GrantId, Revocation};
use crate::party::PartyId;
use crate::path::AssetRef;
use crate::signature::canonical_config;

/// A 32-byte BLAKE3 hash.
type Hash32 = [u8; 32];

/// The maximum delegation-chain depth the fold will walk. A chain deeper than
/// this fails closed (conveys nothing) rather than walking unbounded — a hard
/// DoS / stack-growth bound. The fold is iterative, so this caps work, never the
/// call stack.
pub const MAX_DELEGATION_DEPTH: usize = 64;

/// The content-addressed identity of a [`LedgerFact`] — equal to the content id
/// of the fact's payload (a binding's `binding_id`, a grant's `GrantId`, a
/// revocation's `RevocationId`). The ledger dedups + enforces immutability by
/// this id.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FactId(pub Hash32);

impl FactId {
    /// Construct from raw 32 bytes.
    #[inline]
    #[must_use]
    pub const fn from_bytes(bytes: Hash32) -> Self {
        Self(bytes)
    }

    /// Borrow the underlying 32 bytes.
    #[inline]
    #[must_use]
    pub const fn as_bytes(&self) -> &Hash32 {
        &self.0
    }

    /// Lowercase 64-char hex.
    #[must_use]
    pub fn to_hex(self) -> String {
        blake3::Hash::from_bytes(self.0).to_hex().to_string()
    }
}

impl std::fmt::Debug for FactId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FactId({})", blake3::Hash::from_bytes(self.0).to_hex())
    }
}

/// The genesis fact binding an asset to its owning party. The owner is the only
/// party whose ROOT grant on the asset conveys authority.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct AssetBinding {
    asset: AssetRef,
    owner: PartyId,
}

impl AssetBinding {
    /// Bind `asset` to `owner`.
    #[must_use]
    pub fn new(asset: AssetRef, owner: PartyId) -> Self {
        Self { asset, owner }
    }

    /// The bound asset.
    #[inline]
    #[must_use]
    pub fn asset(&self) -> &AssetRef {
        &self.asset
    }

    /// The owning party.
    #[inline]
    #[must_use]
    pub fn owner(&self) -> &PartyId {
        &self.owner
    }

    /// The content-addressed identity:
    /// `blake3(b"kx-catalog/asset-binding/v1" ‖ canonical_bincode(self))`.
    #[must_use]
    pub fn binding_id(&self) -> FactId {
        let mut h = blake3::Hasher::new();
        h.update(b"kx-catalog/asset-binding/v1");
        // SAFETY (expect): AssetBinding is AssetRef (strings + [u8;32]) + PartyId
        // (string) — no floats, no non-encodable variant; bincode encode is
        // infallible (mirrors `Grant::grant_id`).
        let body = bincode::serde::encode_to_vec(self, canonical_config()).expect(
            "AssetBinding canonical encoding is infallible (no floats, no non-encodable types)",
        );
        h.update(&body);
        FactId(*h.finalize().as_bytes())
    }
}

/// An append-only ledger fact. A closed enum — there is no untyped fact. `Grant`
/// is boxed so the enum stays pointer-sized (the grant is the large variant).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum LedgerFact {
    /// An asset → owner binding.
    Bind(AssetBinding),
    /// A capability grant.
    Grant(Box<Grant>),
    /// A revocation (revoke by new fact, D-LOCK-4).
    Revoke(Revocation),
}

impl LedgerFact {
    /// The content-addressed id of this fact (= the payload's content id). The
    /// per-payload domain tags (`asset-binding` / `grant` / `revocation`) keep
    /// cross-kind ids distinct.
    #[must_use]
    pub fn fact_id(&self) -> FactId {
        match self {
            Self::Bind(b) => b.binding_id(),
            Self::Grant(g) => FactId(*g.grant_id().as_bytes()),
            Self::Revoke(r) => FactId(*r.revocation_id().as_bytes()),
        }
    }
}

/// The outcome of an append: a fresh insert vs. an idempotent no-op (the fact
/// was byte-identically present).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AppendOutcome {
    /// First append of this fact.
    Appended(FactId),
    /// A byte-identical fact was already present — no-op (idempotent).
    AlreadyPresent(FactId),
}

impl AppendOutcome {
    /// The fact id this outcome refers to.
    #[inline]
    #[must_use]
    pub const fn fact_id(&self) -> FactId {
        match self {
            Self::Appended(f) | Self::AlreadyPresent(f) => *f,
        }
    }

    /// `true` iff this was a fresh append (not an idempotent no-op).
    #[inline]
    #[must_use]
    pub const fn is_appended(&self) -> bool {
        matches!(self, Self::Appended(_))
    }
}

/// A grant-ledger append failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LedgerError {
    /// A DIFFERENT fact already exists at this [`FactId`]. Facts are
    /// content-addressed, so the same id MUST mean the same bytes — a mismatch
    /// is a hash-collision tripwire (cryptographically unreachable): refuse
    /// loudly rather than overwrite. Carries the conflicting id (hex).
    #[error("immutable ledger conflict at fact_id {0}")]
    ImmutabilityConflict(String),
    /// An asset is already bound to a DIFFERENT owner. An asset has exactly one
    /// owner; re-binding to a new owner is refused (the binding is genesis).
    #[error("asset ownership conflict: {0}")]
    OwnerConflict(String),
}

/// One active grant chain a party holds on an asset, paired with the catalog
/// actions it conveys AND the runtime warrant a `Use` under THOSE actions runs
/// with. The `actions` and `warrant` come from the SAME folded chain — there is
/// no constructor that pairs a warrant from one chain with actions from another,
/// so the action/warrant decoupling that an earlier draft suffered is
/// **unrepresentable**.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GrantWarrant {
    leaf: GrantId,
    actions: CatalogActionSet,
    warrant: WarrantSpec,
}

impl GrantWarrant {
    /// Internal constructor — only the ledger fold builds these, guaranteeing
    /// `actions` + `warrant` originate from one chain.
    pub(crate) fn new(leaf: GrantId, actions: CatalogActionSet, warrant: WarrantSpec) -> Self {
        Self {
            leaf,
            actions,
            warrant,
        }
    }

    /// The leaf grant id identifying this chain.
    #[inline]
    #[must_use]
    pub fn leaf(&self) -> GrantId {
        self.leaf
    }

    /// The effective catalog actions this chain conveys (post chain-narrowing).
    #[inline]
    #[must_use]
    pub fn actions(&self) -> &CatalogActionSet {
        &self.actions
    }

    /// The chain's effective runtime warrant.
    #[inline]
    #[must_use]
    pub fn warrant(&self) -> &WarrantSpec {
        &self.warrant
    }

    /// `true` iff this chain conveys `action`.
    #[inline]
    #[must_use]
    pub fn conveys(&self, action: CatalogAction) -> bool {
        self.actions.contains(action)
    }
}

/// The catalog actions a party effectively holds on an asset — the union across
/// all of their active grant chains, plus the per-chain action breakdown. The
/// actions-only view (no warrant): warrant resolution is action-aligned and
/// lives in [`GrantLedger::resolve_effective_warrant_for`].
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct EffectiveGrants {
    per_grant: Vec<(GrantId, CatalogActionSet)>,
    actions: CatalogActionSet,
}

impl EffectiveGrants {
    /// Build from the folded per-chain `(leaf, actions)` pairs.
    pub(crate) fn from_parts(per_grant: Vec<(GrantId, CatalogActionSet)>) -> Self {
        let mut actions = CatalogActionSet::None;
        for (_, a) in &per_grant {
            actions = actions.union(a);
        }
        Self { per_grant, actions }
    }

    /// The union of actions across every active chain (the `is_authorized` view).
    #[inline]
    #[must_use]
    pub fn actions(&self) -> &CatalogActionSet {
        &self.actions
    }

    /// The leaf grant ids of every active chain.
    pub fn active(&self) -> impl Iterator<Item = GrantId> + '_ {
        self.per_grant.iter().map(|(g, _)| *g)
    }

    /// The leaf grant ids of the active chains that convey `action`.
    pub fn grants_conveying(&self, action: CatalogAction) -> impl Iterator<Item = GrantId> + '_ {
        self.per_grant
            .iter()
            .filter(move |(_, a)| a.contains(action))
            .map(|(g, _)| *g)
    }

    /// `true` iff the party holds no active grant on the asset.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.per_grant.is_empty()
    }
}

/// `true` iff warrant `a` conveys NO MORE capability than `b` on every axis
/// (`a ⊆ b`). Reuses the frozen per-axis `is_subset_of` / `is_within`
/// primitives; never synthesizes. Qualitative axes use subset; quantitative axes
/// use `≤`; `tls_required` is restrictive-when-true (so `a ⊆ b` requires `a` at
/// least as restrictive, i.e. `a.tls_required || !b.tls_required`); the
/// child-set axes (class / executor / environment / syscall profile) require
/// EQUALITY — a difference there makes the two warrants incomparable, never one
/// "within" the other.
pub(crate) fn warrant_within(a: &WarrantSpec, b: &WarrantSpec) -> bool {
    // The destructure below is the drift guard: adding a `WarrantSpec` axis
    // fails to compile here, forcing this comparison (and `most_permissive`) to
    // be revisited rather than silently ignoring the new axis.
    let WarrantSpec {
        mote_class,
        nd_class,
        fs_scope,
        net_scope,
        syscall_profile_ref,
        tool_grants,
        model_route,
        resource_ceiling,
        environment_ref,
        executor_class,
        secret_scope,
        cost_ceiling,
        tls_required,
    } = a;

    // Qualitative (subset) axes.
    fs_scope.is_subset_of(&b.fs_scope)
        && net_scope.is_subset_of(&b.net_scope)
        && secret_scope.is_subset_of(&b.secret_scope)
        && tool_grants.is_subset(&b.tool_grants)
        // Quantitative axes (per-axis ≤).
        && model_route.is_within(&b.model_route)
        && resource_ceiling.cpu_milli <= b.resource_ceiling.cpu_milli
        && resource_ceiling.mem_bytes <= b.resource_ceiling.mem_bytes
        && resource_ceiling.wall_clock_ms <= b.resource_ceiling.wall_clock_ms
        && resource_ceiling.fd_count <= b.resource_ceiling.fd_count
        && resource_ceiling.disk_bytes <= b.resource_ceiling.disk_bytes
        && cost_ceiling.micro_usd <= b.cost_ceiling.micro_usd
        // tls_required: a no more permissive than b ⇔ a at least as restrictive.
        && (*tls_required || !b.tls_required)
        // Child-set / opaque axes: equality (difference ⇒ incomparable).
        && *mote_class == b.mote_class
        && *nd_class == b.nd_class
        && *syscall_profile_ref == b.syscall_profile_ref
        && *environment_ref == b.environment_ref
        && *executor_class == b.executor_class
}

/// Select the MOST-PERMISSIVE real warrant among `candidates`, breaking ties /
/// incomparable maxima by the lexicographically-smallest leaf [`GrantId`] (a
/// stable, content-addressed order). Never synthesizes a warrant — the result is
/// always one a single grant chain actually conveyed (each is itself
/// `intersect(owner_root, …chain…)`, hence `⊆ owner_root`), so this can neither
/// escalate past the owner nor compose axes across grants the grantors did not
/// jointly authorize. `None` when there are no candidates.
pub(crate) fn most_permissive(mut candidates: Vec<GrantWarrant>) -> Option<WarrantSpec> {
    candidates.sort_by(|x, y| x.leaf.as_bytes().cmp(y.leaf.as_bytes()));
    let mut best: Option<&GrantWarrant> = None;
    for c in &candidates {
        best = match best {
            None => Some(c),
            // `best ⊆ c` ⇒ c is at least as permissive ⇒ prefer c. Otherwise keep
            // best (it has the smaller GrantId, since `candidates` is sorted).
            Some(b) if warrant_within(b.warrant(), c.warrant()) => Some(c),
            Some(b) => Some(b),
        };
    }
    best.map(|gw| gw.warrant.clone())
}

/// The grant-ledger seam — backend-agnostic (in-memory now; durable / cloud
/// behind the same trait, D94). A SEPARATE TRUTH from the journal; it never
/// writes the journal. Authorization is computed by the fold, never trusted from
/// a fact.
pub trait GrantLedger {
    /// Bind an asset to an owner (genesis). Idempotent on an identical binding;
    /// [`LedgerError::OwnerConflict`] if the asset is already bound to a
    /// different owner.
    ///
    /// # Errors
    ///
    /// [`LedgerError::OwnerConflict`] on a rebind to a different owner.
    fn append_binding(&self, binding: AssetBinding) -> Result<AppendOutcome, LedgerError>;

    /// Append a grant. Minimal + idempotent (authority is the fold's business).
    ///
    /// # Errors
    ///
    /// [`LedgerError::ImmutabilityConflict`] only on the cryptographically
    /// unreachable same-id-different-bytes tripwire.
    fn append_grant(&self, grant: Grant) -> Result<AppendOutcome, LedgerError>;

    /// Append a revocation (revoke by new fact). Minimal + idempotent; the
    /// revoker's authority is decided by the fold.
    ///
    /// # Errors
    ///
    /// [`LedgerError::ImmutabilityConflict`] only on the same tripwire.
    fn append_revocation(&self, revocation: Revocation) -> Result<AppendOutcome, LedgerError>;

    /// The bound owner of `asset`, if any.
    fn owner_of(&self, asset: &AssetRef) -> Option<PartyId>;

    /// The catalog actions `party` effectively holds on `asset` (the fail-closed,
    /// chain-narrowed, revocation-honoring fold). Total — never errors (the
    /// actions-only fold never invokes warrant narrowing).
    fn effective_grants(&self, party: &PartyId, asset: &AssetRef) -> EffectiveGrants;

    /// Every active grant chain `party` holds on `asset`, each bundling the
    /// actions it conveys WITH the runtime warrant under that chain (folded
    /// against `owner_root`, the owner's base warrant). The transparency API a
    /// dispatch layer (M7.3) inspects.
    ///
    /// # Errors
    ///
    /// Propagates [`NarrowingError`] if any active chain proposes a runtime-scope
    /// widen (a grant can never widen the granting party's warrant).
    fn effective_grant_warrants(
        &self,
        party: &PartyId,
        asset: &AssetRef,
        owner_root: &WarrantSpec,
    ) -> Result<Vec<GrantWarrant>, NarrowingError>;

    /// `true` iff `party` holds `action` on `asset` (the union of all active
    /// chains). The default consults [`GrantLedger::effective_grants`].
    fn is_authorized(&self, party: &PartyId, asset: &AssetRef, action: CatalogAction) -> bool {
        self.effective_grants(party, asset)
            .actions()
            .contains(action)
    }

    /// The runtime warrant a `Use`-style invocation of `action` runs under — the
    /// most-permissive REAL warrant among the active chains that ACTUALLY convey
    /// `action`. `Ok(None)` if no active chain conveys `action` (even when other
    /// chains convey other actions) — fail-closed + action-aligned.
    ///
    /// # Errors
    ///
    /// Propagates [`NarrowingError`] from the chain fold.
    fn resolve_effective_warrant_for(
        &self,
        party: &PartyId,
        asset: &AssetRef,
        action: CatalogAction,
        owner_root: &WarrantSpec,
    ) -> Result<Option<WarrantSpec>, NarrowingError> {
        let candidates: Vec<GrantWarrant> = self
            .effective_grant_warrants(party, asset, owner_root)?
            .into_iter()
            .filter(|gw| gw.conveys(action))
            .collect();
        Ok(most_permissive(candidates))
    }

    /// The most-permissive REAL warrant across ALL active chains, action-agnostic
    /// (a convenience for callers that do not key on an action). Prefer
    /// [`GrantLedger::resolve_effective_warrant_for`] when the action matters.
    ///
    /// # Errors
    ///
    /// Propagates [`NarrowingError`] from the chain fold.
    fn resolve_effective_warrant(
        &self,
        party: &PartyId,
        asset: &AssetRef,
        owner_root: &WarrantSpec,
    ) -> Result<Option<WarrantSpec>, NarrowingError> {
        Ok(most_permissive(
            self.effective_grant_warrants(party, asset, owner_root)?,
        ))
    }

    /// Enumerate every appended fact in append order.
    fn list_facts<'a>(&'a self) -> Box<dyn Iterator<Item = LedgerFact> + 'a>;

    /// Count of appended facts.
    fn len(&self) -> usize;

    /// `true` when no facts are appended.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<L: GrantLedger + ?Sized> GrantLedger for Arc<L> {
    fn append_binding(&self, binding: AssetBinding) -> Result<AppendOutcome, LedgerError> {
        (**self).append_binding(binding)
    }

    fn append_grant(&self, grant: Grant) -> Result<AppendOutcome, LedgerError> {
        (**self).append_grant(grant)
    }

    fn append_revocation(&self, revocation: Revocation) -> Result<AppendOutcome, LedgerError> {
        (**self).append_revocation(revocation)
    }

    fn owner_of(&self, asset: &AssetRef) -> Option<PartyId> {
        (**self).owner_of(asset)
    }

    fn effective_grants(&self, party: &PartyId, asset: &AssetRef) -> EffectiveGrants {
        (**self).effective_grants(party, asset)
    }

    fn effective_grant_warrants(
        &self,
        party: &PartyId,
        asset: &AssetRef,
        owner_root: &WarrantSpec,
    ) -> Result<Vec<GrantWarrant>, NarrowingError> {
        (**self).effective_grant_warrants(party, asset, owner_root)
    }

    fn list_facts<'a>(&'a self) -> Box<dyn Iterator<Item = LedgerFact> + 'a> {
        (**self).list_facts()
    }

    fn len(&self) -> usize {
        (**self).len()
    }
}
