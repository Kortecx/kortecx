//! The kortecx **skill manifest** (`kortecx.skill/v1`) — the declarative,
//! portable unit of *know-how*: instructions + a tool grant-WISH set + catalog
//! metadata. A skill is what an App attaches (the `kx-app` `SkillRef` rail) to
//! gain a reusable capability without authoring a blueprint change.
//!
//! ## What a skill is (and is not)
//! - **Declarative only** (D159/D174.4): instructions markdown + `(tool_id →
//!   version)` wishes. NO code — the executable leg is always an out-of-process
//!   MCP connector or a bundled broker capability.
//! - **Never authority** (SN-8): a manifest cannot carry a warrant, grant,
//!   secret, or credential — [`DENY_KEYS`] are refused anywhere in the raw
//!   tree, the shape is closed (`deny_unknown_fields`), and the server grants
//!   only `wish ∩ caller grants ∩ fireable` at bind. A skill on its own grants
//!   nothing.
//! - **Content-addressed**: the instructions body lives in the content store;
//!   the stored manifest carries its 64-hex ref, and the catalog `skill_ref`
//!   is derived over the canonical manifest bytes (sorted keys, no floats).
//!
//! ## Two forms, one type
//! [`SkillManifest`] serves both the **pack form** (`skill.json` beside
//! `instructions.md` — see [`SkillPack`]) and the **stored form** (the catalog
//! row, `instructions_ref` filled). `AddSkill` turns the former into the
//! latter by storing the body and splicing the server-derived ref.
//!
//! Pure leaf: `serde` / `serde_json` / `thiserror` only. Never the journal, the
//! gateway service, or the frozen trio — a dependency-wall test enforces it.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod manifest;
mod pack;

pub use manifest::{
    canonical_json, SkillError, SkillManifest, DENY_KEYS, MAX_SKILL_INSTRUCTIONS_BYTES,
    MAX_SKILL_MANIFEST_BYTES, SKILL_SCHEMA,
};
pub use pack::SkillPack;
