// SPDX-License-Identifier: Apache-2.0
//! [`InMemoryGrantLedger`] — the reference [`GrantLedger`] backend.
//!
//! An append-only `Vec<LedgerFact>` truth + derived `BTreeMap` indices under a
//! single [`RwLock`]: O(log n) append + per-query lookup, sub-linear at scale,
//! deterministic. Process-local + rebuildable — not for production durability (a
//! persistent backend implements the same trait, D94). It proves [`GrantLedger`]
//! carries no storage-substrate assumption (the role `InMemoryCatalog` plays for
//! [`crate::CatalogRegistry`]).
//!
//! ## The fold
//!
//! Authorization is an iterative, depth-bounded, single-pass fold over a grant's
//! delegation chain ([`fold_chain`]): collect leaf→root bounded by
//! [`MAX_DELEGATION_DEPTH`] (cycle / missing / over-depth → fail-closed), then
//! fold root→leaf narrowing actions (set-intersection) and the runtime warrant
//! (the FROZEN `kx_warrant::intersect`), honoring authorized revocations at every
//! hop. No recursion ⇒ a pathologically deep chain caps WORK, never the stack.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::RwLock;

use kx_warrant::{intersect, NarrowingError, WarrantSpec};

use crate::action::CatalogActionSet;
use crate::grant::{Grant, GrantId};
use crate::ledger::{
    AppendOutcome, AssetBinding, EffectiveGrants, GrantLedger, GrantWarrant, LedgerError,
    LedgerFact, MAX_DELEGATION_DEPTH,
};
use crate::party::PartyId;
use crate::path::AssetRef;

/// The append-only truth + derived indices.
///
/// `pub(crate)` so the durable [`crate::SqliteGrantLedger`] holds the SAME `Inner`
/// and shares the SAME fold/read/apply logic (no in-memory-vs-replayed divergence
/// by construction). Fields stay private to this module; cross-module callers use
/// [`Inner::apply_fact`] (write) + the `pub(crate)` read functions below.
#[derive(Debug, Default)]
pub(crate) struct Inner {
    /// The append-only fact log (the truth; everything else is a derived index).
    facts: Vec<LedgerFact>,
    /// Content id → position in `facts` (idempotency + immutability tripwire).
    by_id: BTreeMap<crate::ledger::FactId, usize>,
    /// Asset → owner (genesis bindings).
    bindings: BTreeMap<AssetRef, PartyId>,
    /// Grant id → position in `facts` (O(log n) grant lookup for the fold).
    grants: BTreeMap<GrantId, usize>,
    /// (grantee, asset) → the leaf grant ids that party holds on that asset.
    grants_by_grantee_asset: BTreeMap<(PartyId, AssetRef), Vec<GrantId>>,
    /// Grant id → the parties that have recorded a revocation of it. The fold
    /// filters to AUTHORIZED revokers (grantor or owner); recording is unchecked.
    revoked: BTreeMap<GrantId, BTreeSet<PartyId>>,
}

impl Inner {
    /// Apply an already-validated, non-duplicate fact: assign it the next append
    /// position, update the derived indices, and push it onto the log. The SINGLE
    /// fold step — used by BOTH the in-memory append (after its conflict/dedup
    /// gate) and the durable rebuild (replaying the persisted log in `seq` order),
    /// so the two backends can never diverge.
    pub(crate) fn apply_fact(&mut self, fact: LedgerFact) {
        let pos = self.facts.len();
        let fid = fact.fact_id();
        match &fact {
            LedgerFact::Bind(b) => {
                self.bindings.insert(b.asset().clone(), b.owner().clone());
            }
            LedgerFact::Grant(g) => {
                let gid = g.grant_id();
                self.grants.insert(gid, pos);
                self.grants_by_grantee_asset
                    .entry((g.grantee().clone(), g.asset().clone()))
                    .or_default()
                    .push(gid);
            }
            LedgerFact::Revoke(r) => {
                self.revoked
                    .entry(r.grant_id())
                    .or_default()
                    .insert(r.revoker().clone());
            }
        }
        self.by_id.insert(fid, pos);
        self.facts.push(fact);
    }

