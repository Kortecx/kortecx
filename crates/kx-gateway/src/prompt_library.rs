//! The curated **system-prompt / role library** — a versioned, testable asset of
//! capability-teaching prompts (the kx-skill-manifest discipline, applied to prompts).
//!
//! Today it houses the **NL→DAG planner contract** ([`PLANNER_SYSTEM`]) and the curated
//! **authoring-role palette** ([`AUTHORING_ROLES`]) the `ProposeWorkflow` path drives (a
//! goal → a proposed multi-agent DAG, propose-then-confirm, D209.3). The role
//! names are aligned to the SDK persona library (`@kortecx/sdk` `PERSONAS`) so the console
//! maps a proposed step's `role` → its persona framing at author time (the identity-bearing
//! role instruction folds into the step prompt on the CLIENT, exactly as a hand-authored
//! persona does — this module supplies the PLANNER contract + the palette, never a
//! per-step identity axis, so it stays off-MoteId / off-digest).
//!
//! These prompts are **presentation only**: fed to `render_chat`, never journaled,
//! never an identity input. The library is the single source the planner reads; enriching
//! it steers the model without touching `7d22d4bd` or replay.
//!
//! Scope note (don't-fake-gaps, D142): per-ROLE run-time system-prompt selection would
//! need a role axis carried on the Mote (identity-bearing → digest-sensitive), so it is
//! deliberately NOT wired into `dispatch_system_prompt` here; the role framing reaches the
//! model via the client-side persona path, and the planner contract is the library's live
//! run-time consumer.

/// The NL→DAG **planner contract** (the system prompt for the `ProposeWorkflow` model
/// turn). It teaches the model to decompose a goal into a small multi-step DAG and emit
/// EXACTLY one strict `{"plan":{…}}` envelope — the minimal trust surface `kx-planner`
/// decodes: a step names ONLY a `role` (from the provided palette) + a free-form `intent`;
/// edges are `{parent,child}` indices. The runtime supplies every capability axis from the
/// vetted role recipe, so the model must NEVER name a model, a tool, or a permission.
/// The exact envelope shape round-trips through [`kx_planner::decode_plan`] (pinned by the
/// `planner_example_decodes_and_uses_palette_roles` test).
pub(crate) const PLANNER_SYSTEM: &str = "You are a precise workflow planner. Turn the user's \
GOAL into the SMALLEST multi-step plan of collaborating agent roles that fully achieves it. \
Decompose the goal into a short ordered pipeline (or a fan-out that gathers into a final \
step): typically 2 to 5 steps. Choose each step's ROLE from the provided role palette — use \
ONLY those role names, never invent one. Write each step's INTENT as one concrete, \
self-contained instruction for that role (what it must produce), phrased so the role can act \
without seeing the others. Order the steps so each depends on the ones before it, and \
connect them with edges (an edge parent to child feeds the parent's output into the child). \
Reply with EXACTLY one JSON object and NOTHING else — no prose, no code fence, no \
explanation:\n\
{\"plan\":{\"version\":1,\"steps\":[{\"role\":\"<palette role>\",\"intent\":\"<what this step \
produces>\"}],\"edges\":[{\"parent\":<step index>,\"child\":<step index>}]}}\n\
Rules: version is always 1; step and edge indices are 0-based into steps[]; an edge's parent \
index must be smaller than its child index; do NOT include any model id, tool, permission, or \
any field other than role/intent per step and parent/child per edge. Keep the plan minimal — \
add a step only when it does distinct work.";

