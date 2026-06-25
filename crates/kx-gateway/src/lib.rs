#![forbid(unsafe_code)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown
)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! # kx-gateway — the kortecx single-system gateway server (R1 / D130)
//!
//! > **Phase: reachability (v0.1.0).** The first kortecx binary that binds a
//! > network port outside tests. It hosts the FROZEN
//! > [`KxGateway`](kx_proto::proto::kx_gateway_server::KxGateway) gRPC service
//! > over [`kx_gateway_core`] so a human / SDK / browser can connect and run a
//! > workflow over the wire. See the README (How it works; core-vs-distributed legend).
//!
//! ## What it hosts
//!
//! `kx-gateway serve` brings up, in ONE process:
//! - an embedded **coordinator** (the sole journal writer, D40) on a loopback
//!   port (behind the default-on `embedded-worker` feature);
//! - an embedded **local worker** that leases → runs (PURE, deterministic) →
//!   proposes commits, so a submitted run actually reaches `Committed`;
//! - the **gateway** service ([`kx_gateway_core::GatewayService`]) over a
//!   *second, read-only* journal handle + the shared content store. `SubmitRun`
//!   proxies to the embedded coordinator; `GetProjection` / `GetContent` /
//!   `StreamEvents` fold the journal read-only.
//!
//! This is a **single-system** server (D129): one process, one journal, one
//! principal namespace. Multi-node coordination + multi-tenant isolation are the
//! cloud product; a hosted OSS server is single-tenant by construction.
//!
//! ## Auth (R2: real bearer-token resolver)
//!
//! The freshly-bound port defaults to **deny-all** ([`auth::DenyAll`]): every RPC
//! returns `unauthenticated` unless the operator either passes `--dev-allow-local`
//! (a fixed local-dev principal, loopback-only) or configures bearer tokens
//! (`--auth-token <token>=<party>` / `--auth-token-file <path>`) which install a
//! [`auth::TokenResolver`]. The [`auth::PrincipalResolver`] seam is the fill point
//! — mTLS / OIDC are later impls of the same trait (OIDC stays cloud, D94/D101.1).
//! Identity is **server-derived** from transport metadata, never client-asserted
//! (SN-8): the client supplies a credential, not a claimed identity.
//!
//! ## The no-write discipline (D120.5)
//!
//! The gateway never writes the journal: it holds a [`kx_gateway_core::ReadOnly`]
//! handle (no `append`) + a `ContentReader` (no `put`) and a propose-proxy. Only
//! the embedded coordinator appends — sole-writer is structural, not by
//! convention. The frozen trio (`kx-scheduler` / `kx-executor`) source is
//! untouched; this crate only *consumes* their public API.

