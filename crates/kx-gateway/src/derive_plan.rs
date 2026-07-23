//! `DeriveApp`'s model-output contract: the ids-only capability MENU handed to the model, the
//! fail-closed decoder for what it writes back, and the intersection that turns a named tool id
//! into a wish the runtime will honour.
//!
//! **The gap this closes.** Nothing in the runtime derived a tool contract from a goal.
//! `ProposeWorkflow` structurally cannot: it takes `tool_contract` from the vetted recipe, and
//! every authoring role resolves to a pure model recipe whose contract is empty — so every plan
//! a user has ever previewed had zero tools, and the only way to plug an App in was to create it
//! and attach afterwards.
//!
//! **Why letting the model name a tool is not a widening (SN-8).** The menu is computed
//! HOST-side from `app_run::principal_tool_ceiling` (the caller's party authority ∩ the
//! broker-fireable set ∩ the durable registry) — the same ceiling `GetAppManifest` reports
//! against and `RunApp` intersects at fire. Every id the model returns is intersected back
//! against that ceiling, and what survives is written as a `requested_grants` WISH, which
//! `app_run` intersects AGAIN at run. The model can therefore only ask for something the
//! caller could already have clicked in `ToolsPicker`; it can never grant, and it can never
//! reach past the ceiling. What is new is the ASKING, not the authority.
//!
//! **Why the menu is ids-only and bounded.** The whole exchange is ONE decode — the serve does
//! not chunk a prompt ([`crate::scaffold`]'s `SIBLING_CONTEXT_MAX` reasoning), so an oversize
//! prompt does not degrade, it aborts. Descriptions are dropped, the menu is byte-bounded, and
//! what did not fit is REPORTED rather than silently omitted: a menu that quietly shows half the
//! registry produces an App that quietly cannot do half its job.

use std::collections::{BTreeMap, BTreeSet};

use kx_planner::{Plan, PlanEdge, PlanStep, PlanStepKind};
use serde::Deserialize;

use crate::manifest::strip_json_wrappers;

/// Hard cap on the derive payload BEFORE parse. Shares the manifest ceiling's reasoning: a
/// bounded model answer is a parse bound, not a security one (the security is the intersection).
pub(crate) const MAX_DERIVE_BYTES: usize = 64 * 1024;
/// Hard cap on the steps a derived app may declare. Each step is a model call at run, so this
/// bounds a single scheduled fire. Deliberately below the codified `workflow.json` ceiling (24):
/// the contract asks for the SMALLEST workflow, and this is the fail-closed answer to a model
/// that ignores it.
pub(crate) const MAX_DERIVE_STEPS: usize = 8;
/// Hard cap on tools ONE step may request. A step reaching for more than a handful of tools is
/// a step that has not been decomposed.
pub(crate) const MAX_TOOLS_PER_STEP: usize = 6;
/// Hard cap on entries in ONE app-level name list (skills / integrations / datasets). An app
/// reaching for a dozen skills has not been scoped, and each entry costs a real resolution.
pub(crate) const MAX_APP_LEVEL_NAMES: usize = 8;
/// Hard cap on the app name the model proposes (the console shows it in a `maxLength={80}`
/// field, and the handle is derived from it).
pub(crate) const MAX_DERIVE_NAME_BYTES: usize = 80;
/// Hard cap on the proposed one-sentence description.
pub(crate) const MAX_DERIVE_DESCRIPTION_BYTES: usize = 400;
/// Byte budget for the rendered capability menu. Sized so the menu, the role palette, the
/// contract and the user's prompt together stay comfortably inside one decode on the smallest
/// context the serve accepts (`AGENT_MIN_CTX_TOKENS` = 2048).
pub(crate) const MAX_DERIVE_MENU_BYTES: usize = 2 * 1024;

/// A derive that did not decode. Its own type rather than a reuse of the manifest/codified
/// errors: these messages reach the author as the reason their app was not designed, and every
/// one of those strings names a different file.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum DeriveError {
    /// The payload exceeded [`MAX_DERIVE_BYTES`] before parsing.
    #[error("the design is oversize: {got} bytes > max {MAX_DERIVE_BYTES}")]
    Oversize {
        /// The payload byte length.
        got: usize,
    },
    /// The payload was not valid UTF-8.
    #[error("the design was not valid UTF-8")]
    NotUtf8,
    /// The payload did not parse, or was not the shape the runtime consumes.
    #[error("the design is malformed: {0}")]
    Malformed(String),
}

