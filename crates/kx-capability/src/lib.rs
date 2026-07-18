// SPDX-License-Identifier: Apache-2.0
//! `kx-capability` — D24 capability-broker seam (P1.8.5).
//!
//! The [`CapabilityBroker`] trait is the executor's sole interface to
//! effects. One implementation per deployment shape:
//!
//! - **P1.8.5 (this crate, OSS):** [`LocalCapabilityBroker`] — trivial
//!   in-process pass-through with content-store staging. Single-tenant.
//! - **P5 (cloud, `kx-cloud`):** hardened bubblewrap + seccomp +
//!   per-tenant isolation behind this **same trait** — never a fork.
//!
//! Per `capability-broker.md` §3, this crate has **no `kx-journal` or
//! `kx-projection` dependency** (verified by `cargo tree`): the
//! recovery-state-independence invariant is structurally enforced. The
//! broker does the per-call contract check, dispatches the effect, stages
//! the response payload into the [`ContentStore`][kx_content::ContentStore],
//! and returns a [`BrokerHandle`]. The executor decides what to commit;
//! the broker never writes the journal.
//!
//! # The per-call contract (D24 + D30 composition)
//!
//! [`CapabilityBroker::dispatch`] enforces, in this order:
//!
//! 1. The named capability is in `mote.def.tool_contract`
//!    ([`BrokerError::UnknownCapability`]).
//! 2. The capability honors `request.pattern`
//!    ([`BrokerError::UnsupportedPattern`]).
//! 3. The capability is in `warrant.tool_grants`
//!    ([`BrokerError::CapabilityExceedsWarrant`] on
//!    [`kx_warrant::WarrantField::ToolGrants`]).
//! 4. `request.net_scope` ⊆ `warrant.net_scope` and
//!    `request.fs_scope` ⊆ `warrant.fs_scope`
//!    ([`BrokerError::CapabilityExceedsWarrant`] on
//!    [`kx_warrant::WarrantField::NetScope`] /
//!    [`kx_warrant::WarrantField::FsScope`]).
//! 5. Routes to the capability via [`Capability::invoke`]; the
//!    capability returns response bytes the broker stages into the
//!    content store.
//!
//! # D38 §1 — idempotency token plumbing
//!
//! [`idempotency_token_for`] derives the 32-byte idempotency token from
//! a Mote's identity. For `IdempotencyClass::Token` tools, the executor
//! sets `EffectRequest.idempotency_key = Some(idempotency_token_for(mote))`
//! before dispatch; the broker passes it through to the capability,
//! which embeds it in the remote API's idempotency header. Recovery
//! re-dispatch of the same Mote produces the SAME token → the remote API
//! dedupes → no double-effect.
//!
//! # D38 §2a — deterministic readback probe
//!
//! [`CapabilityBroker::probe_readback`] is the broker's surface for the
//! `IdempotencyClass::Readback` flow. The capability's
//! [`Capability::probe`] method queries world state deterministically;
//! `Ok(Some(handle))` means "already applied; skip dispatch"; `Ok(None)`
//! means "proceed". The probe is a deterministic check (D20
//! chain-terminator rule); **never a model call**.
//!
//! # Reading further
//!
//! - `docs/design/capability-broker.md` (private corpus) — the locked D24 spec.
//! - `docs/design/decisions.md` D24, D38 — interlocking decisions.
//! - `05-progress-tracker.md` SN-8 — *model proposes, runtime enforces*.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
// TODO(workspace.lints cleanup): kx-capability uses `.expect()` on RwLock
// `read()` / `write()` in `LocalCapabilityBroker`. These are documented
// infallible at the call sites (poisoning is only possible if a prior
// registration panicked while holding the write lock; the OSS impl
// performs no fallible work under the lock). Follow-up cleanup PR
// migrates to a typed `BrokerError::RegistryPoisoned` variant or to
// `parking_lot::RwLock` (no poisoning). Until then, the documented
// `expect(...)` is the audit trail. Pattern matches the existing
// kx-warrant precedent (its workspace.lints comment in Cargo.toml).
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
// Inline test modules use unwrap freely; expect is already allowed at
// crate level for the RwLock-poison sites. Integration tests under
// tests/*.rs carry their own per-file allow as usual.
#![cfg_attr(test, allow(clippy::unwrap_used))]

mod broker;
mod capability;
mod errors;
mod fs_confine;
mod fs_list;
mod fs_read;
mod fs_write;
mod local;
mod request;
mod token;

pub use broker::CapabilityBroker;
pub use capability::Capability;
pub use errors::{BrokerError, CapabilityFailureReason};
pub use fs_confine::resolve_confined_file;
pub use fs_list::FsListCapability;
pub use fs_read::{FsReadCapability, DEFAULT_MAX_READ_BYTES};
pub use fs_write::{FsWriteCapability, DEFAULT_MAX_WRITE_BYTES};
pub use local::LocalCapabilityBroker;
pub use request::{BrokerHandle, EffectRequest};
pub use token::{idempotency_token_for, run_scoped_token, INSTANCE_ID_LEN};

#[cfg(test)]
mod tests;