/// The APP-DERIVE contract — the system prompt for the `DeriveApp` model turn.
///
/// A **sibling** of [`PLANNER_SYSTEM`], not a variant of it. Two things differ, and both are
/// the reason a variant would not do:
///
/// 1. **Shape is a judgement, not a default.** `PLANNER_SYSTEM` says "Order the steps so each
///    depends on the ones before it", which is an instruction to emit a CHAIN — and every plan
///    it has ever produced was one. This contract instead teaches that a step with no incoming
///    edge runs CONCURRENTLY, so independent work should fan out and rejoin. The `parent <
///    child` rule stays: it is a topological numbering, not a linearity rule, and a fan-out
///    (`0→1, 0→2, 1→3, 2→3`) satisfies it.
/// 2. **The model may NAME a capability.** Steps carry a `tools` list drawn from a menu the
///    SERVER computed from the caller's own ceiling. Naming is not granting: every id is
///    intersected back against that ceiling host-side, so this widens what can be ASKED FOR
///    and nothing else. The role palette still supplies every other capability axis.
pub(crate) const DERIVE_SYSTEM: &str = "You are designing a Kortecx APP: a durable, reusable \
automation that a schedule, a trigger, or another workflow runs. Turn the user's request into \
the SMALLEST workflow of collaborating agent roles that fully achieves it.\n\
SHAPE THE WORKFLOW FOR THE WORK — this is your judgement, not a template:\n\
- Steps that do NOT need each other's output run AT THE SAME TIME. Give them no edge between \
them, and they run in PARALLEL.\n\
- When several parallel steps produce material that must be combined, add ONE final step that \
gathers them, with an edge from each into it.\n\
- Chain two steps ONLY when the second genuinely needs what the first produced.\n\
- A goal that is one piece of work is ONE step. Do not pad a plan to look thorough.\n\
Choose each step's ROLE from the provided role palette — use ONLY those role names, never \
invent one. Write each step's INTENT as one concrete, self-contained instruction for that role \
(what it must produce), phrased so the role can act without seeing the others.\n\
EVERY CAPABILITY BELONGS TO THE STEP THAT USES IT — never to the app. Each step names its \
own TOOLS, SKILLS, INTEGRATIONS, DATASETS and APPS, picking from the provided capability menu \
and ONLY from it, by the exact name the menu shows:\n\
- tools: give a step a tool only when that step genuinely has to reach outside the model to \
do its job.\n\
- skills: attach one when a step needs that specific expertise to do its part.\n\
- integrations: attach one only to the step that actually talks to that service.\n\
- datasets: attach one to the step whose answer must be grounded in it.\n\
- apps: when the menu already lists an app that DELIVERS what a step needs, name that app on \
the step instead of designing the work again. The app runs and the step receives its result.\n\
Most steps need few or none — use an empty list. Never invent a name, and never name a \
permission, a credential, or a model. A step that merely combines what other steps produced \
usually needs nothing at all.\n\
Also give the app a short NAME (2-5 words, what it does) and a one-sentence DESCRIPTION, plus \
DELIVERS: one short phrase naming what a run of it produces (e.g. \"a weekly digest of \
pipeline and support activity\"), so other apps can find it later.\n\
Reply with EXACTLY one JSON object and NOTHING else — no prose, no code fence, no \
explanation. This example gathers two INDEPENDENT things and combines them — note that steps 0 \
and 1 have no edge into them, so they run AT THE SAME TIME, step 2 waits for both, each step \
carries ONLY what it itself needs, and step 1 calls an existing app rather than re-designing \
the work it already does:\n\
{\"app\":{\"name\":\"Weekly Sales Digest\",\"description\":\"Combines pipeline and support \
signals into one weekly digest.\",\"delivers\":\"a weekly digest of pipeline and support \
activity\",\"steps\":[\
{\"role\":\"researcher\",\"intent\":\"Collect this week's closed deals and their values\",\
\"tools\":[\"crm/query\"],\"skills\":[],\"integrations\":[\"crm\"],\"datasets\":[],\"apps\":[]},\
{\"role\":\"researcher\",\"intent\":\"Collect this week's support escalations\",\
\"tools\":[],\"skills\":[\"triage\"],\"integrations\":[],\"datasets\":[\"support\"],\
\"apps\":[\"apps/local/escalation-review\"]},\
{\"role\":\"writer\",\"intent\":\"Write one digest covering both\",\
\"tools\":[],\"skills\":[],\"integrations\":[],\"datasets\":[],\"apps\":[]}],\
\"edges\":[{\"parent\":0,\"child\":2},{\"parent\":1,\"child\":2}]}}\n\
Rules: step and edge indices are 0-based into steps[]; an edge's parent index must be smaller \
than its child index; every list is always present on every step, empty when unused; an app \
handle is written EXACTLY as the menu shows it and nothing else (no description, no \
parentheses); do NOT include a model id, a permission, or any field other than the ones shown. \
Do NOT connect every step in one line — an edge means the child CONSUMES the parent's output, \
so two steps that read different sources must not be chained.";

/// One curated authoring role: a stable `name` (aligned to the SDK persona library) + a
/// one-line `framing` the planner palette shows the model. The heavy MoteDef axes come
/// from the vetted recipe (`build_authoring_role_catalog`), never from this table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AuthoringRole {
    /// The role name (also a `@kortecx/sdk` persona name — the console maps role→persona).
    pub(crate) name: &'static str,
    /// A one-line capability framing for the planner's palette (off-identity, display).
    pub(crate) framing: &'static str,
}

