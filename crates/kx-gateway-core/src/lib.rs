#![forbid(unsafe_code)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown
)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! # kx-gateway-core — the external `KxGateway` backend (M8 / D120)
//!
//! > **Phase: client surfaces (M8).** The OSS backend behind the
//! > [`KxGateway`](kx_proto::proto::kx_gateway_server::KxGateway) service — the
//! > client-facing surface over the durable runtime. It is a **read-fold +
//! > propose-proxy**, not a new write path. See the README (How it works).
//!
//! ## What it does
//!
//! - **Read-fold** — `GetProjection` folds the run's journal into a
//!   [`ProjectionView`](kx_proto::proto::ProjectionView) (render-a-run-as-a-DAG);
//!   `GetContent` returns a committed result by ref; `StreamEvents` is a
//!   resumable [`EventFrame`](kx_proto::proto::EventFrame) cursor. Every
//!   `MoteSnapshot` is **server-derived from the fold** — the client never
//!   computes a `MoteId` (SN-8 / D70).
//! - **Propose-proxy** — `SubmitRun` registers a run and submits its Motes
//!   through the [`RunSubmitter`] seam to the coordinator (the sole journal
//!   writer, D40). It returns only after the journaled `instance_id` (never acks
//!   ahead of the journal).
//!
//! ## The no-write wall (D120.5)
//!
//! gateway-core adds **no journal write path**, enforced at the type level
//! (Rule 5.2 — a write cannot type-check):
//! - reads go through [`JournalReader`] (no `append`) + [`ContentReader`]
//!   (no `put`); the [`ReadOnly`] newtype exposes only the read methods of any
//!   [`kx_journal::Journal`].
//! - submits go through [`RunSubmitter`] to the coordinator over gRPC — so
//!   gateway-core never links `kx-coordinator`/`kx-executor`/`kx-scheduler`/
//!   `kx-capture` (a dep-wall test pins it).
//!
//! Auth / ownership / multitenancy never appear in gateway-core signatures —
//! they live in `kx-cloud/gateway-auth` (D102.1). The `instance_id` is treated
//! as an opaque ownership ticket: a request is authorized iff the run's journal
//! names that `instance_id`; `GetContent` returns a **uniform** not-authorized
//! (no existence oracle).

mod active_model;
mod alerts_view;
mod apps_view;
mod branches_view;
mod bundles_view;
mod capture_view;
mod datasets;
mod error;
mod events;
mod feedback_view;
mod fuzzy_discovery;
mod identity;
mod locks_view;
mod mcp_gateway_admin;
// MM-3 (D110): the LOCAL secret-store admin seam (PutSecret/ListSecretNames/
// DeleteSecret). Pure vocabulary trait; the host impl is keychain-backed. The
// value is write-only (put arg) — never on a return type, the wire, or the journal.
mod secret_admin;
// D114/M11 (autonomy safety): the approval + cost-readout admin seam
// (ListPendingApprovals/Grant/Deny/GetRunCost). Async; the host impl dispatches
// coordinator commands (grant/deny append durable decision facts). No writer dep.
mod approval_admin;
// D113 (trigger seam): the trigger admin seam (Register/List/Deregister/Submit/Test).
// Async (binds + submits a run via the Invoke propose-proxy); the host impl owns the
// triggers.db store + the binder + submitter. No journal-writer dep added here.
mod model_lifecycle;
mod model_pull;
mod models_view;
mod mote_def_view;
mod mote_detail;
mod react;
mod reader;
mod replan;
mod run_inputs_view;
mod runs;
mod scaffold;
mod server_info;
mod service;
mod submit;
mod telemetry_view;
mod tool_registry_admin;
mod toolscout_view;
mod trigger_admin;
mod uploads;
mod view;
mod writer;

