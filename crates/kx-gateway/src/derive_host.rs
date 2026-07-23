//! `HostAppDeriver` — the served-model side of the `DeriveApp` seam.
//!
//! One prompt in, a reviewable App design out. The shape mirrors
//! [`crate::propose_host::HostWorkflowProposer`] deliberately: run the served model, decode
//! fail-closed, and judge admissibility with the SAME vetted `compile_plan` gate. What is new
//! is the capability axis — the model is handed an ids-only menu built from the caller's own
//! ceiling, and everything it names is intersected back against that ceiling here.
//!
//! It VALIDATES ONLY: it registers nothing, saves no envelope, creates no branch, writes no
//! journal. It is therefore unaffected by the run-registration dedup and is digest-invariant.
//! A model, decode, or compile failure is an honest [`AppDerivation::Rejected`], never a panic
//! (D142).
//!
//! **The two lanes derive different things, because they ARE different things.** A scheduled app
//! IS its workflow, so the scheduled lane derives a DAG and the scaffold plans its files later,
//! through the contract that already owns that job. A hosted app IS its files, so the hosted
//! lane derives the file manifest — reusing `manifest_plan_directive` + `decode_manifest`
//! verbatim rather than growing a second file-planning contract that would drift from the first.

use std::collections::BTreeMap;
use std::sync::Arc;

use kx_gateway_core::{
    AppDerivation, AppDeriver, DatasetView, DeriveInput, DerivedApp, DerivedFile, DerivedStep,
    ScaffoldLane, SkillCatalog,
};
use kx_inference::{InferenceBackend, InferenceInput};
use kx_mote::{InferenceParams, ModelId, RoleId};
use kx_planner::{compile_plan, seed_from_plan_bytes, RoleRecipeResolver};
use kx_tool_registry::ToolRegistry;
use kx_warrant::{ExecutorClass, RoleRegistry};

use crate::app_run::principal_tool_ceiling;
use crate::derive_plan::{decode_derived, CapabilityMenu, DerivedPlan, MenuTruncation};
use crate::manifest::{decode_manifest, manifest_plan_directive};
use crate::model_exec::{build_authoring_role_catalog, shaper_warrant};
use crate::prompt_library::{derive_user_message, DERIVE_SYSTEM};
use crate::routing_backend::RoutingBackend;

/// The hosted framework the design falls back to when it names none, or names one that is not
/// a real template. Matches `framework_contract`'s own `_` arm, so a fallback here and a
/// fallback there cannot describe different projects.
const DEFAULT_FRAMEWORK: &str = "vite_react";

/// The vetted hosted templates. A framework outside this set is REPLACED by
/// [`DEFAULT_FRAMEWORK`] with a notice, never passed through: the supervisor can only launch a
/// template it has, and a design promising "solid" would scaffold a project that cannot start.
const HOSTED_FRAMEWORKS: &[&str] = &["vite_react", "next_js", "svelte"];

/// The live capability registries the menu is computed from. Grouped into one struct because
/// they travel together and are only ever read together — six positional `Arc`s on a
/// constructor is how `runApp`'s salt ended up permanently empty.
pub(crate) struct CapabilitySources {
    /// The shared library (grants + blueprint_base) — the caller-authority leg of the ceiling.
    pub(crate) lib: Arc<crate::provision::DemoLibrary>,
    /// The LIVE tool registry (the SAME `Arc` the coordinator + broker share).
    pub(crate) tools: Arc<dyn ToolRegistry>,
    /// The broker-fireable view (the SAME truth the admission backstops intersect against).
    pub(crate) registered: Arc<dyn kx_gateway_core::RegisteredToolsView>,
    /// The per-principal skill catalog. `None` ⇒ the menu offers no skills (honest: a serve
    /// without skills.db has none to offer).
    pub(crate) skills: Option<Arc<dyn SkillCatalog>>,
    /// The caller's registered MCP connections. `None` ⇒ no integrations offered.
    pub(crate) connections: Option<Arc<kx_mcp_gateway::SqliteConnectionStore>>,
    /// The live dataset store. `None` on a build without the retrieval seam (`hnsw` off).
    pub(crate) datasets: Option<Arc<dyn DatasetView>>,
}

/// The host deriver: the served-model backend, the vetted role catalog it compiles against, and
/// the live registries the capability menu is built from.
pub(crate) struct HostAppDeriver {
    backend: Arc<RoutingBackend>,
    model_id: ModelId,
    exec_class: ExecutorClass,
    role_registry: Arc<dyn RoleRegistry>,
    recipes: Arc<dyn RoleRecipeResolver>,
    sources: CapabilitySources,
}