// PR-2c F-7: render a model Mote's resolved Data context (WorkItem.parent_results)
// into the prompt. Self-contained + FFI-free; shares the `inference` feature with
// the model executor that consumes it.
#[cfg(feature = "serve-engine")]
mod assemble_serve;
mod auth;
mod config;
// D139: the embedded web console — hyper serving the compile-time-embedded SPA
// on a third loopback listener. Behind the off-by-default `console` feature so
// plain builds never need node or a built `ui/dist` (build.rs embeds it).
#[cfg(feature = "console")]
mod console;
// T3.7: the host-side Datasets data-plane (the kx-dataset-hnsw-backed DatasetView
// seam). Pulls the FFI-free dataset crates, so it's behind the off-by-default
// `hnsw` feature (the default build + the dep-wall stay unchanged).
#[cfg(feature = "hnsw")]
mod datasets;
// W1a-2: the alerts.db read-cache (the AlertView seam) — the operator alerts
// inbox folded from the journal's TERMINAL `Failed` facts. Read-only,
// off-truth-path, rebuildable (the capture.db posture). The triage lifecycle
// (ack/resolve), rule engine, and notifications are a Cloud capability (D156).
mod alerts;
// PR-7: the bundles.db sidecar (the BundleStore seam) — context-bundle manifests
// (PutContextBundle) + the bind-time resolution of `context_bundles`. Rebuildable-
// to-empty (caller-authored, never journaled), off-journal, off-digest; like
// uploads, no executor wrapper — always-on, FFI-free (rusqlite already in closure).
mod bundles;
// POC-4: the apps.db sidecar (the AppCatalog seam) — SaveApp / ListApps / GetApp.
// Stores a caller's kortecx.app/v1 envelopes (canonicalized + summary-derived via
// the kx-app leaf type); off-journal, off-digest, rebuildable-to-empty (like
// bundles, no broker dep — kx_content::ContentRef::of derives app_ref).
mod apps;
// POC-5b: the locks.db sidecar (the LockStore seam) — per-App lock toggled by
// LockApp/UnlockApp + enforced at the AdvanceBranch chokepoint. Off-journal,
// off-digest, rebuildable-to-empty (FAILS OPEN on loss — an availability gate).
mod locks;
// POC-5a: the host AppScaffolder impl — the server-side scaffold orchestrator that
// drives the fixed-skeleton write loop into a CoW branch. Gated to `embedded-worker`
// (it binds + submits recipes + folds the projection to await each step).
#[cfg(feature = "embedded-worker")]
mod scaffold;
// D155 Phase-A: the branches.db sidecar (the BranchStore seam) — CreateBranch /
// SnapshotInto manifests of {host-path -> ContentRef}. SnapshotInto reads confined
// host files into CAS (reusing fs-list's airtight confinement via kx-capability),
// so the module is gated to `embedded-worker` (where the broker + content store +
// kx-capability live; the gateway-only `--no-default-features` closure is
// unchanged). Off-journal, off-digest, rebuildable-to-empty.
#[cfg(feature = "embedded-worker")]
mod branches;
// The Morphic Data Engine (campaign Batch 2): the durable serve-path capture
// projection (capture.db sidecar folded from the read-only journal handle).
// Always-on, off-truth-path; FFI-free (rusqlite is already in the closure).
mod capture;
// POC-5a (CAS env-knobs / F4): additive, default-preserving operator overrides for
// the serve context window + agentic-edit decode budget + chat-RAG fan-in caps.
mod env_caps;
mod error;
// PR-4.1: the feedback.db sidecar (the FeedbackStore seam) — the SubmitFeedback
// 👍/👎 rows + their ListFeedback read-back. Rebuildable-to-empty (client-origin
// product signal, never journaled), off-journal, off-digest. Like uploads, no
// executor wrapper — always-on, FFI-free (rusqlite already in the closure).
mod feedback;
mod live_tail;
// W1a (T-OBS2): the always-available Prometheus `/metrics` listener (opt-in via
// `--metrics-listen`). FFI-free (hyper http1 only); not feature-gated.
mod metrics;
// PR-2d-2: the bundled deterministic stdio MCP tool's wiring (locate the bin,
// register the capability + the typed ToolDef). Behind `serve-engine` (the react
// decode arm lives in the serve-engine executor; the MCP adapter is FFI-free).
#[cfg(feature = "serve-engine")]
mod mcp_tool;
// PR-6b-1 (D159): the EXTERNAL MCP gateway host wiring — the McpGatewayAdmin impl
// over kx_mcp_gateway::McpGateway + the BrokerCapabilitySink. Behind the
// `mcp-gateway` feature (ON by default); FFI-free, off-journal, off-digest. DIALS
// external MCP servers + governs connections; the autonomous-loop auto-grant is
// PR-6b-2 (a dialed tool is registered + fireable through a granting warrant).
#[cfg(feature = "mcp-gateway")]
mod mcp_gateway;
// AL1: the in-process model executor for `kx serve` (live LLM dispatch). FFI-FREE
// (the generic loop), so it's behind the off-by-default `serve-engine` feature; the
// llama.cpp-specific bits inside are additionally `inference`-gated.
#[cfg(feature = "serve-engine")]
mod model_exec;
// The host-owned routing backend (one InferenceBackend + lifecycle over N serve
// engines — llama.cpp and/or Ollama). FFI-free; rides `serve-engine`.
#[cfg(feature = "serve-engine")]
mod routing_backend;
// PR-4.2 (T-STREAM1): the ADVISORY in-process token broker — the rendezvous
// between the model executor (publisher) and the live-token subscribers. Only
// a serve-engine build dispatches models + publishes tokens, so it's serve-engine-
// gated; out-of-band (never journal / digest / identity).
#[cfg(feature = "serve-engine")]
mod token_broker;
// PR-4.2 (T-STREAM1): the LiveTokenTailer — the broker-backed `TokenTailer` impl
// the gRPC `StreamModelTokens` handler + the WS `/tokens` bridge subscribe
// through. Needs the broker, so serve-engine-gated; the model-less build serves the
// core `NoTokenTailer` (an honest empty stream).
#[cfg(feature = "serve-engine")]
mod token_tail;
// Batch A: the host-side model catalog (the ModelCatalogView seam) — always
// wired so an FFI-free serve answers ListModels with an honest empty list.
mod models;
// POC-3: the host-side model lifecycle (the ModelLifecycleControl seam + the
// BackendEngine adapter). FFI-free traits, so it rides `serve-engine`; the llama
// `BackendEngine` newtype inside is additionally `inference`-gated.
#[cfg(feature = "serve-engine")]
mod model_lifecycle;
// Model Control v2: the host-side active-default-model control (SetActiveModel) +
// the model-acquisition orchestrator (PullModel/GetPullStatus). serve-engine-gated
// (they register into the live routing/catalog; the direct-URL download arm inside
// `model_pull` is additionally `inference`-gated).
#[cfg(feature = "serve-engine")]
mod active_model;
#[cfg(feature = "serve-engine")]
mod model_pull;
// Batch B: the host-side def resolver (the MoteDefView seam) — always wired
// over the SAME content store the coordinator persists admitted defs into.
mod mote_defs;
mod provision;
#[cfg(feature = "embedded-worker")]
mod real_exec;
// PR-D: the run_inputs.db sidecar (the RunInputsStore seam) — the Invoke args
// captured at submit so a run recovered from ListRuns can pre-fill its recipe
// form and be re-invoked with edits ("Re-run with changes"). Rebuildable-to-
// EMPTY, off-journal, off-digest, off-identity.
mod run_inputs;
mod server;
// Batch C: the telemetry.db sidecar (the TelemetryView seam) — host-measured
// execution exhaust (wall-clock / model usage / fired tool), joined to the
// journal's Committed facts by a background tick. Rebuildable-to-EMPTY,
// off-journal, off-digest; the hot-path sink is bounded + fail-open. Needs the
// embedded worker (the executor wrapper measures the worker's mote loop).
#[cfg(feature = "embedded-worker")]
mod telemetry;
// UI-3: the host-side teams (MembershipView) + grants (GrantView) read seams + the
// idempotent demo-team seed.
mod teams;
mod tls;
// W1.A5: the host-side advisory toolscout view (manifests + bundle scoring +
// the lowering dry-run verdict). Always-on; display-only by construction.
mod toolscout;
// PR-6a: the host-side declarative tools registry admin (the ToolRegistryAdmin
// seam over the durable tools.db) + admission-time SSRF vetting of a
// RegisterTool's server_host. Always-on; off-journal, off-digest. DIALING the
// external MCP server + Connections + parallel fan-out are PR-6b/Cloud (D159).
mod tools;
// Batch A: the uploads.db sidecar (the UploadsLedger seam) — the PutContent
// audit rows + the uploads-scope authorized set. Rebuildable-to-empty (truth
// stays in the content store), off-journal, off-digest.
mod uploads;
#[cfg(feature = "embedded-worker")]
mod ws;