    /// `true` iff a fact with this content id is already present (the
    /// idempotency/immutability tripwire the durable append consults under its
    /// transaction).
    pub(crate) fn contains_fact(&self, fid: &crate::ledger::FactId) -> Option<&LedgerFact> {
        self.by_id.get(fid).map(|&pos| &self.facts[pos])
    }

    /// The bound owner of `asset`, if any (the owner-conflict gate for bindings).
    pub(crate) fn owner_of_asset(&self, asset: &AssetRef) -> Option<&PartyId> {
        self.bindings.get(asset)
    }

    /// The count of appended facts.
    pub(crate) fn len_facts(&self) -> usize {
        self.facts.len()
    }
}

/// An ephemeral, process-local [`GrantLedger`]. Multiple readers, one writer.
///
/// # Examples
///
/// ```
/// use kx_catalog::{
///     AssetBinding, AssetPath, AssetRef, CatalogAction, CatalogActionSet, Grant,
///     GrantLedger, InMemoryGrantLedger, PartyId,
/// };
/// use kx_warrant::{Role, WarrantSpec};
///
/// let ledger = InMemoryGrantLedger::new();
/// let asset = AssetRef::Path(AssetPath::new("acme", "research", "lit-review").unwrap());
/// let owner = PartyId::new("admin@acme");
/// let mate = PartyId::new("teammate@acme");
///
/// ledger.append_binding(AssetBinding::new(asset.clone(), owner.clone())).unwrap();
/// assert_eq!(ledger.owner_of(&asset), Some(owner.clone()));
///
/// // The owner's base warrant; a role that does not widen it.
/// let owner_root = WarrantSpec::default();
/// let role = Role { name: "read-only".into(), version: 1, spec: WarrantSpec::default(), description: String::new() };
/// let g = Grant::root(asset.clone(), owner, mate.clone(),
///     CatalogActionSet::allow([CatalogAction::Read, CatalogAction::Use]), role);
/// ledger.append_grant(g).unwrap();
///
/// assert!(ledger.is_authorized(&mate, &asset, CatalogAction::Use));
/// assert!(!ledger.is_authorized(&mate, &asset, CatalogAction::Delegate));
/// ```
#[derive(Debug, Default)]
pub struct InMemoryGrantLedger {
    inner: RwLock<Inner>,
}

impl InMemoryGrantLedger {
    /// Construct an empty ledger.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Look up a grant by id, borrowing it from the fact log.
pub(crate) fn grant_at<'a>(inner: &'a Inner, gid: &GrantId) -> Option<&'a Grant> {
    inner
        .grants
        .get(gid)
        .and_then(|&pos| match &inner.facts[pos] {
            LedgerFact::Grant(g) => Some(g.as_ref()),
            _ => None,
        })
}

/// `true` iff `gid` has an AUTHORIZED revocation: a revoker that is the grant's
/// grantor (you may revoke what you granted) or the asset owner (an owner may
/// revoke any grant on their asset). An unauthorized party's recorded revocation
/// conveys nothing.
pub(crate) fn is_revoked(
    inner: &Inner,
    gid: &GrantId,
    grant: &Grant,
    owner: Option<&PartyId>,
) -> bool {
    match inner.revoked.get(gid) {
        None => false,
        Some(revokers) => revokers
            .iter()
            .any(|r| r == grant.grantor() || owner == Some(r)),
    }
}

/// The folded result of one delegation chain.
pub(crate) struct ChainFold {
    actions: CatalogActionSet,
    /// `Some` iff `owner_root` was supplied (the warrant-bearing fold); `None`
    /// for the actions-only fold (which never invokes warrant narrowing, so it
    /// is total / infallible).
    warrant: Option<WarrantSpec>,
}

