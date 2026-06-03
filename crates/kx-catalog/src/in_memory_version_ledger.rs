// SPDX-License-Identifier: Apache-2.0
//! [`InMemoryVersionLedger`] — the reference [`VersionLedger`] backend.
//!
//! An append-only `Vec<AssetVersion>` truth + derived `BTreeMap` indices under a
//! single [`RwLock`]: O(log n) publish + resolve, O(depth) lineage, O(subtree)
//! descendants — sub-linear at scale, deterministic. Process-local + rebuildable —
//! not for production durability (a persistent backend implements the same trait,
//! D94). It proves [`VersionLedger`] carries no storage-substrate assumption (the
//! role [`crate::InMemoryGrantLedger`] plays for [`crate::GrantLedger`]).
//!
//! ## The handle move (D-LOCK-4 "update")
//!
//! Publishing a version pushes an immutable fact and, iff it ranks ahead of the
//! current latest, MOVES the handle (`by_path_latest`). `rank = (revision,
//! version_id-bytes)` is a total, deterministic, insertion-order-independent order,
//! so a rebuild from the append-only log yields the same resolution regardless of
//! append order — and a rollback (a higher-revision publish pinning OLDER content)
//! correctly moves the handle while retaining every prior version.
//!
//! ## The folds
//!
//! Lineage is an iterative, depth-bounded, cycle-safe, **integrity-checked** walk
//! of the `prior` edges ([`fold_lineage`]); forward lineage is a bounded BFS
//! ([`fold_descendants`], the `kx_projection::transitive_consumers` pattern,
//! replicated WITHOUT the dependency). Both are COMPUTED not stored and ADVISORY
//! (D84) — they never gate. No recursion ⇒ a pathological chain caps WORK, never
//! the stack.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::RwLock;

use crate::path::AssetPath;
use crate::version::{AssetVersion, VersionId, VersionedContent};
use crate::version_ledger::{
    PublishOutcome, VersionLedger, VersionLedgerError, MAX_VERSION_CHAIN_DEPTH,
    MAX_VERSION_DESCENDANTS,
};

/// A 32-byte BLAKE3 hash.
type Hash32 = [u8; 32];

/// The append-only truth + derived indices.
#[derive(Debug, Default)]
struct Inner {
    /// The append-only version log (the truth; everything else is a derived index).
    versions: Vec<AssetVersion>,
    /// Content id → position in `versions` (idempotency + immutability tripwire).
    by_id: BTreeMap<VersionId, usize>,
    /// THE MUTABLE HANDLE: path → the latest (highest-rank) version published on it.
    by_path_latest: BTreeMap<AssetPath, VersionId>,
    /// Reverse adjacency for forward lineage: version → its direct children (those
    /// whose `prior` == it). Built incrementally on publish — `descendants` is then
    /// O(subtree), never an O(n) scan.
    children: BTreeMap<VersionId, Vec<VersionId>>,
}

/// An ephemeral, process-local [`VersionLedger`]. Multiple readers, one writer.
///
/// # Examples
///
/// ```
/// use kx_catalog::{
///     AssetPath, AssetVersion, InMemoryVersionLedger, PartyId, Provenance,
///     TaskSignatureHash, VersionLedger, VersionedContent,
/// };
///
/// let ledger = InMemoryVersionLedger::new();
/// let handle = AssetPath::new("acme", "recipes", "summarize").unwrap();
/// let alice = PartyId::new("alice@acme");
///
/// // v1 — first publish of the handle.
/// let v1 = AssetVersion::root(
///     handle.clone(),
///     VersionedContent::Recipe(TaskSignatureHash::from_bytes([1u8; 32])),
///     alice.clone(),
///     Provenance::from_recipe([1u8; 32]),
/// );
/// let v1_id = ledger.publish(v1).unwrap().version_id();
/// assert_eq!(ledger.resolve(&handle).unwrap().1, v1_id);
///
/// // v2 — "update" = publish a NEW version + move the handle (D-LOCK-4).
/// let v2 = AssetVersion::successor(
///     v1_id, 0, handle.clone(),
///     VersionedContent::Recipe(TaskSignatureHash::from_bytes([2u8; 32])),
///     alice, Provenance::from_recipe([2u8; 32]),
/// );
/// let v2_id = ledger.publish(v2).unwrap().version_id();
/// assert_eq!(ledger.resolve(&handle).unwrap().1, v2_id); // handle moved to v2
/// assert!(ledger.get_version(&v1_id).is_some());          // v1 retained forever
/// assert_eq!(ledger.lineage(&v2_id).len(), 2);            // v2 -> v1
/// ```
#[derive(Debug, Default)]
pub struct InMemoryVersionLedger {
    inner: RwLock<Inner>,
}

