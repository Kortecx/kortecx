//! The curated **system-prompt / role library** — a versioned, testable asset of
//! capability-teaching prompts (the kx-skill-manifest discipline, applied to prompts).
//!
//! Today it houses the **NL→DAG planner contract** ([`PLANNER_SYSTEM`]) and the curated
//! **authoring-role palette** ([`AUTHORING_ROLES`]) the `ProposeWorkflow` path drives (a
//! goal → a proposed multi-agent DAG, propose-then-confirm, D209.3 / SN-8). The role
//! names are aligned to the SDK persona library (`@kortecx/sdk` `PERSONAS`) so the console
//! maps a proposed step's `role` → its persona framing at author time (the identity-bearing
//! role instruction folds into the step prompt on the CLIENT, exactly as a hand-authored
//! persona does — this module supplies the PLANNER contract + the palette, never a
//! per-step identity axis, so it stays off-MoteId / off-digest).
//!
//! These prompts are **presentation only** (SN-8): fed to `render_chat`, never journaled,
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
/// vetted role recipe (SN-8), so the model must NEVER name a model, a tool, or a permission.
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
/// ([`PLANNER_SYSTEM`]) rides the system channel; this is the per-request user turn.
#[must_use]
pub(crate) fn planner_user_message(goal: &str) -> String {
    format!("{}\nGOAL: {}", render_role_palette(), goal.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn planner_contract_names_the_strict_envelope() {
        assert!(PLANNER_SYSTEM.contains("\"plan\""));
        assert!(PLANNER_SYSTEM.contains("\"version\":1"));
        assert!(PLANNER_SYSTEM.contains("never invent"));
    }
}