/// Fold a single delegation chain identified by its `leaf` grant id.
///
/// `Ok(None)` = the chain conveys nothing (fail-closed: missing/cyclic/over-deep
/// chain, an authorized revocation anywhere, a broken delegation link, a
/// delegator lacking `Delegate`, or a root grant not from the asset owner).
/// `Err` = a runtime-scope widen surfaced by `kx_warrant::intersect` (only
/// possible when `owner_root` is `Some`).
pub(crate) fn fold_chain(
    inner: &Inner,
    leaf: &GrantId,
    owner: Option<&PartyId>,
    owner_root: Option<&WarrantSpec>,
) -> Result<Option<ChainFold>, NarrowingError> {
    // Phase 1 — collect leaf→root, bounded + cycle/missing guarded (fail-closed).
    let mut chain: Vec<&Grant> = Vec::new();
    let mut seen: BTreeSet<GrantId> = BTreeSet::new();
    let mut cur = Some(*leaf);
    while let Some(id) = cur {
        if chain.len() >= MAX_DELEGATION_DEPTH {
            return Ok(None); // over-depth → fail-closed
        }
        if !seen.insert(id) {
            return Ok(None); // cycle → fail-closed
        }
        let Some(g) = grant_at(inner, &id) else {
            return Ok(None); // missing grant → fail-closed
        };
        chain.push(g);
        cur = g.prior();
    }

    // Phase 2 — fold root→leaf, narrowing actions + warrant at each hop.
    let mut parent_actions = CatalogActionSet::all(); // owner's full authority above the root
    let mut parent_warrant = owner_root.cloned();
    let mut parent: Option<&Grant> = None;
    for g in chain.iter().rev() {
        let gid = g.grant_id();
        if is_revoked(inner, &gid, g, owner) {
            return Ok(None); // an authorized revocation cascades to the whole subtree
        }
        match g.prior() {
            Some(_) => {
                // Delegated hop: the parent must convey Delegate, the parent's
                // grantee must be this grant's grantor, and both on the same asset.
                let Some(p) = parent else {
                    return Ok(None); // dangling delegated grant (no in-chain parent)
                };
                if p.grantee() != g.grantor() || p.asset() != g.asset() {
                    return Ok(None); // broken delegation link
                }
                if !parent_actions.contains(crate::action::CatalogAction::Delegate) {
                    return Ok(None); // delegator did not hold Delegate
                }
            }
            None => {
                // Root hop: the grantor must be the asset's bound owner.
                if owner != Some(g.grantor()) {
                    return Ok(None);
                }
            }
        }
        parent_actions = g.actions().narrow(&parent_actions);
        parent_warrant = match parent_warrant {
            Some(pw) => Some(intersect(&pw, g.runtime_scope())?),
            None => None,
        };
        parent = Some(g);
    }

    Ok(Some(ChainFold {
        actions: parent_actions,
        warrant: parent_warrant,
    }))
}

// ---------------------------------------------------------------------------
// Shared read folds (pub(crate)) — the in-memory AND the durable
// `SqliteGrantLedger` trait impls call these over their respective `Inner`, so
// the read semantics are ONE source of truth (no backend divergence).
// ---------------------------------------------------------------------------

/// The bound owner of `asset`, if any.
pub(crate) fn read_owner_of(inner: &Inner, asset: &AssetRef) -> Option<PartyId> {
    inner.bindings.get(asset).cloned()
}

/// The catalog actions `party` effectively holds on `asset` (the fail-closed
/// chain-narrowed, revocation-honoring fold).
pub(crate) fn read_effective_grants(
    inner: &Inner,
    party: &PartyId,
    asset: &AssetRef,
) -> EffectiveGrants {
    let owner = inner.bindings.get(asset);
    let Some(gids) = inner
        .grants_by_grantee_asset
        .get(&(party.clone(), asset.clone()))
    else {
        return EffectiveGrants::default();
    };
    let mut per_grant: Vec<(GrantId, CatalogActionSet)> = Vec::new();
    for gid in gids {
        // Actions-only fold (owner_root = None): never invokes warrant
        // narrowing, so it cannot error — an Err is structurally impossible.
        if let Ok(Some(cf)) = fold_chain(inner, gid, owner, None) {
            if !cf.actions.is_empty() {
                per_grant.push((*gid, cf.actions));
            }
        }
    }
    EffectiveGrants::from_parts(per_grant)
}

