//! [`RoleRegistry`] — workflow-author-side lookup mapping a descriptor's
//! `RoleId` handle to a versioned [`Role`].
//!
//! The D48 child resolver picks the heavy `MoteDef` axes for a materialized
//! child; the **warrant** axis is independent — D30's monotonic narrowing
//! requires `intersect(parent.warrant, child.role.spec)`. Computing
//! `child.role.spec` needs a registry mapping the descriptor-side
//! `RoleId(String)` handle to a concrete [`Role`] (with its `WarrantSpec`).
//! This module ships the trait + the OSS-default in-memory impl.
//!
//! The closing of `topology.md` §13 KG-1 (the *shaper-spawned-child warrant
//! narrowing not wired* gap) consists of:
//! 1. This trait + the in-memory impl (in this module).
//! 2. `kx_projection::DefaultTopologyMaterializer` taking a
//!    `RoleRegistry` and calling [`crate::intersect`] in its
//!    `try_materialize` path instead of the verbatim ref-copy.
//!
//! See `docs/design/topology.md` §13 KG-1 + `docs/design/decisions.md`
//! §D30 / §D48 (private corpus) for the load-bearing properties.

use std::collections::BTreeMap;
use std::sync::RwLock;

use kx_mote::RoleId;

use crate::spec::Role;

/// Lookup of descriptor-side `RoleId` handles → versioned [`Role`]s.
///
/// The descriptor's [`RoleId`] is a workflow-author-facing string handle
/// (`kx_mote::RoleId(String)`); this trait resolves it to the concrete
/// [`Role`] (carrying the [`crate::WarrantSpec`] template) used by
/// `intersect(shaper.warrant, role)`.
///
/// MUST be deterministic over the lifetime of one workflow execution:
/// resolving the same `RoleId` twice MUST return identical `Role`s.
/// Replay-faithfulness rests on this — a non-deterministic registry
/// silently breaks R49 (a re-fold could materialize children with
/// different warrants than the live fold).
///
/// Object-safe + `Send + Sync` so the materializer can hold an
/// `Arc<dyn RoleRegistry>` and share it across worker threads.
pub trait RoleRegistry: Send + Sync {
    /// Resolve a role by its descriptor-side handle. Returns `None` if the
    /// role is not registered — the materializer turns this into a typed
    /// projection error so the workflow author sees a loud failure rather
    /// than silent warrant widening.
    fn resolve(&self, role_id: &RoleId) -> Option<Role>;
}

/// OSS-default [`RoleRegistry`] backed by an in-memory `BTreeMap`.
///
/// Workflow authors register roles before submitting a shaper. The OSS
/// runtime instantiates one of these per workflow and hands it to the
/// projection's materializer; cloud-side impls may back the same trait
/// with a content-addressed remote registry (see `decisions.md` §D48
/// cloud forward-note for the multi-model-resolution case).
///
/// # Examples
///
/// ```
/// use kx_mote::{ModelId, RoleId};
/// use kx_warrant::{
///     ExecutorClass, FsScope, InMemoryRoleRegistry, ModelRoute, MoteClass,
///     NetScope, ResourceCeiling, Role, RoleRegistry, WarrantSpec,
/// };
/// use kx_content::ContentRef;
/// use std::collections::BTreeSet;
///
/// let spec = WarrantSpec {
///     mote_class: MoteClass::Pure, nd_class: MoteClass::Pure,
///     fs_scope: FsScope::empty(), net_scope: NetScope::None,
///     syscall_profile_ref: ContentRef::from_bytes([0; 32]),
///     tool_grants: BTreeSet::new(),
///     model_route: ModelRoute {
///         model_id: ModelId("m".into()), max_input_tokens: 10,
///         max_output_tokens: 10, max_calls: 1,
///     },
///     resource_ceiling: ResourceCeiling {
///         cpu_milli: 1, mem_bytes: 1, wall_clock_ms: 1,
///         fd_count: 1, disk_bytes: 1,
///     },
///     environment_ref: None, executor_class: ExecutorClass::Bwrap,
/// };
/// let role = Role {
///     name: "worker".into(), version: 1, spec,
///     description: String::new(),
/// };
///
/// let reg = InMemoryRoleRegistry::new();
/// reg.register(RoleId("worker".into()), role.clone());
/// assert_eq!(reg.resolve(&RoleId("worker".into())), Some(role));
/// assert_eq!(reg.resolve(&RoleId("unknown".into())), None);
/// ```
#[derive(Default, Debug)]
pub struct InMemoryRoleRegistry {
    roles: RwLock<BTreeMap<RoleId, Role>>,
}

