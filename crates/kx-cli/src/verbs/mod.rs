//! The gRPC client verbs. Each module exposes an `Args` struct, a hand-rolled
//! `parse(...)`, and an async `execute(...)`. Shared `--wait` finishing lives
//! here (used by `invoke` + `submit`).

pub mod blueprint;
pub mod content;
pub mod events;
pub mod health;
pub mod invoke;
pub mod models;
pub mod projection;
pub mod signatures;
pub mod submit;
pub mod tools;

use std::io::Write;
use std::path::Path;

use crate::error::CliError;
use crate::format;
use crate::wait::{WaitOutcome, WaitState};

/// Emit a `--wait` outcome (writing the committed payload to `--out` if given)
/// and map its terminal disposition to the exit-code contract: `Committed` →
/// success, `Failed` → exit 1, `Running` (timeout) → exit 3 (resumable). The
/// result object is always printed first so an agent gets the handle back even
/// on a timeout.
pub(crate) fn finish_wait(
    outcome: &WaitOutcome,
    json: bool,
    out: Option<&Path>,
) -> Result<(), CliError> {
    let include_payload = out.is_none();
    if let (Some(path), Some(payload)) = (out, &outcome.payload) {
        std::fs::write(path, payload)
            .map_err(|e| CliError::Io(format!("--out {}: {e}", path.display())))?;
    }
    let rendered = format::render_wait(outcome, json, include_payload);
    // Use a single locked write so the object is emitted atomically.
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{rendered}").map_err(|e| CliError::Io(e.to_string()))?;

    match outcome.state {
        WaitState::Committed => Ok(()),
        WaitState::Failed => Err(CliError::Failed),
        WaitState::Running => Err(CliError::WaitTimeout),
    }
}