// ---------------------------------------------------------------------------
// The wire shape the model writes. `deny_unknown_fields` throughout: a model that
// invents an axis (a permission, a model id) is REFUSED, not quietly trimmed — the
// refusal is what keeps the trust surface the size the contract advertises.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeriveEnvelope {
    app: AppJson,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AppJson {
    name: String,
    #[serde(default)]
    description: String,
    steps: Vec<StepJson>,
    #[serde(default)]
    edges: Vec<EdgeJson>,
    // The app-level capability axes. All `default`: a model that omits one has said "none",
    // which is the common case and must not be a decode failure.
    #[serde(default)]
    skills: Vec<String>,
    #[serde(default)]
    integrations: Vec<String>,
    #[serde(default)]
    datasets: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StepJson {
    role: String,
    intent: String,
    #[serde(default)]
    tools: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EdgeJson {
    parent: usize,
    child: usize,
}

/// One decoded step: what the model asked for, before any intersection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DerivedStep {
    /// The palette role this step plays (resolved against the vetted catalog downstream).
    pub(crate) role: String,
    /// The model's free-form per-step instruction.
    pub(crate) intent: String,
    /// The tool ids this step requested, in the order named (deduped, still UNRESOLVED).
    pub(crate) tools: Vec<String>,
}

/// A decoded derive: the app's proposed identity plus its unresolved workflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DerivedPlan {
    /// The proposed app name (the console derives the handle from it).
    pub(crate) name: String,
    /// The proposed one-sentence description.
    pub(crate) description: String,
    /// The steps, in plan order.
    pub(crate) steps: Vec<DerivedStep>,
    /// The `(parent, child)` dependency edges. A step with no incoming edge runs in parallel.
    pub(crate) edges: Vec<(usize, usize)>,
    /// App-level skill names the design asked for (still UNRESOLVED).
    pub(crate) skills: Vec<String>,
    /// App-level connection descriptors the design asked for (still UNRESOLVED).
    pub(crate) integrations: Vec<String>,
    /// App-level dataset names the design asked for (still UNRESOLVED).
    pub(crate) datasets: Vec<String>,
}

impl DerivedPlan {
    /// Project onto the `kx_planner::Plan` the VETTED path judges.
    ///
    /// The derive's extra axis (per-step tools) rides OUTSIDE this projection deliberately:
    /// admissibility — role resolution, warrant intersection, acyclicity, critic precedence —
    /// is decided by the same `compile_plan` gate `ProposeWorkflow` runs, against the same
    /// catalog. Judging it here instead would be a second definition of "admissible", which is
    /// how the two authoring paths would come to disagree.
    pub(crate) fn to_plan(&self) -> Plan {
        Plan {
            version: 1,
            steps: self
                .steps
                .iter()
                .map(|s| PlanStep {
                    role: s.role.clone(),
                    intent: s.intent.clone(),
                    kind: PlanStepKind::Plain,
                    producer: None,
                })
                .collect(),
            edges: self
                .edges
                .iter()
                .map(|&(parent, child)| PlanEdge { parent, child })
                .collect(),
        }
    }
}

