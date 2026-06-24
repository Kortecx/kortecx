//! POC-5a — the App-scaffold seam + the fixed skeleton + pure helpers.
//!
//! `ScaffoldApp` drives a NET-NEW agentic loop that writes a FIXED skeleton of
//! agentic-app files into a fresh CoW-on-CAS branch: for each skeleton file, a
//! single Pure greedy model step (the proven `react-edit` pattern) authors the file
//! body, which is content-addressed and `AdvanceBranch`-ed into the manifest. The
//! STRUCTURE is fixed (deterministic + testable); the CONTENT is model-authored.
//!
//! The ORCHESTRATION (spawn a background task, await each step, advance the branch)
//! lives in the HOST behind the [`AppScaffolder`] seam — gateway-core is a read-fold
//! and propose-proxy that deliberately owns no runtime/spawn surface (the AppCatalog
//! and BranchStore precedent). gateway-core provides the seam, the fixed skeleton, the
//! pure prompt/body helpers, and a NO-tokio single-shot commit check
//! ([`try_committed_body`]) that the host's await loop polls.

use kx_content::ContentRef;
use kx_mote::MoteId;
use kx_projection::MoteState;

use crate::error::{internal, GatewayError};
use crate::reader::{ContentReader, JournalReader};

/// The recipe handle the host seeds for the scaffold write step (a single Pure
/// greedy model step, the `react-edit` clone). The host's `provision.rs` MUST seed
/// this exact handle (the scaffold contract).
pub const APP_SCAFFOLD_WRITE_RECIPE_HANDLE: &str = "kx/recipes/app-scaffold-write";

/// One fixed skeleton file: a stable path + the authoring role the model fills.
pub struct ScaffoldFile {
    /// The manifest path (stable — the deterministic e2e asserts exactly these).
    pub path: &'static str,
    /// A short role description woven into the authoring prompt.
    pub role: &'static str,
}

/// The FIXED skeleton of a scaffolded agentic-app project. STRUCTURE is fixed (the
/// e2e asserts exactly these paths); the model authors each file's CONTENT. Ordered
/// so earlier files (README, app.json) become coherence context for later ones.
pub const SKELETON: &[ScaffoldFile] = &[
    ScaffoldFile {
        path: "README.md",
        role: "a concise README: what the app does, how to run it, and its inputs/outputs",
    },
    ScaffoldFile {
        path: "app.json",
        role: "a JSON object describing the app: name, a one-line description, the model \
               route intent, and the high-level steps (a plain JSON object, no comments)",
    },
    ScaffoldFile {
        path: "prompts/system.md",
        role: "the system prompt that steers the agent's behaviour for this app",
    },
    ScaffoldFile {
        path: "rules/guardrails.md",
        role: "the guardrails / rules the agent must always follow (safety, scope, refusals)",
    },
    ScaffoldFile {
        path: "skills/main.md",
        role: "the primary skill: a focused instruction + the tools it may use, in markdown",
    },
];

/// The scaffold phase reported by `GetScaffoldStatus`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScaffoldPhase {
    /// The branch is being prepared / the first file is in flight.
    Planning,
    /// Skeleton files are being authored (files_pending non-empty).
    Writing,
    /// Every skeleton file has been advanced into the branch.
    Done,
    /// The orchestration failed (the `detail` carries the advisory reason).
    Failed,
}

/// The resolved scaffold status (the host derives `files_*` from the durable branch
/// manifest and `phase`/`detail` from its advisory tracker, falling back to the
/// manifest when untracked — e.g. after a restart).
#[derive(Clone, Debug)]
pub struct ScaffoldStatus {
    /// The current phase.
    pub phase: ScaffoldPhase,
    /// Skeleton paths already present in the branch manifest.
    pub files_done: Vec<String>,
    /// Skeleton paths still to write.
    pub files_pending: Vec<String>,
    /// Advisory progress / failure text (never authority).
    pub detail: String,
}

/// The host-side App-scaffold orchestrator seam. The host impl owns the runtime
/// (spawns the background task, runs the await loop, holds the advisory tracker +
/// the binder/submitter/branches/locks seams). A `None` seam ⇒ `ScaffoldApp` /
/// `GetScaffoldStatus` return `unimplemented` (no served model / branch store).
pub trait AppScaffolder: Send + Sync {
    /// Start (or resume) the background scaffold of `branch_handle` toward `goal`.
    /// Returns `resumed` (true iff the branch already held ≥1 skeleton file). Spawns
    /// a background task and returns immediately (the propose-proxy contract).
    fn start(&self, principal: &str, branch_handle: &str, goal: &str)
        -> Result<bool, GatewayError>;

    /// The current scaffold status for a branch (caller-scoped via the host's
    /// branch store).
    fn status(&self, principal: &str, branch_handle: &str) -> Result<ScaffoldStatus, GatewayError>;
}

/// The outcome of one single-shot terminal-commit check (the host's await loop
/// polls this with its own sleep — gateway-core owns no timer).
pub enum ScaffoldStep {
    /// Not yet committed (the host keeps polling until its deadline).
    Pending,
    /// Committed — the body + its content ref (advance the branch to this ref).
    Ready {
        /// The committed body's 32-byte content ref.
        result_ref: [u8; 32],
        /// The committed body bytes (the host checks emptiness, GR15).
        body: Vec<u8>,
    },
    /// The step reached a non-committed terminal (Failed/Repudiated/Inconsistent).
    Failed,
}