/// The curated authoring-role palette the NL planner may compose. Names are byte-aligned to
/// the SDK `PERSONAS` so a proposed step's role maps to its persona framing on the client.
/// Every role lowers (via the vetted recipe) to a PURE model step — per-step tools / skills
/// / connections + policy are attached afterward in the builder (the C3 authoring surface).
pub(crate) const AUTHORING_ROLES: &[AuthoringRole] = &[
    AuthoringRole {
        name: "researcher",
        framing: "gathers facts + concrete evidence, separates known from inferred, flags gaps",
    },
    AuthoringRole {
        name: "analyst",
        framing: "breaks the problem down, reasons step by step, quantifies, states assumptions",
    },
    AuthoringRole {
        name: "critic",
        framing: "finds flaws, unstated assumptions, edge cases, and failure modes in the work",
    },
    AuthoringRole {
        name: "skeptic",
        framing:
            "challenges each claim: asks for evidence, what would falsify it, where it's wrong",
    },
    AuthoringRole {
        name: "planner",
        framing: "turns a goal into an ordered, concrete plan with dependencies and risks",
    },
    AuthoringRole {
        name: "strategist",
        framing: "weighs options + second-order effects, recommends one course with the reasoning",
    },
    AuthoringRole {
        name: "engineer",
        framing: "produces correct, minimal, maintainable solutions and handles the failure paths",
    },
    AuthoringRole {
        name: "writer",
        framing: "turns material into clear, well-structured prose that leads with the point",
    },
    AuthoringRole {
        name: "editor",
        framing: "tightens for clarity, accuracy, and flow without changing the meaning",
    },
    AuthoringRole {
        name: "summarizer",
        framing: "distills to the essential points in the fewest words that preserve meaning",
    },
];

/// Render the role palette block appended to the planner's user message: one
/// `- name: framing` line per curated role. Deterministic (palette order).
#[must_use]
pub(crate) fn render_role_palette() -> String {
    let mut out = String::from("Role palette (use only these role names):\n");
    for r in AUTHORING_ROLES {
        out.push_str("- ");
        out.push_str(r.name);
        out.push_str(": ");
        out.push_str(r.framing);
        out.push('\n');
    }
    out
}

/// Build the planner's USER message for a goal: the palette + the goal. The planner CONTRACT
/// The NL-CONTROL contract — the system prompt for the `ProposeControlAction` turn.
///
/// A **third sibling** of [`PLANNER_SYSTEM`] and [`DERIVE_SYSTEM`], and deliberately not a
/// variant of either. Those two ask the model to design a computation; this one asks it to
/// fill in ONE registration form. The model picks a domain and emits that domain's fields —
/// it never designs steps, never names a model, and never states an authority.
///
/// **What the model is structurally prevented from saying.** There is no field here for a
/// secret VALUE, for script `argv`, or for script `env`. That is not a rule the model is
/// asked to follow — the decoder rejects unknown keys (`deny_unknown_fields`), and the wire
/// type the proposal lowers into has no such field either. A model that tried would be
/// refused twice before anything reached a human.
///
/// The example below is pinned by `control_example_decodes_through_the_real_decoder`, which
/// decodes it with the SAME function the runtime uses and asserts `CONTROL_SYSTEM` contains
/// it verbatim. A contract whose example does not decode teaches a shape nothing accepts.
pub(crate) const CONTROL_SYSTEM: &str = "You turn one operator request into ONE registration form. You do not design workflows and you do not run anything: you fill in the fields for exactly one DOMAIN, and a human reviews the result before it is issued.\nPick the single best domain for the request:\n  workflows  — save a named multi-step definition\n  tools      — register an external tool the runtime may call\n  connectors — register an MCP server\n  secrets    — declare that a named credential should exist (NAME only)\n  scripts    — register a script the runtime may run sandboxed\n  triggers   — schedule or expose an existing app/recipe/workflow\n  policy     — define or assign a named role that NARROWS tool authority\nReply with EXACTLY one JSON object and NOTHING else — no prose, no code fence:\n{\"control\":{\"domain\":\"triggers\",\"name\":\"nightly-report\",\"fields\":{\"kind\":\"cron\",\"schedule_spec\":\"0 9 * * *\",\"app_handle\":\"ops/reports/daily\"}}}\nEach domain accepts ONLY these keys in `fields`; there are no others:\n  workflows  envelope_json, lifecycle\n  tools      tool_version, description, idempotency_class, server_host, remote_name\n  connectors transport, endpoint, args, tls_required, credential_ref, session_mode\n  secrets    secret_scope, net_scope\n  scripts    script_version, description, interpreter, source, net_hosts, wall_clock_ms\n  triggers   kind, auth, auth_secret_ref, schedule_spec, timezone, enabled, require_approval, recipe_handle, app_handle, workflow_handle\n  policy     description, tools, party      (tools is a list of \"tool_id@version\" strings; giving `party` ASSIGNS the named role to that party instead of defining it)\nRules: `domain` is one of the seven names above, exactly as spelled. `name` is the operator handle for the thing being registered. Use ONLY the keys listed for your chosen domain — an unrecognised key is REFUSED outright, not ignored, so omit anything you are unsure of rather than inventing a name for it. NEVER include a secret value, a password, a token, script arguments, or environment variables: those are set by an operator, not by you, and a form containing one is discarded. NEVER state a model, a permission, or an authority — a role NARROWS what a party may already do and can never grant anything new.";

