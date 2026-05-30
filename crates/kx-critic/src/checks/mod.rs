//! The four deterministic checks. Each `eval(&Spec, &[u8]) -> CriticVerdict` is
//! pure, total, and deterministic (see the crate-level contract).

pub mod dedup;
pub mod pii;
pub mod schema;
pub mod stat_bounds;
