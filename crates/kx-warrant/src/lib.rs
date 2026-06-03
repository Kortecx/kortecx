// SPDX-License-Identifier: Apache-2.0
//! `kx-warrant` — the runtime-enforced capability layer (D30).
//!
//! A **Warrant** is the scoped capability boundary a Mote executes under: a
//! runtime-ENFORCED FACT, typed and structured (never prose), re-derivable
//! bit-for-bit on recovery. A **Role** is a named, versioned, content-addressed
//! `WarrantSpec` template — the RBAC surface.
//!
//! The load-bearing invariant of this crate is **monotonic narrowing**:
//!
//! ```text
//! child.warrant = intersect(parent.warrant, child.role)
//! ```
//!
//! - The **runtime ENFORCES** the intersection; the **model PROPOSES** which
//!   role to assume and may narrow within it.
//! - The model **never authorizes a widen** on any axis. Widening is a typed
//!   error (`NarrowingError::AttemptedWiden`).
//! - The intersection function is **PURE**: same inputs → byte-identical
//!   output. No I/O, no clock, no journal access. Recovery re-derives warrants
//!   bit-for-bit (machine-independent).
//!
//! # Seven narrowable axes (qualitative — widening rejected as typed error)
//!
//! | Axis                  | Semantics                                              |
//! |-----------------------|--------------------------------------------------------|
//! | `fs_scope`            | path-set intersection; per-path mode min-bound         |
//! | `net_scope`           | egress allowlist subset; `None` blocks all egress      |
//! | `syscall_profile_ref` | opaque content-ref; subset check deferred to compiler  |
//! | `tool_grants`         | set subset on `(ToolName, ToolVersion)`                |
//! | `secret_scope`        | secret-ref allowlist subset; `None` authorizes none (D110.3) |
//! | `executor_class`      | set by child's role; not narrowed from parent          |
//! | `environment_ref`     | set by child's role; not narrowed from parent          |
//!
//! # Quantitative axes (narrowed silently via `min()`)
//!
//! - `resource_ceiling.*` — cpu_milli, mem_bytes, wall_clock_ms, fd_count, disk_bytes.
//! - `model_route.max_input_tokens` / `max_output_tokens` / `max_calls`.
//! - `cost_ceiling.micro_usd` — dollar ceiling (D115; axis reserved at M5.3,
//!   spend enforcement at M11).
//!
//! # `tls_required` is tighten-only (narrowed via `||`)
//!
//! A child can add a TLS requirement but never relax a parent's (D118.5).
//!
//! # `mote_class` and `nd_class` are set by child's role (NOT inherited).
//!
//! A child may be `Pure` under a `WorldMutating` parent (workers may choose to
//! be tighter). The intersection function leaves these fields as the child's
//! declared value without narrowing.
//!
//! # Content-addressed identity
//!
//! ```text
//! warrant_ref = blake3(canonical_bincode(WarrantSpec))
//! ```
//!
//! Two semantically-identical warrants produce byte-identical refs;
//! identity-bearing. See [`warrant_ref_of`].
//!
//! # Reading further
//!
//! - `docs/design/warrant.md` (private corpus) — the locked spec for D30.
//! - `docs/design/decisions.md` D30, D32, D33, D35, D36 — interlocking decisions.
//! - `05-progress-tracker.md` SN-8 — *model proposes, runtime enforces*.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
// TODO(workspace.lints cleanup): kx-warrant uses `.expect()` on
// canonical-bincode encode (documented infallible) for `warrant_ref_of`.
// Follow-up cleanup PR migrates to typed error or extracts the encode
// call to a shared helper that returns Result. Until then, the documented
// `expect(...)` is the audit trail.
#![allow(clippy::expect_used)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown,
    clippy::return_self_not_must_use,
    clippy::needless_pass_by_value
)]
// Inline test modules are exempted from the workspace deny on `unwrap_used` /
// `expect_used`. Integration tests under tests/*.rs carry per-file allows.
#![cfg_attr(test, allow(clippy::unwrap_used))]

mod check;
mod classes;
mod errors;
mod fields;
mod narrow;
mod refs;
mod registry;
mod scope;
mod secret;
mod spec;

pub use check::check_tool_requirement;
pub use classes::{ExecutorClass, FsMode, MoteClass};
pub use errors::{NarrowingError, ToolDenied};
pub use fields::{Host, WarrantField};
pub use narrow::{intersect, narrow};
pub use refs::{role_id_of, warrant_ref_of};
pub use registry::{InMemoryRoleRegistry, RoleRegistry};
pub use scope::{FsScope, NetScope};
pub use secret::{SecretRef, SecretScope};
pub use spec::{
    CostCeiling, ModelRoute, ResourceCeiling, Role, ToolGrant, ToolRequirement, WarrantSpec,
};

#[cfg(test)]
mod tests;