pub use alerts_view::{AlertEntry, AlertView};
pub use apps_view::{AppCatalog, AppRecord, MAX_APP_ENVELOPE_BYTES};
pub use branches_view::{
    BranchItemRecord, BranchManifest, BranchStore, MAX_BRANCH_DESCRIPTION_BYTES, MAX_SNAPSHOT_PATHS,
};
pub use bundles_view::{
    BundleItemRecord, BundleManifest, BundleStore, MAX_BUNDLE_DESCRIPTION_BYTES,
    MAX_CONTEXT_BUNDLE_ITEMS,
};
pub use capture_view::{CaptureRecordEntry, CaptureView};
pub use datasets::{
    DatasetError, DatasetHitEntry, DatasetSummaryEntry, DatasetView, IngestDoc, IngestOutcome,
};
pub use error::GatewayError;
pub use feedback_view::{FeedbackEntry, FeedbackRecord, FeedbackStore};
pub use fuzzy_discovery::{score_to_bp, FuzzyDiscoveryView, FuzzyHitEntry};
pub use locks_view::{LockStore, LOCKED_BRANCH_REFUSAL_CODE};
pub use scaffold::{
    authoring_prompt, body_is_empty, derive_phase, split_done_pending, try_committed_body,
    AppScaffolder, ScaffoldFile, ScaffoldPhase, ScaffoldStatus, ScaffoldStep,
    APP_SCAFFOLD_WRITE_RECIPE_HANDLE, SKELETON,
};
// The event-source pieces a live tailer (R5, `kx-gateway`) reuses: the one-time
// ownership gate + the per-range frame builder. The snapshot composition stays
// crate-private (it backs the default `SnapshotTailer`). Batch C adds the
// GLOBAL twin's pieces (cursor seed + per-range builder) for the live global
// tailer.
pub use active_model::ActiveModelControl;
pub use approval_admin::{ApprovalAdmin, ApprovalAdminError, PendingApprovalRow, RunCostRow};
pub use events::{
    check_run_ownership, frames_for_range, global_frames_for_range, seed_global_cursor,
    GlobalCursor,
};
pub use identity::CallerParty;
pub use mcp_gateway_admin::{
    CallToolOutcome, McpAdminError, McpGatewayAdmin, McpServerRegistration, McpServerView,
    RegisterServerOutcome,
};
pub use model_lifecycle::{ModelLifecycleControl, ModelLifecycleOutcome};
pub use model_pull::{ModelPuller, PullAdmission, PullPhase, PullProgress, PullSource};
pub use models_view::{ModelCatalogView, ModelSummaryEntry};
pub use mote_def_view::MoteDefView;
pub use mote_detail::{MAX_CONFIG_ENTRIES, MAX_CONFIG_VALUE_BYTES, MAX_PROMPT_BYTES};
pub use reader::{ContentReader, JournalReader, ReadOnly};
pub use run_inputs_view::{RunInputsEntry, RunInputsRecord, RunInputsStore};
pub use secret_admin::{SecretAdmin, SecretAdminError, SecretNameView};
pub use server_info::ServerInfoFacts;
pub use service::{
    AssetGrantsView, AuthorEdge, AuthorExecutionMode, AuthorStep, AuthorStepKind, BinderError,
    BoundRecipe, CatalogSeamError, EventStream, EventTailer, GatewayService, GlobalEventStream,
    GlobalEventTailer, GrantEntry, GrantView, MembershipView, NoTokenTailer, RecipeBinder,
    RecipeCatalog, RecipeFormFieldEntry, RecipeMetadataEntry, RecipeParamKind, RegisteredSignature,
    RegisteredToolsView, ScoredRecipeEntry, SignatureCatalog, SignatureSummaryEntry,
    SnapshotGlobalTailer, SnapshotTailer, TeamMemberEntry, TeamMembersView, TeamSummaryEntry,
    TokenStream, TokenTailer, WarrantProjection, WorkflowAuthor, BATCH_ITEM_CLAMP_BYTES,
    DEFAULT_PUT_CAP_BYTES, MAX_BATCH_REFS, MAX_FEEDBACK_COMMENT_BYTES, REFUSAL_CODE_METADATA_KEY,
    SEARCH_RECIPES_DEFAULT_LIMIT, SEARCH_RECIPES_MAX_LIMIT,
};
pub use submit::{
    RunSubmitter, SubmitMoteOutcome, SubmitStatus, SubmitterError, TonicCoordinatorSubmitter,
};
pub use telemetry_view::{ModelTokenRollup, MoteTelemetryEntry, TelemetrySummary, TelemetryView};
pub use tool_registry_admin::{
    RegisteredToolEntry, ToolAdminError, ToolParamWire, ToolRegistration, ToolRegistryAdmin,
    ToolSchemaWire,
};
pub use toolscout_view::{
    BundleScoreView, BundleSpecEntry, BundleToolSpecEntry, KeywordSetEntry, LowerVerdictEntry,
    ManifestScoreEntry, ToolManifestEntry, ToolScoutView,
};
pub use trigger_admin::{
    TriggerAdmin, TriggerAdminError, TriggerFireOutcome, TriggerRegistration, TriggerView,
};
pub use uploads::{UploadRecord, UploadsLedger};
pub use writer::ContentWriter;
