//! [`IdempotencyClass`] — per-tool declared idempotency mechanism (D38 §2).
//! Seam B of the runtime's effect/commit story; drives the executor's
//! dispatch protocol selection for WORLD-MUTATING tools.

use serde::{Deserialize, Serialize};

/// Per-tool declared idempotency mechanism (D38 §2). Drives the executor's
/// dispatch protocol selection for WORLD-MUTATING tools. The tool author
/// declares this at registration; the executor reads it at dispatch.
///
/// **No `Default` impl** — the field is required on every `ToolDef`. A
/// silent default is exactly how a token-less WM tool ends up mis-classified
/// as something safer, which is the failure D38 §2c exists to prevent. Every
/// tool MUST declare its class explicitly.
///
/// # Variant scopes
///
/// - [`Token`](Self::Token) — the tool accepts idempotency tokens (D38 §1).
///   The broker sets `EffectRequest.idempotency_key = mote.id.to_hex()`; the
///   remote API's idempotency contract backstops the effect→commit window.
/// - [`Readback`](Self::Readback) — the tool supports deterministic
///   read-back (D38 §2a). The executor probes world state keyed on `MoteId`
///   before dispatch; skips if already applied. Probe is deterministic;
///   never a model call. Naturally suits **read-only tools** where the
///   dispatch IS the probe.
/// - [`Staged`](Self::Staged) — the tool requires staged-intent journaling
///   (D38 §2b), and this is **ENFORCED at runtime**: `kx-journal` carries the
///   `EffectStaged` entry kind, `kx-executor`'s commit protocol runs
///   `append(EffectStaged) → dispatch → verify → Committed`, and lifecycle
///   recovery de-duplicates a staged-but-uncommitted effect on replay rather
///   than blindly re-dispatching it.
/// - [`AtLeastOnce`](Self::AtLeastOnce) — the tool has no closing mechanism
///   (D38 §2c). The executor refuses to dispatch it unless the workflow
///   submission context's `accept_at_least_once` is `true` (a property of the
///   submission spec, NOT the warrant).
///
/// # Example
///
/// ```
/// use kx_tool_registry::IdempotencyClass;
/// // All four variants exist and are inequal — the field is enum-shaped
/// // to make mis-classification a compile-time / serialization error.
/// assert_ne!(IdempotencyClass::Token, IdempotencyClass::Readback);
/// assert_ne!(IdempotencyClass::Staged, IdempotencyClass::AtLeastOnce);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum IdempotencyClass {
    /// The tool accepts idempotency tokens (D38 §1). Broker sets
    /// `EffectRequest.idempotency_key = mote.id.to_hex()`; remote API's
    /// idempotency contract backstops the effect→commit window.
    Token,
    /// The tool supports deterministic read-back (D38 §2a). Executor
    /// probes world state keyed on `MoteId`; skips dispatch if already
    /// applied. Probe is deterministic; never a model call.
    Readback,
    /// The tool requires staged-intent journaling (D38 §2b) — **enforced**.
    /// The executor appends `EffectStaged` before dispatching, verifies, then
    /// commits; recovery reads that entry to de-duplicate an effect that was
    /// staged but not yet committed, instead of re-dispatching it.
    Staged,
    /// The tool has no closing mechanism (D38 §2c). The executor refuses to
    /// dispatch it unless the workflow submission context's
    /// `accept_at_least_once` is `true`.
    AtLeastOnce,
}