impl InMemoryRoleRegistry {
    /// Construct an empty registry.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a role under a descriptor-side handle. Overwriting the
    /// same handle with a different role is permitted but is a workflow-
    /// author smell — KG-1's determinism rests on the registered roles
    /// staying stable across a workflow's lifetime.
    pub fn register(&self, role_id: RoleId, role: Role) {
        self.roles
            .write()
            .expect("InMemoryRoleRegistry poisoned")
            .insert(role_id, role);
    }

    /// Number of registered roles. Useful for asserting setup in tests.
    #[must_use]
    pub fn len(&self) -> usize {
        self.roles
            .read()
            .expect("InMemoryRoleRegistry poisoned")
            .len()
    }

    /// `true` if no roles are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.roles
            .read()
            .expect("InMemoryRoleRegistry poisoned")
            .is_empty()
    }
}

impl RoleRegistry for InMemoryRoleRegistry {
    fn resolve(&self, role_id: &RoleId) -> Option<Role> {
        self.roles
            .read()
            .expect("InMemoryRoleRegistry poisoned")
            .get(role_id)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
    };
    use kx_content::ContentRef;
    use kx_mote::ModelId;
    use std::collections::BTreeSet;
    use std::sync::Arc;
    use std::thread;

    fn sample_role(name: &str) -> Role {
        let spec = WarrantSpec {
            mote_class: MoteClass::Pure,
            nd_class: MoteClass::Pure,
            fs_scope: FsScope::empty(),
            net_scope: NetScope::None,
            syscall_profile_ref: ContentRef::from_bytes([0; 32]),
            tool_grants: BTreeSet::new(),
            model_route: ModelRoute {
                model_id: ModelId("m".into()),
                max_input_tokens: 10,
                max_output_tokens: 10,
                max_calls: 1,
            },
            resource_ceiling: ResourceCeiling {
                cpu_milli: 1,
                mem_bytes: 1,
                wall_clock_ms: 1,
                fd_count: 1,
                disk_bytes: 1,
            },
            environment_ref: None,
            executor_class: ExecutorClass::Bwrap,
        };
        Role {
            name: name.into(),
            version: 1,
            spec,
            description: String::new(),
        }
    }

    #[test]
    fn registry_resolves_registered_handles_and_returns_none_for_unknown() {
        let reg = InMemoryRoleRegistry::new();
        assert!(reg.is_empty());
        let role = sample_role("worker");
        reg.register(RoleId("worker".into()), role.clone());
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.resolve(&RoleId("worker".into())), Some(role));
        assert_eq!(reg.resolve(&RoleId("nope".into())), None);
    }

    #[test]
    fn registry_repeated_resolve_returns_same_role_determinism() {
        let reg = InMemoryRoleRegistry::new();
        let role = sample_role("planner");
        reg.register(RoleId("planner".into()), role.clone());
        for _ in 0..10 {
            assert_eq!(reg.resolve(&RoleId("planner".into())), Some(role.clone()));
        }
    }

    #[test]
    fn registry_send_sync_compile_time() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<InMemoryRoleRegistry>();
        assert_send_sync::<Arc<dyn RoleRegistry>>();
    }

    #[test]
    fn registry_concurrent_readers_get_same_answer() {
        let reg = Arc::new(InMemoryRoleRegistry::new());
        reg.register(RoleId("r".into()), sample_role("r"));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let reg = Arc::clone(&reg);
                thread::spawn(move || reg.resolve(&RoleId("r".into())))
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let first = results[0].clone();
        assert!(first.is_some());
        for r in &results {
            assert_eq!(r, &first);
        }
    }
}
