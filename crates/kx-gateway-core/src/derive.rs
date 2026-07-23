//! The APP-DERIVE seam (`DeriveApp`) — one prompt in, a reviewable App design out.
//!
//! `ProposeWorkflow` answers "what steps would achieve this goal?". `DeriveApp` answers "what
//! APP is this?" — the steps, the shape they run in, the capabilities each one needs, and (on
//! the hosted lane) the project files it would be built from. It VALIDATES ONLY: it saves no
//! envelope, creates no branch, writes no journal. Nothing exists until the author approves and
//! the client calls `SaveApp` + `ScaffoldApp` through the normal path, which re-derives every
//! authoritative axis server-side.
//!
//! **The capability axis is the point.** A proposed step's `tool_contract` has always come from
//! the vetted role recipe, which is empty for every authoring role — so no proposal has ever
//! carried a tool, and an App could only be plugged in after it existed. Here the model may
//! NAME ids from a menu the host built out of the caller's OWN ceiling, and the host intersects
//! what comes back against that same ceiling. Naming is not granting (SN-8): what survives is a
//! WISH, and `RunApp` intersects it again at fire. The model can only ask for what the caller
//! could already have attached by hand.
//!
//! Like the other model-served seams (`WorkflowProposer`, `AppScaffolder`, `AppAuthor`), the
//! host owns the runtime surface: the concrete impl lives in `kx-gateway` behind `serve-engine`
//! and holds the routing backend, the vetted role catalog, and the live registries. gateway-core
//! defines only the seam and the display-shaped outcome. A `None` seam ⇒ `DeriveApp` returns
//! `unimplemented` (no served model on this gateway).

use std::collections::BTreeMap;

/// What the author asked for. One prompt plus the two selectors that sit on it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeriveInput {
    /// The lane: `"scheduled"` (a functional automation) or `"hosted"` (a served web app).
    /// Anything else is refused rather than defaulted — a mis-typed lane silently designing
    /// the wrong kind of app is worse than a refusal.
    pub kind: String,
    /// The scheduled authoring mode: `"contextual"` or `"codified"`. Ignored for `hosted`
    /// (there is no such axis there). Empty ⇒ the server's default for the lane.
    pub mode: String,
    /// The author's single natural-language prompt. The whole input.
    pub prompt: String,
    /// Hosted only: a pinned framework, or `"auto"`/empty to let the design choose one.
    pub framework: String,
    /// FILENAMES of already-uploaded context attachments. Names only, deliberately: the derive
    /// model holds no grant to dereference a content ref, and a file's bytes are not what shapes
    /// the plan — knowing the app will have `changelog.md` to read is.
    pub attachments: Vec<String>,
}

/// One designed step, in DISPLAY shape.
///
/// `role` / `intent` are the model's design; `kind` and `model_id` are SERVER-resolved from the
/// vetted recipe; `tool_contract` is the model's request already INTERSECTED against the
/// caller's ceiling. Everything here is re-derived server-side when the approved App is authored
/// and run (SN-8) — this response is what the author reviews, never what the runtime trusts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedStep {
    /// The vetted role name this step plays.
    pub role: String,
    /// The model's free-form per-step instruction.
    pub intent: String,
    /// The structural kind (`plain` today — the derive contract authors no critics).
    pub kind: String,
    /// The model id resolved from the role recipe (display only).
    pub model_id: String,
    /// The surviving per-step grant set `{tool_id: version}` — the ceiling's version, never the
    /// model's.
    pub tool_contract: BTreeMap<String, String>,
    /// The catalog SKILL names bound to this step, intersected against the caller's catalog.
    ///
    /// This and the two fields below are the BINDINGS: which node uses the capability. The
    /// app-level lists on [`DerivedApp`] are their union — the DECLARATION set a client
    /// writes into the envelope's `references`. Keeping both is what lets the design the
    /// author reviews and the envelope the App runs from describe the same thing.
    pub skills: Vec<String>,
    /// The INTEGRATION names bound to this step, intersected against the caller's registry.
    pub integrations: Vec<String>,
    /// The DATASET names this step grounds on, intersected against the caller's non-empty
    /// datasets.
    pub datasets: Vec<String>,
    /// The APP handles this step calls, intersected against the caller's own catalog.
    ///
    /// The odd one out among the bindings: the three above give this step more to work with,
    /// while this one puts another App's whole workflow into the run and feeds its result to
    /// this step. That is why the design shows it — an author reviewing "researcher, gathers
    /// the evidence" needs to see that gathering means running their research App.
    pub apps: Vec<String>,
}