/// Decode a model-authored app design, fail-closed. Total + panic-free over arbitrary `bytes`.
///
/// Takes the [`crate::manifest::decode_manifest`] posture: strip the fence / reasoning block a
/// chatty model wraps its answer in, bound the payload before parse, refuse any shape that is
/// not exactly what the contract asked for, and say what was wrong in terms the author can act
/// on.
///
/// # Errors
/// [`DeriveError`] when the payload is oversize, not UTF-8, not the `{"app":{…}}` envelope,
/// declares no steps or more than [`MAX_DERIVE_STEPS`], names an empty role or intent, or
/// carries an edge index that is not a step.
pub(crate) fn decode_derived(bytes: &[u8]) -> Result<DerivedPlan, DeriveError> {
    if bytes.len() > MAX_DERIVE_BYTES {
        return Err(DeriveError::Oversize { got: bytes.len() });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| DeriveError::NotUtf8)?;
    let text = strip_json_wrappers(text);
    let env: DeriveEnvelope =
        serde_json::from_str(text).map_err(|e| DeriveError::Malformed(e.to_string()))?;
    let app = env.app;

    if app.steps.is_empty() {
        return Err(DeriveError::Malformed(
            "declares no steps (an app with no steps has nothing to run)".into(),
        ));
    }
    if app.steps.len() > MAX_DERIVE_STEPS {
        return Err(DeriveError::Malformed(format!(
            "declares {} steps, over the {MAX_DERIVE_STEPS} ceiling",
            app.steps.len()
        )));
    }

    let mut steps = Vec::with_capacity(app.steps.len());
    for (i, s) in app.steps.into_iter().enumerate() {
        let role = s.role.trim().to_string();
        let intent = s.intent.trim().to_string();
        if role.is_empty() {
            return Err(DeriveError::Malformed(format!("step {i} names no role")));
        }
        if intent.is_empty() {
            return Err(DeriveError::Malformed(format!(
                "step {i} ({role}) has an empty intent — nothing for the role to do"
            )));
        }
        // Dedupe while preserving the order named: a model that lists the same tool twice has
        // still asked for it once, and refusing that would be pedantry the author pays for.
        let mut seen = BTreeSet::new();
        let tools: Vec<String> = s
            .tools
            .into_iter()
            .filter_map(|t| {
                let t = normalize_tool_id(&t);
                (!t.is_empty() && seen.insert(t.clone())).then_some(t)
            })
            .take(MAX_TOOLS_PER_STEP)
            .collect();
        steps.push(DerivedStep {
            role,
            intent,
            tools,
        });
    }

    // An edge naming a step that does not exist is a decode failure, not something to drop:
    // the shape the model returned is not the shape it claims, and silently repairing it would
    // hand the author a workflow they never saw.
    let mut edges = Vec::with_capacity(app.edges.len());
    for e in &app.edges {
        if e.parent >= steps.len() || e.child >= steps.len() {
            return Err(DeriveError::Malformed(format!(
                "edge {}→{} names a step outside the {} declared",
                e.parent,
                e.child,
                steps.len()
            )));
        }
        if e.parent == e.child {
            return Err(DeriveError::Malformed(format!(
                "step {} depends on itself",
                e.parent
            )));
        }
        edges.push((e.parent, e.child));
    }
    edges.sort_unstable();
    edges.dedup();

    Ok(DerivedPlan {
        name: clamp_chars(app.name.trim(), MAX_DERIVE_NAME_BYTES),
        description: clamp_chars(app.description.trim(), MAX_DERIVE_DESCRIPTION_BYTES),
        steps,
        edges,
        skills: clean_names(app.skills),
        integrations: clean_names(app.integrations),
        datasets: clean_names(app.datasets),
    })
}

/// Trim, drop empties, dedupe, and bound one app-level name list. Bounded by
/// [`MAX_TOOLS_PER_STEP`]'s sibling reasoning: an app reaching for a dozen skills has not been
/// scoped, and each entry costs a real resolution downstream.
fn clean_names(raw: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    raw.into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && seen.insert(s.clone()))
        .take(MAX_APP_LEVEL_NAMES)
        .collect()
}

/// Normalize a model-named tool id to the bare id the ceiling is keyed by.
///
/// The menu asks for a BARE id and resolves the version host-side, because a model asked for a
/// version writes a semver (`"1.0.0"`) where the envelope requires an integer — the exact
/// malformed-version failure the codified fold hit live. But a model does not only answer the
/// question it was asked: it copies what it was SHOWN.
///
/// **Found live.** The menu rendered `- retrieve (v1)` and Gemma-4-12B returned `"retrieve (v1)"`
/// as the id. It had correctly chosen a tool the caller could fire, and the intersection dropped
/// it — reporting the drop as an authority problem when it was a formatting one. The menu no
/// longer shows a version at all (see [`CapabilityMenu::render`]); this strips both the
/// `id@version` and the ` (v…)` forms anyway, because tolerating a shape the prompt no longer
/// teaches costs nothing and silently losing a real grant costs the App part of its job.
fn normalize_tool_id(raw: &str) -> String {
    let mut t = raw.trim();
    // ` (v1)` / `(1)` — the display parenthetical.
    if let Some((id, rest)) = t.split_once('(') {
        if rest.trim_end().ends_with(')') {
            t = id.trim();
        }
    }
    // `id@version`.
    match t.split_once('@') {
        Some((id, _version)) => id.trim().to_string(),
        None => t.trim().to_string(),
    }
}