/// Single-shot, NO-tokio check of a write step's terminal mote: fold the read-only
/// projection once and resolve the committed body (mirrors the `GetContent` fold).
/// The host's await loop calls this between sleeps.
pub fn try_committed_body(
    reader: &dyn JournalReader,
    content: &dyn ContentReader,
    terminal: MoteId,
) -> Result<ScaffoldStep, GatewayError> {
    let head = reader.current_seq().map_err(internal)?;
    let (projection, _) = crate::view::fold_through(reader, head)?;
    // Bind the state into an owned value (both fields are `Copy`) so the iterator's
    // borrow of `projection` ends before we re-borrow it for `result_ref_of`.
    let state = projection
        .iter_motes()
        .find(|(id, _)| *id == terminal)
        .map(|(_, s)| s);
    match state {
        Some(MoteState::Committed) => {
            let result_ref = projection
                .result_ref_of(&terminal)
                .ok_or_else(|| internal("committed scaffold step has no result ref"))?;
            let body = content
                .get(&ContentRef::from_bytes(result_ref.0))
                .ok_or_else(|| internal("committed scaffold body missing from store"))?;
            Ok(ScaffoldStep::Ready {
                result_ref: result_ref.0,
                body,
            })
        }
        Some(MoteState::Failed | MoteState::Repudiated | MoteState::Inconsistent) => {
            Ok(ScaffoldStep::Failed)
        }
        // None (not yet folded) or Pending/Scheduled ⇒ keep polling.
        _ => Ok(ScaffoldStep::Pending),
    }
}

/// Build the authoring directive for one skeleton file. GR15: the committed answer
/// IS the file body verbatim (reasoning is stripped by the recipe), so the directive
/// asks for ONLY the body — no commentary, no code fences.
#[must_use]
pub fn authoring_prompt(file: &ScaffoldFile, goal: &str, has_siblings: bool) -> String {
    let siblings = if has_siblings {
        "The attached context shows sibling files already written for this app; keep this file \
         consistent with them. "
    } else {
        ""
    };
    format!(
        "You are scaffolding files for a durable, governed agentic application.\n\
         App goal: {goal}\n\n\
         Write the COMPLETE contents of the file `{path}` — {role}. {siblings}\
         Return ONLY the file body — no commentary, no explanation, and no markdown code fences.",
        path = file.path,
        role = file.role,
    )
}

/// `true` iff `body` is empty or whitespace-only (the GR15 fail-closed guard — a
/// stripped reasoning block that produced no body must never advance the manifest).
#[must_use]
pub fn body_is_empty(body: &[u8]) -> bool {
    body.iter().all(u8::is_ascii_whitespace)
}

/// Resolve `files_done` / `files_pending` over the FIXED skeleton given the branch
/// manifest's current path set (pure — the host calls this from `status`).
#[must_use]
pub fn split_done_pending(
    manifest_paths: &std::collections::BTreeSet<String>,
) -> (Vec<String>, Vec<String>) {
    let mut done = Vec::new();
    let mut pending = Vec::new();
    for f in SKELETON {
        if manifest_paths.contains(f.path) {
            done.push(f.path.to_string());
        } else {
            pending.push(f.path.to_string());
        }
    }
    (done, pending)
}

/// Derive a phase honestly from the skeleton coverage when no tracker entry exists
/// (e.g. after a restart): Done iff complete, Planning iff empty, else Writing.
#[must_use]
pub fn derive_phase(files_done: &[String], files_pending: &[String]) -> ScaffoldPhase {
    if files_pending.is_empty() && !files_done.is_empty() {
        ScaffoldPhase::Done
    } else if files_done.is_empty() {
        ScaffoldPhase::Planning
    } else {
        ScaffoldPhase::Writing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skeleton_paths_are_stable() {
        // The deterministic e2e pins exactly these paths; guard the contract here.
        let paths: Vec<&str> = SKELETON.iter().map(|f| f.path).collect();
        assert_eq!(
            paths,
            vec![
                "README.md",
                "app.json",
                "prompts/system.md",
                "rules/guardrails.md",
                "skills/main.md",
            ]
        );
    }

    #[test]
    fn authoring_prompt_demands_body_only_and_includes_goal() {
        let p = authoring_prompt(&SKELETON[0], "summarize PDFs", false);
        assert!(p.contains("summarize PDFs"));
        assert!(p.contains("README.md"));
        assert!(p.contains("no markdown code fences"));
        assert!(!p.contains("sibling files")); // no siblings on the first file
        let p2 = authoring_prompt(&SKELETON[2], "summarize PDFs", true);
        assert!(p2.contains("sibling files")); // siblings included once prior files exist
    }

    #[test]
    fn body_is_empty_detects_whitespace_only() {
        assert!(body_is_empty(b""));
        assert!(body_is_empty(b"   \n\t "));
        assert!(!body_is_empty(b"x"));
        assert!(!body_is_empty(b"  hi  "));
    }

    #[test]
    fn split_and_derive_phase_cover_the_states() {
        let none = std::collections::BTreeSet::<String>::new();
        let (d, p) = split_done_pending(&none);
        assert!(d.is_empty());
        assert_eq!(p.len(), SKELETON.len());
        assert_eq!(derive_phase(&d, &p), ScaffoldPhase::Planning);

        let all: std::collections::BTreeSet<String> =
            SKELETON.iter().map(|f| f.path.to_string()).collect();
        let (d, p) = split_done_pending(&all);
        assert_eq!(d.len(), SKELETON.len());
        assert!(p.is_empty());
        assert_eq!(derive_phase(&d, &p), ScaffoldPhase::Done);

        let some: std::collections::BTreeSet<String> = [SKELETON[0].path.to_string()].into();
        let (d, p) = split_done_pending(&some);
        assert_eq!(derive_phase(&d, &p), ScaffoldPhase::Writing);
    }
}