impl InMemoryVersionLedger {
    /// Construct an empty ledger.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Look up a version by id, borrowing it from the log.
fn version_at<'a>(inner: &'a Inner, id: &VersionId) -> Option<&'a AssetVersion> {
    inner.by_id.get(id).map(|&pos| &inner.versions[pos])
}

/// `true` iff `child`'s `prior` edge to `parent` is a valid lineage edge: the
/// child names `parent` as its prior, both are on the SAME handle, and the child's
/// revision strictly advances the parent's. The publish path enforces this at
/// write time (so a stored chain is always well-formed); the folds re-apply it as
/// defense-in-depth, so even a corrupt / non-validating backend cannot make the
/// advisory forward (descendants) and backward (lineage) views disagree.
fn is_valid_edge(child: &AssetVersion, parent: &AssetVersion, parent_id: VersionId) -> bool {
    child.prior() == Some(parent_id)
        && child.handle() == parent.handle()
        && child.revision() > parent.revision()
}

/// Walk a version's ancestor chain (latest → oldest), bounded + cycle/missing
/// guarded + lineage-integrity-checked. A node whose `prior` references a missing
/// version, a DIFFERENT handle, or a non-decreasing revision is INCLUDED (it is a
/// real published fact) but the walk STOPS there (a forged/foreign graft conveys
/// no further ancestry — the provenance analog of "a forged grant conveys
/// nothing"). Over-depth returns the bounded prefix (ADVISORY → cannot escalate).
fn fold_lineage(inner: &Inner, start: VersionId) -> Vec<AssetVersion> {
    let mut chain: Vec<AssetVersion> = Vec::new();
    let mut seen: BTreeSet<VersionId> = BTreeSet::new();
    let mut cur = Some(start);
    while let Some(id) = cur {
        if chain.len() >= MAX_VERSION_CHAIN_DEPTH {
            break; // depth cap → bounded prefix
        }
        if !seen.insert(id) {
            break; // cycle → fail-closed (stop)
        }
        let Some(v) = version_at(inner, &id) else {
            break; // missing → fail-closed (stop)
        };
        // Lineage integrity decides whether the walk continues PAST this node
        // (defense-in-depth: publish already refuses a malformed edge, so on the
        // in-memory backend this only ever stops at a root).
        let continue_to = match v.prior() {
            None => None, // root: the chain ends here
            Some(parent_id) => match version_at(inner, &parent_id) {
                Some(p) if is_valid_edge(v, p, parent_id) => Some(parent_id),
                // missing parent / foreign graft / revision inversion → stop here.
                _ => None,
            },
        };
        chain.push(v.clone());
        cur = continue_to;
    }
    chain
}

/// Forward-lineage BFS: every version transitively descending from `root` via the
/// `children` index, applying the SAME [`is_valid_edge`] integrity check as
/// [`fold_lineage`] so the forward and backward views agree on what edges are real.
/// `visited` makes it cycle-safe; [`MAX_VERSION_DESCENDANTS`] bounds the work.
/// `root` itself is not a descendant. The emit order is BFS-over-publish-order and
/// is NOT a stable guarantee (it is advisory, never hashed).
fn fold_descendants(inner: &Inner, root: VersionId) -> Vec<VersionId> {
    let mut visited: BTreeSet<VersionId> = BTreeSet::new();
    let mut order: Vec<VersionId> = Vec::new();
    let mut queue: VecDeque<VersionId> = VecDeque::new();
    visited.insert(root); // mark (not emitted) so a cycle back to root terminates
    queue.push_back(root);
    while let Some(cur) = queue.pop_front() {
        if order.len() >= MAX_VERSION_DESCENDANTS {
            break; // work/output bound
        }
        let Some(cur_v) = version_at(inner, &cur) else {
            continue;
        };
        if let Some(kids) = inner.children.get(&cur) {
            for &child in kids {
                // Only follow a child that is a valid same-handle, strictly-
                // advancing successor of `cur` (so a forged cross-handle graft is
                // excluded forward, exactly as fold_lineage excludes it backward).
                let valid = version_at(inner, &child).is_some_and(|c| is_valid_edge(c, cur_v, cur));
                if valid && visited.insert(child) {
                    order.push(child);
                    queue.push_back(child);
                }
            }
        }
    }
    order
}