pub use auth::{DenyAll, DevAllowLocal, Principal, PrincipalResolver, TokenResolver};
pub use config::{
    Cli, ConsoleMode, GatewayConfig, TlsPaths, DEFAULT_CONSOLE_LISTEN, DEFAULT_CONTENT_MAX_BYTES,
    DEFAULT_MAX_LEASE, DEFAULT_WS_LISTEN, USAGE,
};
#[cfg(feature = "hnsw")]
pub use datasets::HostDatasetView;
#[cfg(all(feature = "hnsw", feature = "serve-engine"))]
pub use datasets::HostEmbedder;
pub use error::GatewayError;
pub use live_tail::{GlobalLiveTailer, LiveTailer};
pub use provision::{
    DemoLibrary, HostRecipeBinder, HostRecipeCatalog, HostSignatureCatalog, HostWorkflowAuthor,
    CHAT_RAG_RECIPE_HANDLE, DEMO_RECIPE_HANDLE, JUDGE_RECIPE_HANDLE, MODEL_RECIPE_HANDLE,
    PASSTHROUGH_DAG_HANDLE, REACT_AUTO_RECIPE_HANDLE, REACT_EDIT_RECIPE_HANDLE,
    REACT_FS_RECIPE_HANDLE, REACT_RECIPE_HANDLE, VISION_RECIPE_HANDLE,
};
pub use server::{serve, start, RunningGateway};
pub use teams::{seed_workspace_team, HostGrantView, HostMembershipView, WORKSPACE_TEAM_HANDLE};

#[cfg(feature = "embedded-worker")]
pub use server::{default_executor_class, pure_run_request};
