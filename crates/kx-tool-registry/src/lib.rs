// SPDX-License-Identifier: Apache-2.0
//! `kx-tool-registry` — the two-file tool layer (D32).
//!
//! **Two files, not one.**
//! - **Per-workflow file** (the CLI config form of a workflow) — markdown body
//!   for human intent + typed FRONT-MATTER for the enforceable Warrant/Role.
//!   The front-matter is the ONLY enforcement source. Parsed by the SDK/CLI at
//!   workflow-submit time (out of scope for this crate).
//! - **Shared organizational registry** (this crate) — holds the available
//!   tools. Each tool declares its OWN `ToolRequirement`. Built-ins ship on
//!   fresh install; custom tools accrete.
//!
//! Workflow `tool_grants` are **REFERENCES** into the registry, not copies.
//! The registry holds the spec; workflows reference by `(ToolName, ToolVersion)`.
//!
//! # Resolution path (D32 §5)
//!
//! `local → registry → MCP`. Invisible to the capability model — the warrant
//! sees only the `(ToolName, ToolVersion)` reference. The resolved tier is
//! **content-addressed** by a [`ToolResolutionEvent`] (the event struct is a
//! pure resolution artifact — NOT itself a journal entry). In M1.2 (D79) the
//! coordinator captures the resolved `(tool_id, tool_version, resolved_kind,
//! resolved_def_hash)` of every grant — via [`resolve_run_versions`] — as
//! off-DAG run **metadata** (a journaled `RunVersionsResolved` fact attached to
//! the run's `instance_id`); these versions are audit/lineage metadata, never
//! folded into identity. At resolution time, [`check_tool_requirement`]
//! enforces `tool.required_capability ⊆ warrant`; the broker (P1.8.5) never
//! sees a tool whose capability exceeds the warrant.
//!
//! [`check_tool_requirement`]: kx_warrant::check_tool_requirement
//!
//! # MCP tools as egress (monotonic with `net_scope`)
//!
//! MCP tools are remote → granting one requires the warrant's `net_scope` to
//! permit the MCP endpoint's host. A `net_scope = None` warrant cannot resolve
//! any MCP tool — the subset check rejects the resolution at the registry
//! layer, before any dispatch.
//!
//! # Self-generated tools INERT until human review
//!
//! Tools emitted by Motes are recorded with
//! [`ToolProvenance::SelfGenerated`] and start in
//! [`RegistrationStatus::PendingHumanReview`]. They are **INERT** —
//! [`ToolRegistry::resolve`] returns [`ResolutionError::PendingHumanReview`]
//! until [`ToolRegistry::approve_registration`] is called. Approval enforces
//! `def.required_capability ⊆ generating_lineage_warrant`. This closes the
//! privilege-laundering path where a model could emit a tool with broader
//! scope than the lineage that authored it (SN-8: model proposes, runtime
//! enforces).
//!
//! # OSS impl vs cloud impl
//!
//! [`InMemoryToolRegistry`] is the OSS impl — accretes within a single process
//! lifetime; appropriate for the OSS demo + local dev. The trait surface admits
//! a future `kx-cloud-tool-registry-hosted` crate (per-tenant persistence,
//! multi-host accretion, attestation) without trait change per D28.
//!
//! # Reading further
//!
//! - `docs/design/tool-registry.md` (private corpus) — the locked D32 spec.
//! - `docs/design/decisions.md` D32 — interlocking with D24 (broker), D30
//!   (warrant), D29 (validator).
//! - `05-progress-tracker.md` SN-8 — *model proposes, runtime enforces*.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown,
    clippy::return_self_not_must_use,
    clippy::needless_pass_by_value,
    // ToolProvenance::SelfGenerated carries a WarrantSpec (~hundreds of bytes);
    // HumanAuthored carries just a small String. The size disparity is
    // intentional — boxing the WarrantSpec would obscure the semantic shape
    // (the lineage warrant is part of the provenance, not a side reference)
    // for a negligible memory win. SelfGenerated registrations are also rare
    // (most tools are HumanAuthored).
    clippy::large_enum_variant
)]
// Inline test modules are exempted from the workspace deny on `unwrap_used` /
// `expect_used`. Integration tests under tests/*.rs carry per-file allows.
// TODO(workspace.lints cleanup): canonical-bincode encode on RegistrationToken
// + ToolResolutionEvent are documented infallible (no floats; closed enum).
// Follow-up cleanup PR migrates to shared helper or typed error.
#![allow(clippy::expect_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

mod errors;
mod idempotency_class;
mod ids;
mod provenance;
mod registry;
mod token;
mod tool_def;
mod tool_kind;

pub use errors::{RegistrationError, ResolutionError};
pub use idempotency_class::IdempotencyClass;
pub use ids::{McpEndpointId, RegistrationToken, ReviewerId};
pub use provenance::{RegistrationStatus, ToolProvenance};
pub use registry::{resolve_run_versions, InMemoryToolRegistry, ToolRegistry};
pub use token::registration_token_of;
pub use tool_def::{ResolvedTool, ToolDef, ToolResolutionEvent};
pub use tool_kind::ToolKind;

// Re-exports for downstream ergonomic use.
pub use kx_warrant::{FsScope, NetScope, ResourceCeiling};

#[cfg(test)]
mod tests;