impl VersionLedger for InMemoryVersionLedger {
    fn publish(&self, version: AssetVersion) -> Result<PublishOutcome, VersionLedgerError> {
        let vid = version.version_id();
        let mut guard = self.inner.write().expect("poisoned lock");
        if let Some(&pos) = guard.by_id.get(&vid) {
            return if guard.versions[pos] == version {
                Ok(PublishOutcome::AlreadyPresent(vid))
            } else {
                Err(VersionLedgerError::ImmutabilityConflict(vid.to_hex()))
            };
        }
        // Lineage validation (fail-closed at the door): a successor's prior must be
        // present, on the SAME handle, and exactly one revision below. This keeps
        // the stored chain well-formed — a forged cross-handle graft or an inflated
        // `revision` can never land, so the handle-move rank cannot be gamed and the
        // forward/backward folds always agree.
        if let Some(pid) = version.prior() {
            let Some(parent) = version_at(&guard, &pid) else {
                return Err(VersionLedgerError::PriorNotFound(pid.to_hex()));
            };
            if parent.handle() != version.handle() {
                return Err(VersionLedgerError::InvalidLineage {
                    version_id: vid.to_hex(),
                    reason: format!("prior {pid} is on a different handle"),
                });
            }
            if parent.revision().checked_add(1) != Some(version.revision()) {
                return Err(VersionLedgerError::InvalidLineage {
                    version_id: vid.to_hex(),
                    reason: format!(
                        "revision {} is not prior.revision ({}) + 1",
                        version.revision(),
                        parent.revision()
                    ),
                });
            }
        }
        // Decide the handle move from a READ of current state (before any mutation).
        // rank = (revision, id-bytes): a total, deterministic, order-independent order.
        let new_rank = (version.revision(), *vid.as_bytes());
        let move_handle = match guard.by_path_latest.get(version.handle()) {
            None => true,
            Some(cur_vid) => {
                let cur_rank: (u32, Hash32) =
                    guard.by_id.get(cur_vid).map_or((0u32, [0u8; 32]), |&cpos| {
                        (guard.versions[cpos].revision(), *cur_vid.as_bytes())
                    });
                new_rank > cur_rank
            }
        };
        let handle = version.handle().clone();
        let parent = version.prior();
        let pos = guard.versions.len();
        guard.versions.push(version);
        guard.by_id.insert(vid, pos);
        if let Some(parent) = parent {
            guard.children.entry(parent).or_default().push(vid);
        }
        if move_handle {
            guard.by_path_latest.insert(handle, vid);
        }
        Ok(PublishOutcome::Published(vid))
    }

    fn resolve(&self, handle: &AssetPath) -> Option<(VersionedContent, VersionId)> {
        let guard = self.inner.read().expect("poisoned lock");
        let vid = *guard.by_path_latest.get(handle)?;
        let v = version_at(&guard, &vid)?;
        Some((*v.content(), vid))
    }

    fn get_version(&self, id: &VersionId) -> Option<AssetVersion> {
        let guard = self.inner.read().expect("poisoned lock");
        version_at(&guard, id).cloned()
    }

    fn lineage(&self, id: &VersionId) -> Vec<AssetVersion> {
        let guard = self.inner.read().expect("poisoned lock");
        fold_lineage(&guard, *id)
    }

    fn descendants(&self, id: &VersionId) -> Vec<VersionId> {
        let guard = self.inner.read().expect("poisoned lock");
        fold_descendants(&guard, *id)
    }

    fn list_versions<'a>(&'a self) -> Box<dyn Iterator<Item = AssetVersion> + 'a> {
        let guard = self.inner.read().expect("poisoned lock");
        // Snapshot under the read lock (append order), then release before iterating.
        let versions: Vec<AssetVersion> = guard.versions.clone();
        Box::new(versions.into_iter())
    }

    fn len(&self) -> usize {
        self.inner.read().expect("poisoned lock").versions.len()
    }
}

// Compile-time proof the ledger is shareable across threads (so `Arc<…>` works
// for the concurrency tests + a multi-threaded gateway).
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<InMemoryVersionLedger>();
};