/// Truncate to `max` BYTES on a char boundary (never mid-UTF-8).
fn clamp_chars(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].trim_end().to_string()
}

// ---------------------------------------------------------------------------
// The capability menu.
// ---------------------------------------------------------------------------

/// The ids-only capability menu handed to the derive model, plus an honest record of what did
/// not fit.
///
/// Every axis is NAMES ONLY. Descriptions would be the useful thing to include and are exactly
/// what the one-decode budget cannot afford, so they are omitted rather than half-included.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CapabilityMenu {
    /// The caller's resolvable tool ceiling as `id → version`.
    pub(crate) tools: BTreeMap<String, String>,
    /// Catalog skill names.
    pub(crate) skills: Vec<String>,
    /// Registered connection descriptors.
    pub(crate) connections: Vec<String>,
    /// Dataset names that actually hold an indexed document.
    pub(crate) datasets: Vec<String>,
}

/// What the menu could not show, so the console can say so instead of implying completeness.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct MenuTruncation {
    /// Tool ids omitted for the byte budget.
    pub(crate) tools_omitted: usize,
}

impl CapabilityMenu {
    /// Render the menu block for the derive prompt, bounded to [`MAX_DERIVE_MENU_BYTES`].
    ///
    /// Tools are the only axis the model may NAME, so they are rendered first and get whatever
    /// budget remains; the other axes are informational context that shapes the intents. When
    /// the budget runs out mid-tool-list the render STOPS on a whole entry and reports the
    /// count dropped — a half-written id is worse than an absent one, because the model would
    /// name it and the intersection would silently drop it.
    pub(crate) fn render(&self) -> (String, MenuTruncation) {
        let mut out = String::from("Capability menu — the ONLY tool ids you may name:\n");
        let mut truncation = MenuTruncation::default();
        if self.tools.is_empty() {
            out.push_str("- (no tools are available to this account; use an empty tools list)\n");
        }
        let mut omitted = 0usize;
        for id in self.tools.keys() {
            // The id ALONE. Showing `(v1)` here made Gemma-4-12B return `"retrieve (v1)"` as the
            // id, which matched nothing and dropped a grant the caller could really fire. The
            // version is resolved host-side from the ceiling and the contract forbids naming
            // one, so rendering it was decoration that cost a capability.
            let line = format!("- {id}\n");
            if out.len() + line.len() > MAX_DERIVE_MENU_BYTES {
                omitted += 1;
                continue;
            }
            out.push_str(&line);
        }
        truncation.tools_omitted = omitted;
        // The remaining axes are CONTEXT, not a naming surface — they tell the model what this
        // app can be grounded in and connected to, which changes the intents it writes. Each is
        // a single joined line and is dropped whole if it does not fit.
        for (label, values) in [
            ("Skills available", &self.skills),
            ("Integrations connected", &self.connections),
            ("Datasets to ground on", &self.datasets),
        ] {
            if values.is_empty() {
                continue;
            }
            let line = format!("{label}: {}\n", values.join(", "));
            if out.len() + line.len() <= MAX_DERIVE_MENU_BYTES {
                out.push_str(&line);
            }
        }
        (out, truncation)
    }

    /// Intersect one step's named tool ids against the menu, returning the grant map and the
    /// ids that were dropped.
    ///
    /// This is the enforcement point that makes naming safe. It is deliberately PER STEP and
    /// returns the drops rather than failing: a step that names two real tools and one
    /// hallucinated one keeps both real ones. Losing a whole good step to one bad sibling is
    /// precisely the failure the codified fold shipped and a live proof caught.
    pub(crate) fn resolve(&self, named: &[String]) -> (BTreeMap<String, String>, Vec<String>) {
        let mut granted = BTreeMap::new();
        let mut dropped = Vec::new();
        for id in named {
            match self.tools.get(id) {
                Some(version) => {
                    granted.insert(id.clone(), version.clone());
                }
                None => dropped.push(id.clone()),
            }
        }
        (granted, dropped)
    }