impl HostAppDeriver {
    /// Wire the deriver for a served model. The role catalog is the same curated authoring
    /// palette the proposer resolves against (SN-8 axes come from the vetted recipes).
    pub(crate) fn new(
        backend: Arc<RoutingBackend>,
        model_id: ModelId,
        exec_class: ExecutorClass,
        sources: CapabilitySources,
    ) -> Self {
        let (role_registry, recipes) = build_authoring_role_catalog(&model_id, exec_class);
        Self {
            backend,
            model_id,
            exec_class,
            role_registry,
            recipes,
            sources,
        }
    }

    /// Build the ids-only capability menu for `principal`.
    ///
    /// Every axis is scoped to this caller. The tool axis reuses
    /// [`principal_tool_ceiling`] — the SAME function `GetAppManifest` reports against and
    /// `RunApp` intersects at fire — so a tool this menu offers can never be one the run would
    /// then drop, and a tool it withholds is one the caller genuinely cannot fire.
    ///
    /// A registry read that fails degrades that ONE axis to empty rather than failing the
    /// derive: a design with no skills offered is a smaller design, while a refused derive is
    /// no design at all.
    fn menu(&self, principal: &str) -> CapabilityMenu {
        let tools: BTreeMap<String, String> = principal_tool_ceiling(
            &self.sources.lib,
            principal,
            self.sources.registered.as_ref(),
            self.sources.tools.as_ref(),
        )
        .unwrap_or_default()
        .into_iter()
        .collect();

        let skills = self
            .sources
            .skills
            .as_ref()
            .and_then(|c| c.list(principal, 0, None).ok())
            .map(|(records, _has_more)| records.into_iter().map(|r| r.name).collect())
            .unwrap_or_default();

        let connections = self
            .sources
            .connections
            .as_ref()
            .and_then(|c| c.list().ok())
            .map(|cs| cs.into_iter().map(|c| c.name).collect())
            .unwrap_or_default();

        // Only a dataset holding an indexed document can ground anything. Offering an empty
        // one would produce an App whose grounding silently retrieves nothing — the same
        // honesty rule the console's own dataset chips already apply.
        let datasets = self
            .sources
            .datasets
            .as_ref()
            .map(|d| {
                d.list_datasets()
                    .into_iter()
                    .filter(|s| s.doc_count > 0)
                    .map(|s| s.name)
                    .collect()
            })
            .unwrap_or_default();

        CapabilityMenu {
            tools,
            skills,
            connections,
            datasets,
        }
    }
}

#[tonic::async_trait]
impl AppDeriver for HostAppDeriver {
    async fn derive(&self, principal: &str, input: DeriveInput) -> AppDerivation {
        // Model inference is BLOCKING — run the whole render→decode→compile off the async
        // worker (the backend + catalog are cheap Arc clones; the join failure is honest).
        let backend = self.backend.clone();
        let model_id = self.model_id.clone();
        let exec_class = self.exec_class;
        let role_registry = self.role_registry.clone();
        let recipes = self.recipes.clone();
        let menu = self.menu(principal);
        match tokio::task::spawn_blocking(move || {
            derive_blocking(
                backend.as_ref(),
                &model_id,
                exec_class,
                role_registry.as_ref(),
                recipes.as_ref(),
                &menu,
                &input,
            )
        })
        .await
        {
            Ok(outcome) => outcome,
            Err(e) => rejected(&format!("the derive task failed: {e}")),
        }
    }
}

