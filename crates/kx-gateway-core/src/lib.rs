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

mod capture_view;
mod datasets;
mod error;
mod events;
mod feedback_view;
mod identity;
mod models_view;
mod mote_def_view;
mod mote_detail;
mod react;
mod reader;
mod replan;
mod run_inputs_view;
mod runs;
mod service;
mod submit;
mod telemetry_view;
mod toolscout_view;
mod uploads;
mod view;
mod writer;

pub use capture_view::{CaptureRecordEntry, CaptureView};
pub use datasets::{
    DatasetError, DatasetHitEntry, DatasetSummaryEntry, DatasetView, IngestDoc, IngestOutcome,
};
pub use error::GatewayError;
pub use feedback_view::{FeedbackEntry, FeedbackRecord, FeedbackStore};
// The event-source pieces a live tailer (R5, `kx-gateway`) reuses: the one-time
// ownership gate + the per-range frame builder. The snapshot composition stays
// crate-private (it backs the default `SnapshotTailer`). Batch C adds the
// GLOBAL twin's pieces (cursor seed + per-range builder) for the live global
// tailer.
pub use events::{
    check_mote_in_run, check_run_ownership, frames_for_range, global_frames_for_range,
    seed_global_cursor, GlobalCursor,
};
pub use identity::CallerParty;
pub use models_view::{ModelCatalogView, ModelSummaryEntry};
pub use mote_def_view::MoteDefView;
pub use mote_detail::{MAX_CONFIG_ENTRIES, MAX_CONFIG_VALUE_BYTES, MAX_PROMPT_BYTES};
pub use reader::{ContentReader, JournalReader, ReadOnly};
pub use run_inputs_view::{RunInputsEntry, RunInputsRecord, RunInputsStore};
pub use service::{
    AssetGrantsView, AuthorEdge, AuthorExecutionMode, AuthorStep, AuthorStepKind, BinderError,
    BoundRecipe, CatalogSeamError, EventStream, EventTailer, GatewayService, GlobalEventStream,
    GlobalEventTailer, GrantEntry, GrantView, MembershipView, NoTokenTailer, RecipeBinder,
    RecipeCatalog, RecipeFormFieldEntry, RecipeMetadataEntry, RecipeParamKind, RegisteredSignature,
    ScoredRecipeEntry, SignatureCatalog, SignatureSummaryEntry, SnapshotGlobalTailer,
    SnapshotTailer, TeamMemberEntry, TeamMembersView, TeamSummaryEntry, TokenStream, TokenTailer,
    WarrantProjection, WorkflowAuthor, BATCH_ITEM_CLAMP_BYTES, DEFAULT_PUT_CAP_BYTES,
    MAX_BATCH_REFS, MAX_FEEDBACK_COMMENT_BYTES, REFUSAL_CODE_METADATA_KEY,
    SEARCH_RECIPES_DEFAULT_LIMIT, SEARCH_RECIPES_MAX_LIMIT,
};
pub use submit::{
    RunSubmitter, SubmitMoteOutcome, SubmitStatus, SubmitterError, TonicCoordinatorSubmitter,
};
pub use telemetry_view::{MoteTelemetryEntry, TelemetryView};
pub use toolscout_view::{
    BundleScoreView, BundleSpecEntry, BundleToolSpecEntry, KeywordSetEntry, LowerVerdictEntry,
    ManifestScoreEntry, ToolManifestEntry, ToolScoutView,
};
pub use uploads::{UploadRecord, UploadsLedger};
pub use writer::ContentWriter;
