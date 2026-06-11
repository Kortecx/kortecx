//! W1.A5 — the [`TaskBundle`] authoring shape (the MCP-intelligence foundation).
//!
//! A **`TaskBundle`** is the content-addressed template for a multi-tool task: an
//! intent, an ordered tool sequence, per-tool advisory metadata, and an
//! integer-scaled tolerance threshold. It is what a user (or, later, a model
//! through the kx-mcp-gateway composer) authors; `kx-toolscout` LOWERS it to a
//! [`WorkflowDef`](https://docs.rs/kx-workflow) behind the exact-equality
//! `tool_grants` gate, and the **frozen** `kx_workflow::compile` derives the
//! Mote DAG. The bundle itself is off-journal and off the identity path — Mote
//! identity enters only through the lowered, compiled DAG.
//!
//! Kept as a thin TYPE crate (the `TaskSignature` precedent) so wave-3
//! consumers (kx-mcp-gateway) can take the type without the toolscout
//! machinery.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod bundle;

pub use bundle::{TaskBundle, TaskBundleFingerprint, ToolMeta, TASK_BUNDLE_SCHEMA_VERSION};