/// The synchronous core (generic over the backend so a stub can drive it in a unit test).
/// Validate-only; never mutates state.
fn derive_blocking<B: InferenceBackend>(
    backend: &B,
    model_id: &ModelId,
    exec_class: ExecutorClass,
    role_registry: &dyn RoleRegistry,
    recipes: &dyn RoleRecipeResolver,
    menu: &CapabilityMenu,
    input: &DeriveInput,
) -> AppDerivation {
    if input.prompt.trim().is_empty() {
        return rejected("describe what the app should do — the prompt is empty");
    }
    let hosted = match input.kind.trim() {
        "scheduled" | "" => false,
        "hosted" => true,
        other => {
            // Refuse rather than default: silently designing the wrong KIND of app is the one
            // mistake the author cannot see in the review.
            return rejected(&format!(
                "unknown app kind {other:?} (expected \"scheduled\" or \"hosted\")"
            ));
        }
    };

    let parent = shaper_warrant(model_id, exec_class);
    let params = InferenceParams {
        max_output_tokens: crate::env_caps::planner_max_output_tokens()
            .min(parent.model_route.max_output_tokens),
        ..InferenceParams::default()
    };
    let run = |system: &str, user: &str| -> Result<String, String> {
        let Some(rendered) = backend.render_chat(model_id, system, user) else {
            return Err(
                "the served model could not format the design prompt (start `kx serve` \
                        with an inference or serve-engine build and a resolved model)"
                    .to_string(),
            );
        };
        backend
            .dispatch(model_id, &InferenceInput::text(rendered), &params, &parent)
            .map(|out| String::from_utf8_lossy(&out.bytes).into_owned())
            .map_err(|e| format!("the model could not design this app: {e}"))
    };

    // (1) The design turn: the contract on the system channel, the palette + the bounded
    //     ids-only menu + the author's one prompt on the user channel.
    let (menu_block, truncation) = menu.render();
    let user = derive_user_message(&brief(input), &menu_block);
    let raw = match run(DERIVE_SYSTEM, &user) {
        Ok(r) => r,
        Err(e) => return rejected(&e),
    };
    let plan = match decode_derived(raw.as_bytes()) {
        Ok(p) => p,
        Err(e) => return rejected(&format!("the model did not return a usable design: {e}")),
    };

    // (2) The STRUCTURAL gate — the same `compile_plan` the proposer runs, against the same
    //     vetted catalog. Role resolution, warrant intersection (narrowing-only), acyclicity
    //     and critic precedence are all judged there, so the two authoring paths can never
    //     disagree about what is admissible.
    let projected = plan.to_plan();
    let seed = seed_from_plan_bytes(raw.as_bytes());
    if let Err(e) = compile_plan(&projected, seed, &parent, role_registry, recipes) {
        return rejected(&format!("the designed workflow is not admissible: {e}"));
    }

    // (3) The capability intersection — the enforcement point that makes naming safe.
    let (steps, mut derived) = intersect_capabilities(&plan, menu, recipes, &truncation);

    if hosted {
        // A hosted app has no DAG to run: the steps above were the model reasoning about the
        // work, and the thing the author needs to review is the project. Keep the design's
        // name/description, drop the steps, and plan the files through the contract that
        // already owns that job.
        let framework = resolve_framework(&input.framework, &mut derived.notices);
        derived.edges.clear();
        derived.framework = framework.to_string();
        // Drop every capability axis too. `hostsupervisor` launches a hosted app from its
        // framework + install/dev/build commands and reads no tool, skill, connection or
        // dataset — so returning the wishes the discarded steps implied would hand a client
        // grants it cannot use and a rail it cannot honour. The console does not author them
        // on this lane; saying so in the RESPONSE is what keeps any other client honest too.
        derived.tools.clear();
        derived.skills.clear();
        derived.connections.clear();
        derived.datasets.clear();
        let goal = format!("{}. {}", derived.description.trim(), input.prompt.trim());
        // The directive rides the SYSTEM channel with a short user turn, rather than the
        // directive as the user turn and an empty system: a chat template is only obliged to
        // render the roles it is given content for, and an empty system is the kind of input
        // that renders differently per architecture. Both channels non-empty is the shape every
        // other model turn on this path already uses.
        match run(
            &manifest_plan_directive(&goal, ScaffoldLane::Hosted(framework)),
            "Plan the files for this app.",
        )
        .and_then(|raw| decode_manifest(raw.as_bytes()).map_err(|e| e.to_string()))
        {
            Ok(files) => {
                derived.files = files
                    .into_iter()
                    .map(|f| DerivedFile {
                        path: f.path,
                        role: f.role,
                    })
                    .collect();
            }
            Err(e) => {
                // The design itself is sound; only the file plan is missing. Degrade to "the
                // scaffold will plan it" rather than throwing away a usable app design —
                // approving with no file list is exactly today's behaviour, which works.
                derived.notices.push(format!(
                    "the file plan could not be prepared ({e}) — the scaffold will plan the \
                     project when you create the app"
                ));
            }
        }
    } else {
        derived.steps = steps;
    }

    AppDerivation::Derived(Box::new(derived))
}