/// One planned project file (the hosted lane's review surface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedFile {
    /// The relative project path.
    pub path: String,
    /// One sentence on what the file contains — what the author reviews it by.
    pub role: String,
}

/// A reviewable App design. Persists nothing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DerivedApp {
    /// The proposed app name (the client derives the catalog handle from it).
    pub name: String,
    /// The proposed one-sentence description.
    pub description: String,
    /// What a run of this app produces, in one phrase. Advisory prose, and the line the
    /// composition menu shows other apps' authors — which is why the derive proposes it
    /// rather than leaving every app undiscoverable until someone writes it by hand.
    pub delivers: String,
    /// The designed steps, in plan order. Empty on the hosted lane (a hosted App has no DAG).
    pub steps: Vec<DerivedStep>,
    /// The dependency edges as `(parent_index, child_index)`. **A step with no incoming edge
    /// runs in parallel** — this list is the whole of the shape decision, and an empty list for
    /// a multi-step design means every step runs at once, on purpose.
    pub edges: Vec<(u32, u32)>,
    /// The planned project files. Hosted only; the scheduled lane's files are planned by the
    /// scaffold at approve time, which is where that contract already lives.
    pub files: Vec<DerivedFile>,
    /// The resolved hosted framework (`vite_react` | `next_js` | `svelte`). Empty on scheduled.
    pub framework: String,
    /// App-level tool wishes: the union of every step's surviving grant.
    ///
    /// This and the three lists below are the DECLARATION set — what the App must have
    /// registered for the design to work, which is what a client writes into the envelope's
    /// `references`. Which NODE uses each one is [`DerivedStep`]'s business.
    pub tools: BTreeMap<String, String>,
    /// Catalog skill names the design asked for, intersected against the caller's catalog.
    pub skills: Vec<String>,
    /// Connection descriptors the design asked for, intersected against the caller's registry.
    pub connections: Vec<String>,
    /// Dataset names to ground on, intersected against the caller's non-empty datasets.
    pub datasets: Vec<String>,
    /// App handles the design calls, intersected against the caller's own catalog.
    pub apps: Vec<String>,
    /// Human-readable advisories about what the design did NOT get: ids dropped because they
    /// were not on the menu, a menu bounded by the one-decode budget. Surfaced to the author
    /// verbatim; never parsed for enforcement.
    ///
    /// This list is why the derive can be trusted to be honest about its own limits — a design
    /// that quietly asked for a tool it did not receive would produce an App that quietly cannot
    /// do part of its job.
    pub notices: Vec<String>,
}

/// The outcome of an App derive. Never a transport error: a failure (no served model, an
/// unknown lane, an inadmissible design) is a `Rejected { reason }` the author reads
/// (don't-fake-gaps, D142).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppDerivation {
    /// A decoded, compiled and intersected design, ready to review.
    Derived(Box<DerivedApp>),
    /// The derive was refused; `reason` is human-readable (never parsed for enforcement).
    Rejected {
        /// The advisory reason (surfaced to the author).
        reason: String,
    },
}

/// The host-side App-derive seam. The host impl owns the served-model backend, the vetted role
/// catalog, and the live capability registries; `derive` runs the model and compiles the result
/// (validate-only). A `None` seam ⇒ `DeriveApp` returns `unimplemented`.
///
/// Async so the host can offload the BLOCKING model inference (e.g. via
/// `tokio::task::spawn_blocking`) — gateway-core stays runtime-light (no direct `tokio`).
#[tonic::async_trait]
pub trait AppDeriver: Send + Sync {
    /// Derive a reviewable App design from one prompt. Never errors at the transport level: a
    /// failure is an [`AppDerivation::Rejected`].
    ///
    /// `principal` scopes every capability the design may reach: the menu is built from this
    /// caller's ceiling and the result is intersected back against it.
    async fn derive(&self, principal: &str, input: DeriveInput) -> AppDerivation;
}
