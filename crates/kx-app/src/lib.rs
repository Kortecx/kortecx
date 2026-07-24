//! The kortecx **App envelope** (`kortecx.app/v1`) — the durable, reusable unit
//! of work a user names, saves, lists, and re-runs ("an App").
//!
//! An [`AppEnvelope`] wraps the existing portable **blueprint** (a `DagSpec`,
//! carried VERBATIM as opaque JSON so this crate never duplicates — and so can
//! never drift from — the DAG schema) with:
//!
//! - **references** — by-REFERENCE pointers to context / tools / connections /
//!   datasets plus a minimal prompt / rule / skill / memory artifact rail. A
//!   reference is a name + a content ref (or a registry id); it NEVER inlines
//!   bytes and NEVER carries authority.
//! - a **steering config** — four axes (model+routing, tools+grants-as-WISH,
//!   context+data, guards+budgets) the server RE-RESOLVES at bind. The config
//!   steers; it never grants. A `requested_grants` entry is a wish the server
//!   intersects with the importer's own grant ledger ∩ the step warrant.
//! - **replay** intent — a per-step `FROZEN | RE_RUN` hint (metadata only at
//!   this layer; the runtime's existing whole-run replay does the work).
//! - an optional **`branch_handle`** — reserved for the per-App project branch
//!   (the scaffold that writes into it is a later step; this crate only carries
//!   the handle).
//!
//! ## What an App is NOT (negative-tested, [`AppEnvelope::validate`])
//! An envelope carries **no authority**: no `WarrantSpec`, no `tool_grants`
//! authority, no secret bytes, no credential values, no `instance_id`. A
//! connection reference names a credential (`credential_ref`) by NAME and its
//! descriptor must carry no URL userinfo. The serializer is structurally
//! incapable of emitting authority — `tests/secret_leak.rs` pins it.
//!
//! ## Canonical bytes (SN-8)
//! The hashable / on-the-wire form is canonical JSON: keys sorted (via
//! [`serde_json::Value`]'s `BTreeMap` map, with `preserve_order` OFF — pinned
//! by a unit test), compact separators, integers only (no floats). The pretty
//! form ([`AppEnvelope::to_pretty_json`]) is the human export artifact; both
//! round-trip to the same canonical bytes.
//!
//! Pure leaf: `serde` / `serde_json` / `thiserror` only. Never the journal, the
//! gateway service, or the frozen trio — a dependency-wall test enforces it.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod envelope;

pub use envelope::{
    canonical_json, summary_of, AppEnvelope, AppError, AppKind, AppMode, AppRef, AppSummary,
    ArtifactRef, ConnectionRef, ContextRef, ContextSteering, DatasetRef, Guards, HostedConfig,
    HostedFramework, ModelSteering, Reach, References, Replay, ReplayMode, SkillRef,
    SteeringConfig, ToolRef, ToolsSteering, APP_SCHEMA, EXPERIENCE_SCHEMA, MAX_APP_CORPUS_BYTES,
    MAX_APP_CORPUS_REFS,
};