/// Every active grant chain `party` holds on `asset`, each bundling its actions
/// with the runtime warrant folded against `owner_root`.
pub(crate) fn read_effective_grant_warrants(
    inner: &Inner,
    party: &PartyId,
    asset: &AssetRef,
    owner_root: &WarrantSpec,
) -> Result<Vec<GrantWarrant>, NarrowingError> {
    let owner = inner.bindings.get(asset);
    let Some(gids) = inner
        .grants_by_grantee_asset
        .get(&(party.clone(), asset.clone()))
    else {
        return Ok(Vec::new());
    };
    let mut out: Vec<GrantWarrant> = Vec::new();
    for gid in gids {
        if let Some(cf) = fold_chain(inner, gid, owner, Some(owner_root))? {
            // owner_root is Some ⇒ the fold always yields Some(warrant).
            if let (false, Some(w)) = (cf.actions.is_empty(), cf.warrant) {
                out.push(GrantWarrant::new(*gid, cf.actions, w));
            }
        }
    }
    Ok(out)
}

/// A snapshot of the append-only fact log (append order).
pub(crate) fn snapshot_facts(inner: &Inner) -> Vec<LedgerFact> {
    inner.facts.clone()
}

impl GrantLedger for InMemoryGrantLedger {
    fn append_binding(&self, binding: AssetBinding) -> Result<AppendOutcome, LedgerError> {
        let fact = LedgerFact::Bind(binding.clone());
        let fid = fact.fact_id();
        let mut guard = self.inner.write().expect("poisoned lock");
        // Owner conflict takes precedence: an asset has exactly one owner.
        if let Some(existing) = guard.owner_of_asset(binding.asset()) {
            if existing != binding.owner() {
                return Err(LedgerError::OwnerConflict(format!(
                    "asset {} already bound to a different owner",
                    binding.asset()
                )));
            }
            return Ok(AppendOutcome::AlreadyPresent(fid)); // same owner → idempotent
        }
        guard.apply_fact(fact);
        Ok(AppendOutcome::Appended(fid))
    }

    fn append_grant(&self, grant: Grant) -> Result<AppendOutcome, LedgerError> {
        let fact = LedgerFact::Grant(Box::new(grant));
        let fid = fact.fact_id();
        let mut guard = self.inner.write().expect("poisoned lock");
        if let Some(existing) = guard.contains_fact(&fid) {
            return if *existing == fact {
                Ok(AppendOutcome::AlreadyPresent(fid))
            } else {
                Err(LedgerError::ImmutabilityConflict(fid.to_hex()))
            };
        }
        guard.apply_fact(fact);
        Ok(AppendOutcome::Appended(fid))
    }

    fn append_revocation(
        &self,
        revocation: crate::grant::Revocation,
    ) -> Result<AppendOutcome, LedgerError> {
        let fact = LedgerFact::Revoke(revocation);
        let fid = fact.fact_id();
        let mut guard = self.inner.write().expect("poisoned lock");
        if let Some(existing) = guard.contains_fact(&fid) {
            return if *existing == fact {
                Ok(AppendOutcome::AlreadyPresent(fid))
            } else {
                Err(LedgerError::ImmutabilityConflict(fid.to_hex()))
            };
        }
        guard.apply_fact(fact);
        Ok(AppendOutcome::Appended(fid))
    }

    fn owner_of(&self, asset: &AssetRef) -> Option<PartyId> {
        read_owner_of(&self.inner.read().expect("poisoned lock"), asset)
    }

    fn effective_grants(&self, party: &PartyId, asset: &AssetRef) -> EffectiveGrants {
        read_effective_grants(&self.inner.read().expect("poisoned lock"), party, asset)
    }

    fn effective_grant_warrants(
        &self,
        party: &PartyId,
        asset: &AssetRef,
        owner_root: &WarrantSpec,
    ) -> Result<Vec<GrantWarrant>, NarrowingError> {
        read_effective_grant_warrants(
            &self.inner.read().expect("poisoned lock"),
            party,
            asset,
            owner_root,
        )
    }

    fn list_facts<'a>(&'a self) -> Box<dyn Iterator<Item = LedgerFact> + 'a> {
        let facts = snapshot_facts(&self.inner.read().expect("poisoned lock"));
        Box::new(facts.into_iter())
    }

    fn len(&self) -> usize {
        self.inner.read().expect("poisoned lock").facts.len()
    }
}

// Compile-time proof the ledger is shareable across threads (so `Arc<…>` works
// for the concurrency tests + a multi-threaded gateway).
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<InMemoryGrantLedger>();
};