/// Turn a decoded design into the reviewable App by intersecting everything it NAMED against
/// the menu it was given.
///
/// Split out of `derive_blocking` because it is the part with a rule worth stating on its own:
/// each axis is resolved independently and each drop is REPORTED. One invented id costs only
/// itself — never the real grants beside it, and never silently.
fn intersect_capabilities(
    plan: &DerivedPlan,
    menu: &CapabilityMenu,
    recipes: &dyn RoleRecipeResolver,
    truncation: &MenuTruncation,
) -> (Vec<DerivedStep>, DerivedApp) {
    let mut notices = Vec::new();
    if truncation.tools_omitted > 0 {
        notices.push(format!(
            "{} more registered tools exist than fit the design prompt — attach any that are \
             missing below before creating the app",
            truncation.tools_omitted
        ));
    }

    if plan.folded_app_level {
        notices.push(
            "the design attached some capabilities to the app rather than to a step — they \
             were placed on the first step; open any step to move them"
                .to_string(),
        );
    }

    // Every axis is resolved PER STEP and unioned into the app-level sets. The union is the
    // DECLARATION a client writes into `references.*`; the per-step lists are the BINDINGS
    // the runtime resolves against it. Both travel because they answer different questions:
    // "what does this App need registered?" and "which node uses it?".
    let mut app_tools: BTreeMap<String, String> = BTreeMap::new();
    let mut app_skills: Vec<String> = Vec::new();
    let mut app_connections: Vec<String> = Vec::new();
    let mut app_datasets: Vec<String> = Vec::new();
    let mut dropped: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let steps: Vec<DerivedStep> = plan
        .steps
        .iter()
        .map(|s| {
            let (granted, tools_dropped) = menu.resolve(&s.tools);
            dropped.entry("tools").or_default().extend(tools_dropped);
            app_tools.extend(granted.iter().map(|(k, v)| (k.clone(), v.clone())));

            // Each axis independently: one invented name costs only itself, never the real
            // capabilities beside it and never the step.
            let mut resolve = |axis: &'static str, available: &[String], named: &[String]| {
                let (kept, gone) = CapabilityMenu::resolve_names(available, named);
                dropped.entry(axis).or_default().extend(gone);
                kept
            };
            let skills = resolve("skills", &menu.skills, &s.skills);
            let integrations = resolve("integrations", &menu.connections, &s.integrations);
            let datasets = resolve("datasets", &menu.datasets, &s.datasets);
            for (app, step) in [
                (&mut app_skills, &skills),
                (&mut app_connections, &integrations),
                (&mut app_datasets, &datasets),
            ] {
                for n in step {
                    if !app.contains(n) {
                        app.push(n.clone());
                    }
                }
            }

            let recipe = recipes.recipe(&RoleId(s.role.clone()));
            DerivedStep {
                role: s.role.clone(),
                intent: s.intent.clone(),
                // The derive contract authors no critics, so every step is structurally plain.
                // The kind still travels because the console maps it to a builder kind.
                kind: "plain".to_string(),
                model_id: recipe.map(|r| r.model_id.0.clone()).unwrap_or_default(),
                tool_contract: granted,
                skills,
                integrations,
                datasets,
            }
        })
        .collect();

    // Report each axis ONCE across the whole design, not once per step: the author wants to
    // know a name was unavailable, not which of four steps asked for it first.
    for (axis, mut names) in dropped {
        if names.is_empty() {
            continue;
        }
        names.sort_unstable();
        names.dedup();
        notices.push(if axis == "tools" {
            format!(
                "not attached — outside what this account can fire: {}",
                names.join(", ")
            )
        } else {
            format!("{axis} not found, so not attached: {}", names.join(", "))
        });
    }

    let derived = DerivedApp {
        name: plan.name.clone(),
        description: plan.description.clone(),
        edges: plan
            .edges
            .iter()
            .filter_map(|&(p, c)| Some((u32::try_from(p).ok()?, u32::try_from(c).ok()?)))
            .collect(),
        tools: app_tools,
        skills: app_skills,
        connections: app_connections,
        datasets: app_datasets,
        notices,
        ..DerivedApp::default()
    };
    (steps, derived)
}

/// Compose the author's brief: the prompt, plus the lane facts that change what a good design
/// looks like. Attachment FILENAMES ride because knowing the app will have `changelog.md` to
/// read changes the intents; the bytes do not, and the derive holds no grant to read them.
fn brief(input: &DeriveInput) -> String {
    let mut out = input.prompt.trim().to_string();
    if input.kind.trim() == "hosted" {
        out.push_str(
            "\n(This is a HOSTED web app — it is a served web page, not a scheduled automation.)",
        );
    } else if input.mode.trim() == "codified" {
        out.push_str(
            "\n(This app is CODIFIED: its behaviour comes from configuration and code the \
             runtime reads, not from prose alone.)",
        );
    }
    if !input.attachments.is_empty() {
        out.push_str("\nFiles the app can read: ");
        out.push_str(&input.attachments.join(", "));
    }
    out
}

