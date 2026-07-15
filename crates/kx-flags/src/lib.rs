//! kortecx **feature-flag seam** — the dark-launch mechanism.
//!
//! A feature that is not finished can still land on `main`, provided it is
//! **inert**: gated behind a flag that is OFF unless an operator deliberately
//! turns it on. That decouples *merging* from *releasing*, which is what lets a
//! large feature arrive as several small, reviewable, parallel PRs instead of
//! one all-or-nothing PR. `CONTRIBUTING.md` ("Working in parallel") is the
//! contributor-facing version of this; this crate is the mechanism.
//!
//! ## Using it
//! ```
//! use kx_flags::{enabled, Flag};
//!
//! if enabled(&Flag::SERVE_AUTOGRANT) {
//!     // the half-built path — invisible on main until someone opts in
//! }
//! ```
//!
//! Flags are **typed**, not strings: `Flag::SERVE_AUTOGRNAT` is a compile error,
//! where a mistyped string key would silently read `false` forever — a dark
//! feature that can never be switched on, and no test would catch it.
//!
//! ## Resolution
//! `KX_FLAG_<NAME>` → the flag's legacy alias (if any) → the default. Truthy is
//! `1`/`true`/`yes`/`on`; falsy is `0`/`false`/`no`/`off`; **anything else is the
//! default**, so a typo in a *value* can never silently flip a flag on. Parsing
//! trims and ignores case. The precedence shape mirrors the one the serve already
//! documents for its worker-pool knob (`kx-gateway`'s `resolve_worker_pool`:
//! flag → `KX_WORKERS` → `KX_SERVE_WORKER_POOL` → default).
//!
//! The legacy alias exists so a knob that already shipped under its own name can
//! move onto this seam without breaking anyone's scripts: both names keep working,
//! and the canonical one wins if both are set.
//!
//! ## Invariants
//! - **Every flag defaults to `false`.** Pinned by a test over [`Flag::ALL`], so
//!   an unset process is byte-identical to one built before the flag existed.
//!   A default-ON kill-switch is deliberately *not* expressible here — that is a
//!   different thing with different semantics, and conflating the two is how you
//!   get a "flag" that fails open.
//! - **No global cache.** [`enabled`] reads the environment at point-of-use. The
//!   process environment is constant for a serve's lifetime, so a flag reads the
//!   same value every call and determinism holds — while staying trivially
//!   testable, with no process-global state to reset between tests.
//! - **The resolver is pure and total.** [`resolve`] takes the raw strings rather
//!   than reading them, so the whole decision table is unit- and property-testable
//!   without mutating process-global env (which races under the parallel test
//!   runner). No input panics.
//! - **A dependency-free leaf**, off the journal + identity path. Nothing on the
//!   commit path depends on it, so it cannot move the canonical digest; the frozen
//!   trio never depends on it.

mod flag;
mod resolve;

pub use crate::flag::Flag;
pub use crate::resolve::{enabled, parse_bool, resolve, ENV_PREFIX};
