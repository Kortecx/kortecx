#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # kx-mote — the atomic execution unit
//!
//! P1.1 ships an empty seed. P1.2 fills this crate with:
//!
//! - The `Mote` type, `MoteId`, `MoteGraph`.
//! - The lifecycle state machine (`Pending → Scheduled → Running → {Committed, Failed, Repudiated}`)
//!   per `docs/design/mote.md` §7.
//! - The idempotency-identity derivation per `docs/design/idempotency.md`.
//! - The `NdClass` tag (`PURE` / `READ_ONLY_NONDET` / `WORLD_MUTATING`) per `mote.md` §6.
//! - Typed dependency edges per `mote.md` §5.
//!
//! Per the design discipline, this crate has **zero runtime dependencies** (no I/O, no async,
//! no runtime crates) — it is the narrow waist every other kortecx crate imports.