/// Resolve the hosted framework to a real template, recording any substitution.
fn resolve_framework<'a>(requested: &'a str, notices: &mut Vec<String>) -> &'a str {
    let f = requested.trim();
    if f.is_empty() || f == "auto" {
        // `auto` is the author asking the runtime to choose. There is no framework-choice model
        // turn: the templates are equivalent for the apps this lane builds, and spending a whole
        // decode to pick between them would buy variety, not fitness. Say so rather than
        // implying a judgement was made.
        notices.push(format!(
            "framework: {DEFAULT_FRAMEWORK} (the default — pick one explicitly to change it)"
        ));
        return DEFAULT_FRAMEWORK;
    }
    if HOSTED_FRAMEWORKS.contains(&f) {
        return f;
    }
    notices.push(format!(
        "{f:?} is not an available template — using {DEFAULT_FRAMEWORK}"
    ));
    DEFAULT_FRAMEWORK
}

fn rejected(reason: &str) -> AppDerivation {
    AppDerivation::Rejected {
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::Duration;

    use kx_inference::{InferenceError, InferenceOutput};
    use kx_mote::{EffectPattern, LogicRef, ModelId, NdClass, PromptTemplateHash, ToolName};
    use kx_planner::{InMemoryRoleRecipes, RoleRecipe};
    use kx_warrant::{InMemoryRoleRegistry, Role};

    use super::*;

    /// A backend that answers each `dispatch` with the next scripted body. Nothing here reaches
    /// a real model — what is under test is the decode → compile → INTERSECT pipeline, which is
    /// where the design's capabilities are actually decided.
    struct ScriptedBackend {
        replies: Mutex<Vec<String>>,
    }

    impl ScriptedBackend {
        fn new(replies: &[&str]) -> Self {
            Self {
                replies: Mutex::new(replies.iter().rev().map(|s| (*s).to_string()).collect()),
            }
        }
    }

    impl InferenceBackend for ScriptedBackend {
        fn dispatch(
            &self,
            model_id: &ModelId,
            _input: &kx_inference::InferenceInput,
            _params: &InferenceParams,
            _warrant: &kx_warrant::WarrantSpec,
        ) -> Result<InferenceOutput, InferenceError> {
            let body = self
                .replies
                .lock()
                .expect("scripted replies")
                .pop()
                .unwrap_or_default();
            Ok(InferenceOutput {
                bytes: body.into_bytes(),
                output_tokens: 1,
                backend_name: "test",
                model_id: model_id.clone(),
                elapsed: Duration::from_millis(1),
            })
        }

        // The real path FORMATS through the model's own chat template; echoing is enough here
        // (the formatting is not what this pipeline decides).
        fn render_chat(&self, _model_id: &ModelId, system: &str, user: &str) -> Option<String> {
            Some(format!("{system}\n{user}"))
        }

        fn supports(&self, _model_id: &ModelId) -> bool {
            true
        }

        fn name(&self) -> &'static str {
            "test"
        }
    }

    fn model() -> ModelId {
        ModelId("test-model".into())
    }

    /// The same catalog shape `build_authoring_role_catalog` builds, constructed directly so the
    /// test needs no serve runtime.
    fn catalog() -> (InMemoryRoleRegistry, InMemoryRoleRecipes) {
        let warrant = shaper_warrant(&model(), ExecutorClass::Bwrap);
        let registry = InMemoryRoleRegistry::new();
        let recipes = InMemoryRoleRecipes::new();
        for role in crate::prompt_library::AUTHORING_ROLES {
            registry.register(
                RoleId(role.name.into()),
                Role {
                    name: role.name.into(),
                    version: 1,
                    spec: warrant.clone(),
                    description: String::new(),
                },
            );
            recipes.register(
                RoleId(role.name.into()),
                RoleRecipe {
                    logic_ref: LogicRef::from_bytes([0x77; 32]),
                    model_id: model(),
                    prompt_template_hash: PromptTemplateHash::from_bytes([0u8; 32]),
                    tool_contract: BTreeMap::new(),
                    capability: ToolName("kx-model".into()),
                    nd_class: NdClass::Pure,
                    effect_pattern: EffectPattern::IdempotentByConstruction,
                    inference_params: InferenceParams::default(),
                    deterministic_check: None,
                },
            );
        }
        (registry, recipes)
    }

    fn menu() -> CapabilityMenu {
        CapabilityMenu {
            tools: [
                ("mcp-echo/echo".to_string(), "1".to_string()),
                ("retrieve".to_string(), "1".to_string()),
            ]
            .into_iter()
            .collect(),
            skills: vec!["drafting".into()],
            connections: vec!["gmail".into()],
            datasets: vec!["handbook".into()],
        }
    }

    fn run(replies: &[&str], input: &DeriveInput, menu: &CapabilityMenu) -> AppDerivation {
        let backend = ScriptedBackend::new(replies);
        let (registry, recipes) = catalog();
        derive_blocking(
            &backend,
            &model(),
            ExecutorClass::Bwrap,
            &registry,
            &recipes,
            menu,
            input,
        )
    }

    fn scheduled(prompt: &str) -> DeriveInput {
        DeriveInput {
            kind: "scheduled".into(),
            mode: "codified".into(),
            prompt: prompt.into(),
            ..DeriveInput::default()
        }
    }

    fn derived(outcome: AppDerivation) -> DerivedApp {
        match outcome {
            AppDerivation::Derived(a) => *a,
            AppDerivation::Rejected { reason } => panic!("unexpected rejection: {reason}"),
        }
    }

    /// The per-STEP shape the contract now teaches: step 0 reaches a tool, step 1 carries
    /// the skill and the grounding, and the step that merely joins them asks for nothing.
    const FAN_OUT: &str = "{\"app\":{\"name\":\"Market Scan\",\"description\":\"Scans and \
reports.\",\"steps\":[{\"role\":\"researcher\",\"intent\":\"Gather pricing\",\"tools\":\
[\"mcp-echo/echo\"],\"skills\":[],\"integrations\":[],\"datasets\":[]},\
{\"role\":\"analyst\",\"intent\":\"Gather reviews\",\"tools\":[],\"skills\":[\"drafting\"],\
\"integrations\":[\"gmail\"],\"datasets\":[\"handbook\"]},\
{\"role\":\"writer\",\"intent\":\"Write the brief\",\"tools\":[],\"skills\":[],\
\"integrations\":[],\"datasets\":[]}],\"edges\":[{\"parent\":0,\
\"child\":2},{\"parent\":1,\"child\":2}]}}";

    /// The end-to-end shape claim: a fan-out survives decode, compile AND the mapping to the
    /// wire, so two steps with no incoming edge really do reach the console as parallel.
    #[test]
    fn a_parallel_design_survives_the_whole_pipeline() {
        let app = derived(run(&[FAN_OUT], &scheduled("scan the market"), &menu()));
        assert_eq!(app.steps.len(), 3);
        assert_eq!(app.edges, vec![(0, 2), (1, 2)]);
        let has_incoming = |i: u32| app.edges.iter().any(|&(_, c)| c == i);
        assert!(
            !has_incoming(0) && !has_incoming(1),
            "steps 0 and 1 must have no parent — that IS the parallelism"
        );
    }

    /// ★ The good-beside-bad rule, end to end. One invented tool id must cost only itself: the
    /// real grant on the same step survives, the app-level union keeps it, and the author is
    /// TOLD what was dropped rather than left to notice a missing capability at run time.
    #[test]
    fn an_invented_tool_costs_only_itself_and_is_reported() {
        let mixed = FAN_OUT.replace(
            "\"tools\":[\"mcp-echo/echo\"]",
            "\"tools\":[\"mcp-echo/echo\",\"definitely-not-registered\"]",
        );
        let app = derived(run(&[&mixed], &scheduled("scan the market"), &menu()));
        assert_eq!(
            app.steps[0].tool_contract.get("mcp-echo/echo"),
            Some(&"1".to_string()),
            "the real grant on the same step must survive its invented sibling"
        );
        assert!(!app.steps[0]
            .tool_contract
            .contains_key("definitely-not-registered"));
        assert_eq!(app.tools.get("mcp-echo/echo"), Some(&"1".to_string()));
        assert!(
            app.notices
                .iter()
                .any(|n| n.contains("definitely-not-registered")),
            "the drop must be told, not silent: {:?}",
            app.notices
        );
    }

    /// The version on a grant comes from the CEILING. A model writing a semver — the exact
    /// thing that broke the codified fold live — must still produce a usable grant.
    #[test]
    fn a_semver_version_from_the_model_is_replaced_by_the_ceilings() {
        let semver = FAN_OUT.replace("mcp-echo/echo", "mcp-echo/echo@1.0.0");
        let app = derived(run(&[&semver], &scheduled("scan"), &menu()));
        assert_eq!(
            app.steps[0].tool_contract.get("mcp-echo/echo"),
            Some(&"1".to_string())
        );
    }

    /// ★ CAPABILITIES LAND ON THE STEP THAT ASKED. Every axis is intersected per step, and
    /// the app-level lists are the UNION — the declaration set a client writes into
    /// `references`. A step that asked for nothing carries nothing: that discrimination is
    /// the whole reason the axes moved off the app.
    #[test]
    fn every_axis_binds_to_the_step_that_asked_and_the_app_carries_the_union() {
        let app = derived(run(&[FAN_OUT], &scheduled("scan the market"), &menu()));
        assert_eq!(app.steps.len(), 3);
        assert_eq!(app.steps[0].tool_contract.get("mcp-echo/echo"), Some(&"1".into()));
        assert_eq!(app.steps[1].skills, vec!["drafting".to_string()]);
        assert_eq!(app.steps[1].integrations, vec!["gmail".to_string()]);
        assert_eq!(app.steps[1].datasets, vec!["handbook".to_string()]);
        assert!(
            app.steps[0].skills.is_empty() && app.steps[2].skills.is_empty(),
            "a step that did not ask does not receive"
        );
        assert!(
            app.steps[2].tool_contract.is_empty()
                && app.steps[2].integrations.is_empty()
                && app.steps[2].datasets.is_empty(),
            "the joining step needs nothing"
        );
        // The union is the DECLARATION set — what must be registered for this design to run.
        assert_eq!(app.skills, vec!["drafting".to_string()]);
        assert_eq!(app.connections, vec!["gmail".to_string()]);
        assert_eq!(app.datasets, vec!["handbook".to_string()]);
        assert_eq!(app.tools.get("mcp-echo/echo"), Some(&"1".to_string()));
    }

    /// The good-beside-bad rule on the NON-tool axes: an invented skill costs only itself,
    /// the real one on the same step survives, and the drop is reported once for the whole
    /// design rather than once per step.
    #[test]
    fn an_invented_name_on_any_axis_costs_only_itself_and_is_reported_once() {
        let mixed = FAN_OUT.replace(
            "\"skills\":[\"drafting\"]",
            "\"skills\":[\"drafting\",\"no-such-skill\"]",
        );
        let app = derived(run(&[&mixed], &scheduled("scan"), &menu()));
        assert_eq!(app.steps[1].skills, vec!["drafting".to_string()]);
        assert_eq!(app.skills, vec!["drafting".to_string()]);
        assert_eq!(
            app.notices
                .iter()
                .filter(|n| n.contains("no-such-skill"))
                .count(),
            1,
            "one advisory for the design, not one per step: {:?}",
            app.notices
        );
    }

    /// A model answering in the OLD app-level shape still gets a usable design — the names
    /// fold onto the entry step (where `RunApp` would have bound them anyway) and the author
    /// is TOLD the design was adjusted rather than left to wonder why step 0 grew a skill.
    #[test]
    fn a_legacy_app_level_design_folds_onto_the_entry_step_with_a_notice() {
        const LEGACY: &str = "{\"app\":{\"name\":\"Scan\",\"description\":\"d\",\"steps\":[\
{\"role\":\"researcher\",\"intent\":\"a\",\"tools\":[]},\
{\"role\":\"writer\",\"intent\":\"b\",\"tools\":[]}],\"edges\":[{\"parent\":0,\"child\":1}],\
\"skills\":[\"drafting\"],\"datasets\":[\"handbook\"]}}";
        let app = derived(run(&[LEGACY], &scheduled("scan"), &menu()));
        assert_eq!(app.steps[0].skills, vec!["drafting".to_string()]);
        assert_eq!(app.steps[0].datasets, vec!["handbook".to_string()]);
        assert!(app.steps[1].skills.is_empty());
        assert!(
            app.notices.iter().any(|n| n.contains("rather than to a step")),
            "the adjustment must be told: {:?}",
            app.notices
        );
    }

    /// A design naming a capability on a serve that offers NONE must still produce the app —
    /// with an empty grant set and a notice. A refusal here would make an unconfigured serve
    /// unable to author anything.
    #[test]
    fn an_empty_menu_yields_an_app_with_no_grants_rather_than_a_refusal() {
        let app = derived(run(
            &[FAN_OUT],
            &scheduled("scan"),
            &CapabilityMenu::default(),
        ));
        assert_eq!(app.steps.len(), 3);
        assert!(app.tools.is_empty());
        assert!(app.skills.is_empty());
        assert!(app.notices.iter().any(|n| n.contains("mcp-echo/echo")));
    }

    /// The HOSTED lane reviews FILES, not a DAG. The steps the design turn produced are
    /// dropped on purpose (a hosted app has no workflow to run) and the manifest turn's files
    /// take their place.
    #[test]
    fn the_hosted_lane_returns_files_and_no_steps() {
        const MANIFEST: &str = "{\"manifest\":{\"version\":1,\"files\":[\
{\"path\":\"src/App.tsx\",\"role\":\"the root component\"},\
{\"path\":\"src/App.css\",\"role\":\"its styles\"}]}}";
        let app = derived(run(
            &[FAN_OUT, MANIFEST],
            &DeriveInput {
                kind: "hosted".into(),
                prompt: "a pomodoro timer".into(),
                ..DeriveInput::default()
            },
            &menu(),
        ));
        assert!(app.steps.is_empty(), "a hosted app has no DAG");
        assert!(app.edges.is_empty());
        // ...and no capability wishes either. The design turn reasoned about tools (its
        // researcher step asked for one); the hosted supervisor reads none, so carrying them
        // would be a grant the lane cannot use and a rail it cannot honour.
        assert!(app.tools.is_empty(), "a hosted app carries no tool wishes");
        assert!(app.skills.is_empty());
        assert!(app.connections.is_empty());
        assert!(app.datasets.is_empty());
        assert_eq!(app.framework, DEFAULT_FRAMEWORK);
        assert_eq!(
            app.files
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>(),
            vec!["src/App.tsx", "src/App.css"]
        );
    }

    /// A hosted file plan that fails to decode must not throw away a usable app design — it
    /// degrades to "the scaffold will plan it", which is exactly today's working behaviour.
    #[test]
    fn a_failed_hosted_file_plan_degrades_instead_of_rejecting() {
        let app = derived(run(
            &[FAN_OUT, "sorry, I cannot do that"],
            &DeriveInput {
                kind: "hosted".into(),
                prompt: "a pomodoro timer".into(),
                ..DeriveInput::default()
            },
            &menu(),
        ));
        assert!(app.files.is_empty());
        assert!(!app.name.is_empty(), "the design itself survived");
        assert!(app.notices.iter().any(|n| n.contains("file plan")));
    }

    #[test]
    fn a_pinned_framework_is_honoured_and_an_invented_one_is_replaced() {
        let mut notices = Vec::new();
        assert_eq!(resolve_framework("next_js", &mut notices), "next_js");
        assert!(notices.is_empty(), "a valid pin needs no notice");
        assert_eq!(resolve_framework("solid", &mut notices), DEFAULT_FRAMEWORK);
        assert!(notices.iter().any(|n| n.contains("solid")));
        notices.clear();
        assert_eq!(resolve_framework("auto", &mut notices), DEFAULT_FRAMEWORK);
        assert_eq!(resolve_framework("", &mut notices), DEFAULT_FRAMEWORK);
    }

    #[test]
    fn refuses_an_empty_prompt_and_an_unknown_kind() {
        assert!(matches!(
            run(&[FAN_OUT], &scheduled("   "), &menu()),
            AppDerivation::Rejected { .. }
        ));
        let bad_kind = DeriveInput {
            kind: "workflow".into(),
            prompt: "do a thing".into(),
            ..DeriveInput::default()
        };
        match run(&[FAN_OUT], &bad_kind, &menu()) {
            AppDerivation::Rejected { reason } => assert!(reason.contains("workflow")),
            AppDerivation::Derived(_) => panic!("an unknown kind must never be defaulted"),
        }
    }

    /// A role outside the vetted palette is refused by the SAME `compile_plan` gate the
    /// proposer runs — the derive adds a capability axis, it does not add a way in.
    #[test]
    fn an_unvetted_role_is_refused_by_the_shared_compile_gate() {
        let bad_role = FAN_OUT.replace("\"role\":\"researcher\"", "\"role\":\"root\"");
        match run(&[&bad_role], &scheduled("scan"), &menu()) {
            AppDerivation::Rejected { reason } => assert!(
                reason.contains("not admissible"),
                "must be refused by the compile gate: {reason}"
            ),
            AppDerivation::Derived(_) => panic!("an unvetted role must not be designable"),
        }
    }

    /// The lane facts reach the model: a codified app is told it is codified, a hosted app is
    /// told it is a web page, and attachment FILENAMES ride (their bytes never do).
    #[test]
    fn the_brief_carries_the_lane_facts_and_filenames_only() {
        let mut input = scheduled("summarise the changelog");
        input.attachments = vec!["changelog.md".into()];
        let b = brief(&input);
        assert!(b.contains("summarise the changelog"));
        assert!(b.contains("CODIFIED"));
        assert!(b.contains("changelog.md"));
        let hosted = DeriveInput {
            kind: "hosted".into(),
            prompt: "a timer".into(),
            ..DeriveInput::default()
        };
        assert!(brief(&hosted).contains("HOSTED"));
        assert!(!brief(&hosted).contains("CODIFIED"));
    }
}