/// Build the NL-CONTROL user message. The contract ([`CONTROL_SYSTEM`]) rides the system
/// channel; this is the per-request user turn.
///
/// `steer` is the caller's optional domain hint. It is passed as a HINT and not as an
/// instruction to obey blindly: a caller who steers wrong should get a refusal from the
/// admissibility gate, not a confidently-wrong form in the domain they named.
#[must_use]
pub(crate) fn control_user_message(goal: &str, steer: &str) -> String {
    let steer = steer.trim();
    if steer.is_empty() {
        format!("REQUEST: {}", goal.trim())
    } else {
        format!(
            "The operator suggests the {steer} domain.\nREQUEST: {}",
            goal.trim()
        )
    }
}

/// ([`PLANNER_SYSTEM`]) rides the system channel; this is the per-request user turn.
#[must_use]
pub(crate) fn planner_user_message(goal: &str) -> String {
    format!("{}\nGOAL: {}", render_role_palette(), goal.trim())
}

/// Build the DERIVE user message: the role palette, the ids-only capability menu, and the
/// user's one prompt. The contract ([`DERIVE_SYSTEM`]) rides the system channel.
///
/// `menu` is pre-rendered and pre-BOUNDED by the caller (see `derive_plan::CapabilityMenu`)
/// because the whole exchange is ONE decode — the serve does not chunk a prompt, so an
/// unbounded menu does not degrade, it aborts.
#[must_use]
pub(crate) fn derive_user_message(prompt: &str, menu: &str) -> String {
    format!(
        "{}\n{menu}\nWhat the user asked for: {}",
        render_role_palette(),
        prompt.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The example `CONTROL_SYSTEM` teaches, byte-identical to the one inside it.
    ///
    /// Keep it in sync with the contract; the assertion below is what makes "in sync"
    /// checkable rather than a hope.
    const CONTROL_EXAMPLE: &str = r#"{"control":{"domain":"triggers","name":"nightly-report","fields":{"kind":"cron","schedule_spec":"0 9 * * *","app_handle":"ops/reports/daily"}}}"#;

    /// The taught example decodes through the REAL decoder, and the contract contains it.
    ///
    /// Three assertions, and each catches a different way this drifts:
    /// 1. the example decodes — a contract teaching a shape nothing accepts is worse than
    ///    no example, because the model will reproduce it faithfully and be refused;
    /// 2. `CONTROL_SYSTEM` contains it VERBATIM — an example edited in one place and not
    ///    the other teaches the old shape while the test passes on the new one;
    /// 3. it decodes to the domain it claims — a syntactically valid form that lands in
    ///    the wrong domain would satisfy (1) and (2) and still be wrong.
    #[test]
    fn control_example_decodes_through_the_real_decoder() {
        let decoded = crate::control_decode::decode_control(CONTROL_EXAMPLE.as_bytes())
            .expect("the taught example must decode via the same enforcer the runtime uses");
        assert!(
            matches!(
                decoded,
                kx_gateway_core::ControlProposal::RegisterTrigger(_)
            ),
            "the example claims the triggers domain and must decode into it"
        );
        assert!(
            CONTROL_SYSTEM.contains(CONTROL_EXAMPLE),
            "CONTROL_SYSTEM must teach this exact example — it has drifted from the test"
        );
    }

    /// Every domain the contract NAMES is a domain the decoder ACCEPTS.
    ///
    /// The list in the prompt and the match in the decoder are two enumerations of one
    /// fact. Without this, adding a domain to the prose while forgetting the decoder
    /// produces a model that confidently emits forms nothing can read.
    #[test]
    fn every_domain_the_contract_offers_is_one_the_decoder_knows() {
        for domain in [
            "workflows",
            "tools",
            "connectors",
            "secrets",
            "scripts",
            "triggers",
            "policy",
        ] {
            assert!(
                CONTROL_SYSTEM.contains(domain),
                "{domain} is decodable but the contract never offers it"
            );
            let form = format!(r#"{{"control":{{"domain":"{domain}","name":"x","fields":{{}}}}}}"#);
            assert!(
                crate::control_decode::decode_control(form.as_bytes()).is_ok(),
                "the contract offers {domain} but the decoder refuses it"
            );
        }
    }

    /// The control contract is its OWN contract, not a copy of the other two.
    ///
    /// The anti-unification assertion, extended to three: someone "unifying" these later
    /// would make one prompt teach another's shape, and the model would emit a plan where
    /// a registration form was asked for.
    #[test]
    fn the_three_contracts_stay_distinct() {
        let contracts = [
            ("PLANNER", PLANNER_SYSTEM),
            ("DERIVE", DERIVE_SYSTEM),
            ("CONTROL", CONTROL_SYSTEM),
        ];
        for (a_name, a) in contracts {
            for (b_name, b) in contracts {
                if a_name == b_name {
                    continue;
                }
                assert!(
                    !a.contains(b),
                    "{a_name}_SYSTEM contains {b_name}_SYSTEM — the contracts have been unified"
                );
            }
        }
        // And the control contract does NOT teach the planner's step-ordering rule, which
        // is the specific sentence a careless merge would carry across.
        assert!(!CONTROL_SYSTEM.contains("Order the steps so each depends on the ones before it"));
        // Nor does it invite a design: it asks for a form.
        assert!(CONTROL_SYSTEM.contains("registration form"));
    }

    /// The user turn carries the steer as a HINT, and omits it cleanly when absent.
    #[test]
    fn the_control_user_message_carries_the_request_and_any_steer() {
        let bare = control_user_message("  make a nightly report  ", "");
        assert!(bare.contains("REQUEST: make a nightly report"));
        assert!(!bare.contains("suggests"), "no steer, no steer sentence");

        let steered = control_user_message("make a nightly report", "triggers");
        assert!(steered.contains("suggests the triggers domain"));
        assert!(steered.contains("REQUEST: make a nightly report"));
    }

    /// The canonical example the planner contract teaches — its shape MUST decode through
    /// the same enforcer the runtime uses (`kx_planner::decode_plan`), and every role in it
    /// must be a real palette role. Keep it byte-in-sync with the envelope in `PLANNER_SYSTEM`.
    const PLANNER_EXAMPLE: &str = "{\"plan\":{\"version\":1,\"steps\":[\
{\"role\":\"researcher\",\"intent\":\"Gather the key facts and sources on the topic\"},\
{\"role\":\"analyst\",\"intent\":\"Compare the options and weigh the trade-offs\"},\
{\"role\":\"writer\",\"intent\":\"Write a clear, well-structured comparison\"}],\
\"edges\":[{\"parent\":0,\"child\":1},{\"parent\":1,\"child\":2}]}}";

    fn is_palette_role(name: &str) -> bool {
        AUTHORING_ROLES.iter().any(|r| r.name == name)
    }

    // Render↔enforce coherence (mirrors `react_system_envelope_round_trips_through_parse_tool_call`):
    // the shape the planner contract teaches decodes through the enforcer, and every role in
    // the taught example is a real palette role.
    #[test]
    fn planner_example_decodes_and_uses_palette_roles() {
        let plan = kx_planner::decode_plan(PLANNER_EXAMPLE.as_bytes(), 8192)
            .expect("the taught example must decode via the same enforcer the runtime uses");
        assert!(
            plan.steps.len() >= 2,
            "the example teaches a MULTI-step plan"
        );
        for s in &plan.steps {
            assert!(
                is_palette_role(&s.role),
                "example role {:?} is not in the curated palette",
                s.role
            );
        }
    }

    #[test]
    fn role_palette_is_nonempty_unique_and_deterministic() {
        assert!(!AUTHORING_ROLES.is_empty());
        assert_eq!(render_role_palette(), render_role_palette());
        for (i, a) in AUTHORING_ROLES.iter().enumerate() {
            for b in &AUTHORING_ROLES[i + 1..] {
                assert_ne!(a.name, b.name, "duplicate role name {:?}", a.name);
            }
        }
    }

    /// The DERIVE contract's own taught envelope must decode through the enforcer the runtime
    /// uses, and name only palette roles — the `planner_example_decodes_and_uses_palette_roles`
    /// discipline applied to the second contract. A contract that teaches a shape its own
    /// decoder refuses produces a model that is right and a runtime that says it is wrong.
    #[test]
    fn derive_example_decodes_and_uses_palette_roles() {
        const DERIVE_EXAMPLE: &str = "{\"app\":{\"name\":\"Weekly Sales Digest\",\
\"description\":\"Combines pipeline and support signals into one weekly digest.\",\
\"delivers\":\"a weekly digest of pipeline and support activity\",\"steps\":[\
{\"role\":\"researcher\",\"intent\":\"Collect this week's closed deals and their values\",\
\"tools\":[\"crm/query\"],\"skills\":[],\"integrations\":[\"crm\"],\"datasets\":[],\"apps\":[]},\
{\"role\":\"researcher\",\"intent\":\"Collect this week's support escalations\",\
\"tools\":[],\"skills\":[\"triage\"],\"integrations\":[],\"datasets\":[\"support\"],\
\"apps\":[\"apps/local/escalation-review\"]},\
{\"role\":\"writer\",\"intent\":\"Write one digest covering both\",\
\"tools\":[],\"skills\":[],\"integrations\":[],\"datasets\":[],\"apps\":[]}],\
\"edges\":[{\"parent\":0,\"child\":2},{\"parent\":1,\"child\":2}]}}";
        let d = crate::derive_plan::decode_derived(DERIVE_EXAMPLE.as_bytes())
            .expect("the taught envelope must decode via the same enforcer the runtime uses");
        assert_eq!(d.steps.len(), 3);
        for s in &d.steps {
            assert!(
                is_palette_role(&s.role),
                "example role {:?} is not in the curated palette",
                s.role
            );
        }
        // ★ The taught example must itself be a FAN-OUT, and the contract must contain it
        // verbatim. Found live: with a single-edge example, Gemma-4-12B returned a 4-step CHAIN
        // for a prompt naming two explicitly independent sources — prose about parallelism did
        // not survive contact with an example that showed a chain. Whoever "simplifies" this
        // example back to one edge should fail here.
        assert!(
            DERIVE_SYSTEM
                .contains("\"edges\":[{\"parent\":0,\"child\":2},{\"parent\":1,\"child\":2}]"),
            "the contract must TEACH a fan-out by example, not only describe one"
        );
        let with_parent: std::collections::BTreeSet<usize> =
            d.edges.iter().map(|&(_, c)| c).collect();
        assert!(
            !with_parent.contains(&0) && !with_parent.contains(&1),
            "steps 0 and 1 must have no parent — that IS the parallelism being taught"
        );
        assert!(with_parent.contains(&2), "step 2 must join them");

        // ★ THE EXAMPLE IS THE CONTRACT, part two. The capability lists must sit ON THE
        // STEPS and at least one must be NON-EMPTY. The previous contract described the
        // axes in prose and then showed `"skills":[],"integrations":[],"datasets":[]` at
        // the app level — three empty lists, which is what a model copies. Prose lost to
        // the example once already (a single-edge example produced chains for four bullets
        // of parallelism instruction); this is the same failure waiting in a different
        // field, so pin the SHAPE and not just the decode.
        assert!(
            DERIVE_SYSTEM.contains(DERIVE_EXAMPLE),
            "the contract must teach the per-step shape VERBATIM"
        );
        assert!(
            d.steps.iter().all(|s| {
                DERIVE_EXAMPLE.contains(&format!("\"intent\":\"{}\",\"tools\":", s.intent))
            }),
            "every taught step carries its own capability lists"
        );
        assert!(
            d.steps
                .iter()
                .any(|s| !s.skills.is_empty() || !s.integrations.is_empty()),
            "at least one taught step must ATTACH something — an example of three empty \
             lists teaches three empty lists"
        );
        assert!(
            d.steps.iter().any(|s| !s.datasets.is_empty()),
            "grounding must be shown on a step too, not only described"
        );
        assert!(
            !d.folded_app_level,
            "the taught example must not use the legacy app-level shape"
        );
        // The step that merely joins the other two asks for nothing — the discrimination
        // the whole per-node model exists to express.
        assert!(
            d.steps[2].skills.is_empty()
                && d.steps[2].integrations.is_empty()
                && d.steps[2].datasets.is_empty()
                && d.steps[2].tools.is_empty()
                && d.steps[2].apps.is_empty(),
            "the joining step must be taught as needing nothing"
        );
        // ★ The composition axis, taught by example for the same reason as the others: an
        // app that is only DESCRIBED as callable is an app the model re-designs from scratch
        // every time. Exactly one step calls one — enough to teach the shape, not so much
        // that "call an app" reads as the default answer to every step.
        assert_eq!(
            d.steps.iter().filter(|s| !s.apps.is_empty()).count(),
            1,
            "exactly one taught step must CALL an app"
        );
        assert_eq!(d.steps[1].apps, vec!["apps/local/escalation-review"]);
        // And the app must propose its own `delivers`, or nothing it authors is ever
        // discoverable by the next app's author.
        assert!(
            !d.delivers.is_empty(),
            "the taught example must carry a delivers line"
        );
    }

    /// The menu renders a handle WITH what the app delivers, and the decoder must recover the
    /// bare handle from whatever the model echoes back.
    ///
    /// This is the `- retrieve (v1)` failure, pre-empted. That one cost a real grant: the menu
    /// showed a decoration, the model returned it as the id, and the notice blamed the
    /// account's authority. The composition menu cannot drop its decoration — a bare handle
    /// tells a model nothing about which app to pick — so the decoder absorbs it instead.
    #[test]
    fn a_menu_decorated_app_handle_still_decodes_to_the_bare_handle() {
        let echoed = "{\"app\":{\"name\":\"X\",\"description\":\"d\",\"steps\":[\
{\"role\":\"writer\",\"intent\":\"go\",\"apps\":\
[\"apps/local/research — a researched brief\"]}]}}";
        let d = crate::derive_plan::decode_derived(echoed.as_bytes()).expect("decodes");
        assert_eq!(
            d.steps[0].apps,
            vec!["apps/local/research"],
            "the menu's display form must not survive as the handle"
        );
    }

    /// The derive contract must teach PARALLELISM, and must not inherit the planner's
    /// "each depends on the ones before it" — that single sentence is why every proposal the
    /// console has ever shown was a chain. This is the assertion that fails if someone
    /// "unifies" the two contracts later.
    #[test]
    fn the_derive_contract_teaches_shape_and_is_not_the_planner_one() {
        assert!(DERIVE_SYSTEM.contains("PARALLEL"));
        assert!(DERIVE_SYSTEM.contains("AT THE SAME TIME"));
        assert!(
            !DERIVE_SYSTEM.contains("Order the steps so each depends on the ones before it"),
            "the derive contract must not inherit the planner's chain instruction"
        );
        assert!(!DERIVE_SYSTEM.contains(PLANNER_SYSTEM));
        assert!(!PLANNER_SYSTEM.contains(DERIVE_SYSTEM));
        // The menu is the ONLY tool source, and the model still may not name authority.
        assert!(DERIVE_SYSTEM.contains("ONLY from it"));
        assert!(DERIVE_SYSTEM.contains("never name a permission"));
    }

    /// The user turn must carry the palette, the menu and the prompt — all three. A derive
    /// message missing the menu silently produces an app with no capabilities and no reason.
    #[test]
    fn the_derive_user_message_carries_palette_menu_and_prompt() {
        let m = derive_user_message(
            "  triage inbound email  ",
            "Capability menu:\n- echo (v1)\n",
        );
        assert!(m.contains("Role palette"));
        assert!(m.contains("- echo (v1)"));
        assert!(m.contains("triage inbound email"));
        assert!(!m.contains("  triage"), "the prompt is trimmed");
    }

    #[test]
    fn planner_contract_names_the_strict_envelope() {
        assert!(PLANNER_SYSTEM.contains("\"plan\""));
        assert!(PLANNER_SYSTEM.contains("\"version\":1"));
        assert!(PLANNER_SYSTEM.contains("never invent"));
    }
}