    /// Intersect an app-level name list (skills / integrations / datasets) against the menu
    /// axis it was drawn from, returning what survived and what was dropped.
    ///
    /// Same posture as [`Self::resolve`] and for the same reason: a design naming two real
    /// skills and one invented one keeps both real ones. Case-insensitive on the way in — the
    /// menu shows the canonical name and the CANONICAL spelling is what is returned, so a model
    /// that lower-cased a name still gets the grant, and the caller never receives a name its
    /// own catalog would not recognise.
    pub(crate) fn resolve_names(
        available: &[String],
        named: &[String],
    ) -> (Vec<String>, Vec<String>) {
        let mut kept = Vec::new();
        let mut dropped = Vec::new();
        for n in named {
            match available
                .iter()
                .find(|a| a.eq_ignore_ascii_case(n.as_str()))
            {
                Some(canonical) => {
                    if !kept.contains(canonical) {
                        kept.push(canonical.clone());
                    }
                }
                None => dropped.push(n.clone()),
            }
        }
        (kept, dropped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "{\"app\":{\"name\":\"Release Notes Writer\",\"description\":\"Turns a \
changelog into release notes.\",\"steps\":[{\"role\":\"researcher\",\"intent\":\"Gather the \
merged changes\",\"tools\":[\"mcp-echo/echo\"]},{\"role\":\"writer\",\"intent\":\"Write the \
notes\",\"tools\":[]}],\"edges\":[{\"parent\":0,\"child\":1}]}}";

    fn menu() -> CapabilityMenu {
        CapabilityMenu {
            tools: [
                ("mcp-echo/echo".to_string(), "1".to_string()),
                ("retrieve".to_string(), "1".to_string()),
            ]
            .into_iter()
            .collect(),
            skills: vec!["classification".into()],
            connections: vec!["gmail".into()],
            datasets: vec!["handbook".into()],
        }
    }

    #[test]
    fn decodes_the_taught_envelope() {
        let d = decode_derived(GOOD.as_bytes()).expect("the contract's own shape must decode");
        assert_eq!(d.name, "Release Notes Writer");
        assert_eq!(d.steps.len(), 2);
        assert_eq!(d.steps[0].tools, vec!["mcp-echo/echo".to_string()]);
        assert!(d.steps[1].tools.is_empty());
        assert_eq!(d.edges, vec![(0, 1)]);
    }

    /// A fenced / reasoning-wrapped answer is the normal case on a reasoning model, not an edge
    /// case — it must decode through the same wrapper strip the manifest path uses.
    #[test]
    fn decodes_through_a_fence_and_a_reasoning_block() {
        let wrapped = format!("<think>The user wants release notes.</think>\n```json\n{GOOD}\n```");
        assert_eq!(
            decode_derived(wrapped.as_bytes()).expect("wrapped answer must decode"),
            decode_derived(GOOD.as_bytes()).unwrap()
        );
    }

    /// PARALLEL is the shape this whole contract exists to make reachable: two steps with no
    /// edge between them and a join. It must survive decode as written — no implicit chaining.
    #[test]
    fn a_fan_out_decodes_as_a_fan_out() {
        let fan = "{\"app\":{\"name\":\"Market Scan\",\"description\":\"d\",\"steps\":[\
{\"role\":\"researcher\",\"intent\":\"a\",\"tools\":[]},\
{\"role\":\"analyst\",\"intent\":\"b\",\"tools\":[]},\
{\"role\":\"writer\",\"intent\":\"c\",\"tools\":[]}],\
\"edges\":[{\"parent\":0,\"child\":2},{\"parent\":1,\"child\":2}]}}";
        let d = decode_derived(fan.as_bytes()).expect("a fan-out must decode");
        assert_eq!(d.edges, vec![(0, 2), (1, 2)]);
        // Steps 0 and 1 have no incoming edge — that IS the parallelism.
        let has_incoming = |i: usize| d.edges.iter().any(|&(_, c)| c == i);
        assert!(!has_incoming(0) && !has_incoming(1));
        assert!(has_incoming(2));
    }

    #[test]
    fn refuses_the_shapes_the_contract_forbids() {
        // An invented axis (the model naming a permission or a model id) is REFUSED.
        let extra = GOOD.replace("\"tools\":[]", "\"tools\":[],\"model_id\":\"gemma\"");
        assert!(matches!(
            decode_derived(extra.as_bytes()),
            Err(DeriveError::Malformed(_))
        ));
        // No steps.
        let none = "{\"app\":{\"name\":\"n\",\"description\":\"d\",\"steps\":[],\"edges\":[]}}";
        assert!(matches!(
            decode_derived(none.as_bytes()),
            Err(DeriveError::Malformed(_))
        ));
        // An edge naming a step that does not exist.
        let bad_edge = GOOD.replace("{\"parent\":0,\"child\":1}", "{\"parent\":0,\"child\":9}");
        assert!(matches!(
            decode_derived(bad_edge.as_bytes()),
            Err(DeriveError::Malformed(_))
        ));
        // A self-edge.
        let self_edge = GOOD.replace("{\"parent\":0,\"child\":1}", "{\"parent\":1,\"child\":1}");
        assert!(matches!(
            decode_derived(self_edge.as_bytes()),
            Err(DeriveError::Malformed(_))
        ));
        // Oversize, before any parse.
        let big = vec![b'x'; MAX_DERIVE_BYTES + 1];
        assert!(matches!(
            decode_derived(&big),
            Err(DeriveError::Oversize { .. })
        ));
        // Not UTF-8.
        assert_eq!(decode_derived(&[0xff, 0xfe]), Err(DeriveError::NotUtf8));
    }

    #[test]
    fn refuses_more_steps_than_the_ceiling() {
        let step = "{\"role\":\"writer\",\"intent\":\"x\",\"tools\":[]}";
        let steps = [step; MAX_DERIVE_STEPS + 1].join(",");
        let over = format!(
            "{{\"app\":{{\"name\":\"n\",\"description\":\"d\",\"steps\":[{steps}],\"edges\":[]}}}}"
        );
        assert!(matches!(
            decode_derived(over.as_bytes()),
            Err(DeriveError::Malformed(_))
        ));
    }

    /// ★ The good-beside-bad rule. A step naming two real tools and one that is not on the menu
    /// keeps BOTH real ones. Validating the set as a unit is how the codified fold lost a good
    /// three-step workflow to a sibling's bad version string.
    #[test]
    fn one_hallucinated_tool_does_not_cost_its_real_siblings() {
        let (granted, dropped) = menu().resolve(&[
            "mcp-echo/echo".into(),
            "totally-made-up".into(),
            "retrieve".into(),
        ]);
        assert_eq!(granted.len(), 2, "both real tools survive");
        assert_eq!(granted.get("mcp-echo/echo"), Some(&"1".to_string()));
        assert_eq!(granted.get("retrieve"), Some(&"1".to_string()));
        assert_eq!(dropped, vec!["totally-made-up".to_string()]);
    }

    /// The version comes from the CEILING, never from the model — which is why an `id@version`
    /// answer is accepted and its version discarded rather than refused.
    #[test]
    fn a_versioned_id_resolves_to_the_ceilings_version() {
        let d = decode_derived(
            GOOD.replace("mcp-echo/echo", "mcp-echo/echo@1.0.0")
                .as_bytes(),
        )
        .expect("the generous form decodes");
        assert_eq!(d.steps[0].tools, vec!["mcp-echo/echo".to_string()]);
        let (granted, dropped) = menu().resolve(&d.steps[0].tools);
        assert!(dropped.is_empty());
        assert_eq!(granted.get("mcp-echo/echo"), Some(&"1".to_string()));
    }

    /// ★ THE LIVE FINDING. The menu once rendered `- retrieve (v1)`, and Gemma-4-12B returned
    /// `"retrieve (v1)"` as the id — a tool the caller could genuinely fire, dropped by a
    /// formatting mismatch and then reported as an authority problem. Every unit test passed,
    /// because every one of them fed a bare id or `id@version`.
    #[test]
    fn a_display_parenthetical_echoed_back_as_an_id_still_resolves() {
        let d = decode_derived(
            GOOD.replace("\"mcp-echo/echo\"", "\"mcp-echo/echo (v1)\"")
                .as_bytes(),
        )
        .expect("the echoed display form decodes");
        assert_eq!(d.steps[0].tools, vec!["mcp-echo/echo".to_string()]);
        let (granted, dropped) = menu().resolve(&d.steps[0].tools);
        assert!(dropped.is_empty(), "the grant must survive: {dropped:?}");
        assert_eq!(granted.get("mcp-echo/echo"), Some(&"1".to_string()));
    }

    #[test]
    fn duplicate_tool_names_collapse_and_the_per_step_cap_holds() {
        let dup = GOOD.replace(
            "\"tools\":[\"mcp-echo/echo\"]",
            "\"tools\":[\"mcp-echo/echo\",\"mcp-echo/echo\"]",
        );
        assert_eq!(
            decode_derived(dup.as_bytes()).unwrap().steps[0].tools.len(),
            1
        );
        let many: Vec<String> = (0..MAX_TOOLS_PER_STEP + 4)
            .map(|i| format!("\"t{i}\""))
            .collect();
        let over = GOOD.replace(
            "\"tools\":[\"mcp-echo/echo\"]",
            &format!("\"tools\":[{}]", many.join(",")),
        );
        assert_eq!(
            decode_derived(over.as_bytes()).unwrap().steps[0]
                .tools
                .len(),
            MAX_TOOLS_PER_STEP
        );
    }

    #[test]
    fn the_menu_renders_ids_only_and_within_budget() {
        let (rendered, truncation) = menu().render();
        // The id ALONE — no version parenthetical. Found live: showing one made the model
        // return it AS the id.
        assert!(rendered.contains("- mcp-echo/echo\n"));
        assert!(
            !rendered.contains("(v1)"),
            "a version parenthetical invites an unusable id"
        );
        assert!(rendered.contains("Skills available: classification"));
        assert!(rendered.contains("Integrations connected: gmail"));
        assert!(rendered.contains("Datasets to ground on: handbook"));
        assert!(rendered.len() <= MAX_DERIVE_MENU_BYTES);
        assert_eq!(truncation.tools_omitted, 0);
    }

    /// A registry bigger than the budget must bound the PROMPT and report the shortfall — never
    /// emit a half-written id the model would then name into a silent drop.
    #[test]
    fn an_oversize_registry_bounds_the_prompt_and_reports_the_shortfall() {
        let tools: BTreeMap<String, String> = (0..500)
            .map(|i| {
                (
                    format!("server-{i:03}/some-fairly-long-tool-name"),
                    "1".into(),
                )
            })
            .collect();
        let (rendered, truncation) = CapabilityMenu {
            tools,
            ..Default::default()
        }
        .render();
        assert!(rendered.len() <= MAX_DERIVE_MENU_BYTES);
        assert!(truncation.tools_omitted > 0, "the shortfall must be told");
        // Every line that DID make it is whole (a truncated id would be named and then dropped).
        for line in rendered.lines().filter(|l| l.starts_with("- ")) {
            assert!(
                line.ends_with("some-fairly-long-tool-name"),
                "truncated mid-entry: {line:?}"
            );
        }
    }

    #[test]
    fn an_empty_menu_says_so_rather_than_showing_nothing() {
        let (rendered, _) = CapabilityMenu::default().render();
        assert!(rendered.contains("no tools are available"));
    }

    #[test]
    fn the_plan_projection_carries_shape_and_drops_tools() {
        let d = decode_derived(GOOD.as_bytes()).unwrap();
        let plan = d.to_plan();
        assert_eq!(plan.version, 1);
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[0].role, "researcher");
        assert_eq!(plan.steps[0].kind, PlanStepKind::Plain);
        assert_eq!(plan.edges.len(), 1);
        assert_eq!((plan.edges[0].parent, plan.edges[0].child), (0, 1));
    }

    #[test]
    fn an_oversize_name_clamps_on_a_char_boundary() {
        let long = "é".repeat(200);
        let src = GOOD.replace("Release Notes Writer", &long);
        let d = decode_derived(src.as_bytes()).expect("a long name clamps, it does not refuse");
        assert!(d.name.len() <= MAX_DERIVE_NAME_BYTES);
        assert!(d.name.chars().all(|c| c == 'é'));
    }
}
