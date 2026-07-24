//! The flag REGISTRY — every dark-launchable flag in the runtime, in one place.
//!
//! A flag is a `const` here and a `Flag::*` at the call site, so a misspelled flag
//! fails the build instead of silently reading `false`. Adding one is a two-line
//! change: a `const` below, and its name in [`Flag::ALL`].

/// One dark-launch flag: a default-OFF boolean the operator can turn on.
///
/// Construct these only as `const`s in this module — the registry is the point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flag {
    /// Stable identifier, `snake_case` (e.g. `serve_memory`). Never the env name.
    pub name: &'static str,
    /// The canonical env var: `KX_FLAG_<NAME>`, screaming-case. Highest precedence.
    pub env: &'static str,
    /// A pre-existing env var this flag adopted, kept working so migrating a
    /// shipped knob onto the seam breaks nobody. `env` wins if both are set.
    pub legacy_env: Option<&'static str>,
    /// Always `false`. The field exists to make the default explicit at the
    /// definition site (and to keep the resolver honest about what "unset" means)
    /// — `Flag::ALL` is tested to ensure every flag is default-OFF.
    pub default: bool,
}

impl Flag {
    /// The durable-MEMORY subsystem (`recall@1` / `remember@1`, the `react-memory`
    /// recipe, the memory RPCs). A per-principal state surface, so it stays opt-in:
    /// OFF ⇒ the memory RPCs honestly report `unimplemented` and no memory recipe
    /// is seeded, which is byte-identical to a build without the feature.
    pub const SERVE_MEMORY: Flag = Flag {
        name: "serve_memory",
        env: "KX_FLAG_SERVE_MEMORY",
        legacy_env: Some("KX_SERVE_MEMORY"),
        default: false,
    };

    /// The autonomous-loop tool AUTO-GRANT. ON ⇒ the `kx/recipes/react-auto`
    /// recipe is seeded and the binder rebuilds its warrant from the LIVE registry
    /// at bind time (the model may pick from every registered tool). OFF ⇒
    /// `react-auto` is not seeded — deny-by-default, a byte-identical serve.
    pub const SERVE_AUTOGRANT: Flag = Flag {
        name: "serve_autogrant",
        env: "KX_FLAG_SERVE_AUTOGRANT",
        legacy_env: Some("KX_SERVE_AUTOGRANT"),
        default: false,
    };

    /// The cross-run WORK CACHE (`kx-work-cache`). ON ⇒ the serve opens a
    /// `work-cache.db` sidecar and injects it so a PURE result computed once in any
    /// run is served (not recomputed) in every other run with the same
    /// `(mote_def_hash, input_data_id)`. OFF ⇒ the sidecar is never opened and the
    /// executor is handed `None`, so the run path and `ProjectionDigest` are
    /// byte-identical to a build without the feature. Never serves `WorldMutating` work.
    pub const SERVE_WORK_CACHE: Flag = Flag {
        name: "serve_work_cache",
        env: "KX_FLAG_SERVE_WORK_CACHE",
        legacy_env: None,
        default: false,
    };

    /// Every registered flag. Add new flags here — the registry invariants
    /// (default-OFF, unique names, `KX_FLAG_` prefix, no alias collisions) are
    /// property-tested across this slice, so a flag that is not listed is not
    /// covered by them.
    pub const ALL: &'static [Flag] = &[
        Self::SERVE_MEMORY,
        Self::SERVE_AUTOGRANT,
        Self::SERVE_WORK_CACHE,
    ];
}
