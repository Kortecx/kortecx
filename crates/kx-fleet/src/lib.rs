// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! # kx-fleet — teams / fleets (M7, D112)
//!
//! The kortecx **fleet** layer is "the unit users actually want": a **team** that
//! owns and shares catalog recipes, and a **fleet** that nests teams. It is the
//! last M7-era capability before the SDK (M8), and it is built on — never beside —
//! the M7.2 governance machinery (`kx-catalog`).
//!
//! ## What a team IS (and is not)
//!
//! A team is just a group [`PartyId`]: "team `T` may `Use` recipe `A` under warrant
//! `W`" is an ordinary [`kx_catalog::Grant`] on the existing [`GrantLedger`]. The
//! ONLY new truth this crate adds is **who is in a team, under what role** — a set
//! of append-only [`MembershipFact`]s ([`Team`] founding, [`Admit`], [`Removal`],
//! [`Disband`]) folded by a fail-closed [`MembershipLedger`]. A team is **never a
//! stateful in-memory actor** (adheres D109.1 / journal-as-truth): its membership
//! is derived by folding immutable facts, exactly like [`GrantLedger`] derives
//! authority by folding grants.
//!
//! ## "Journaled" = the D-LOCK-4 discipline (NOT a `kx-journal` dependency)
//!
//! Like [`GrantLedger`], the membership ledger is a **separate truth**: append-only,
//! content-addressed, immutable, idempotent, **revoke-by-new-fact**. It is
//! authoritative for *who is in a team*; the journal stays authoritative for *what
//! runs did*. This crate therefore **never depends on `kx-journal`** — the
//! dependency direction is the wall. A durable / cloud backend (D94) implements the
//! same trait; the in-memory backend is rebuildable, not durable.
//!
//! ## Membership is one-level-up delegation (the core insight)
//!
//! A member's effective warrant on a recipe is
//! `intersect(team_grant_warrant, member_role)` via the **FROZEN**
//! [`kx_warrant::intersect`] seam — structurally `⊆` the team's grant, so a member
//! can **never** exceed the team. With **nested fleet-of-teams** it generalizes to a
//! bounded *multi-hop* narrow (member → team → fleet → …), one `intersect` per hop;
//! the fold mirrors `kx-catalog`'s grant-chain fold over membership edges. The
//! composition surface is [`GovernedFleet`] (a [`MembershipLedger`] + a
//! [`GrantLedger`], composed without coupling either impl — Rule 1, exactly
//! `kx_catalog::GovernedCatalog`).
//!
//! ## The SN-8 wall (load-bearing)
//!
//! Like `kx-catalog`, the fleet layer is **off the trust path**: it never gates
//! selection, eviction, or promotion, and carries **no floats** (so even a future
//! mistake could carry none onto a canonical hash). The wall is enforced by the
//! dependency graph — the guarantee-path crates (`kx-scheduler` / `kx-executor` /
//! `kx-projection` / `kx-inference` / `kx-mote` / `kx-journal`) do NOT depend on
//! `kx-fleet`, and neither does `kx-catalog` (the edge is one-way,
//! `kx-fleet → kx-catalog`). Asserted from the manifests in
//! `tests/security_governance.rs`.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown
)]
// `.expect()` on canonical-bincode encode of a type WITHOUT floats and WITHOUT
// non-encodable variants IS infallible (the membership/team content-id sites), and
// `.expect("poisoned lock")` on the `InMemoryMembershipLedger` RwLock is the correct
// propagate-on-catastrophe behavior. Both site classes carry an inline justification;
// this crate-level allow suppresses the workspace `clippy::expect_used = "deny"`
// policy for those documented uses (mirrors kx-catalog / kx-mote / kx-content).
#![allow(clippy::expect_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

mod error;
mod governed;
mod in_memory;
mod ledger;
mod membership;
mod membership_inner;
mod sqlite_membership_ledger;
mod sqlite_util;
mod team;

#[cfg(test)]
mod tests;

pub use error::{GovernedFleetError, MembershipLedgerError};
pub use governed::GovernedFleet;
pub use in_memory::InMemoryMembershipLedger;
pub use ledger::{
    MemberRole, MembershipLedger, MembershipOutcome, TeamEdge, MAX_TEAM_MEMBERS_WALK,
};
pub use membership::{Admit, Disband, MembershipFact, MembershipId, Removal};
pub use sqlite_membership_ledger::{SqliteMembershipLedger, MEMBERSHIP_LEDGER_SCHEMA_VERSION};
pub use team::{Team, TeamId, FLEET_SCHEMA_VERSION};

// REUSE (never modify) the M7.2 governance + frozen narrowing seams — one import
// surface for fleet callers. A team is a `PartyId`, a team grant is a `Grant` on the
// `GrantLedger`, and a membership's runtime scope is a `Role` narrowed via `intersect`.
pub use kx_catalog::{
    AssetRef, CatalogAction, CatalogActionSet, GrantLedger, InMemoryGrantLedger, PartyId,
};
pub use kx_warrant::{intersect, NarrowingError, Role, WarrantSpec};
