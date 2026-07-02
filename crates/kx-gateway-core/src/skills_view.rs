//! The RC-SW1 skill-catalog seam behind `ListSkills` / `GetSkillForm` /
//! `AddSkill` / `RemoveSkill`.
//!
//! A "skill" is a `kortecx.skill/v1` manifest — instructions (content-store, by
//! ref) plus a tool grant-WISH set (`id → version`) the server re-resolves at
//! bind (`wish ∩ caller grants ∩ fireable`). Spoken in gateway-core's own wire
//! vocabulary — **opaque manifest BYTES** in, a host-derived [`SkillRecord`]
//! out. No manifest type crosses the seam, so gateway-core never links
//! `kx-skill`; the host (`kx-gateway`) validates + canonicalizes the manifest
//! and derives the record + `skill_ref`.
//!
//! # Boundaries (load-bearing)
//!
//! - **Off the truth path.** The `skills.db` sidecar is REBUILDABLE-TO-EMPTY
//!   (the `apps.db`/D160 posture): a skill references a content-store blob +
//!   registry ids; it is NOT journal-derivable. Never journaled, never a
//!   `MoteId` input, never a digest input.
//! - **Carries NO authority (SN-8).** The manifest holds instructions + a WISH
//!   set only — the host refuses authority deny-keys fail-closed, and the bind
//!   grants only `wish ∩ caller grants ∩ fireable`. A skill on its own grants
//!   nothing (the conformance harness pins it).
//! - **Server-derived id.** `skill_ref = blake3("kx-skill\0" ‖ name ‖ canonical(manifest))[..16]`;
//!   the client sends bytes, never an identity. The host re-canonicalizes so
//!   client byte-ordering never affects identity.
//! - **Caller-scoped.** Every method takes the SERVER-RESOLVED `principal`;
//!   uniform not-found for absent OR not-owned (no cross-party existence
//!   oracle).
//! - **`None` seam ⇒ degrade.** A host without the sidecar leaves the four
//!   RPCs `unimplemented` (a clear, fail-closed signal old clients understand).

use crate::error::GatewayError;

/// Fail-closed cap on a serialized skill manifest (checked at the `AddSkill`
/// handler BEFORE any host/store touch). Kept equal to `kx-skill`'s own parse
/// cap — the host pins the equality in a test (the crates sit on opposite
/// sides of the no-`kx-skill`-in-gateway-core wall, so the value is mirrored,
/// not imported).
pub const MAX_SKILL_MANIFEST_BYTES: usize = 64 << 10; // 64 KiB

/// Fail-closed cap on an `AddSkill` instructions body (mirrors `kx-skill`'s
/// `MAX_SKILL_INSTRUCTIONS_BYTES`; equality pinned host-side).
pub const MAX_SKILL_INSTRUCTIONS_BODY_BYTES: usize = 256 << 10; // 256 KiB

/// Display cap on the stored instructions preview (`GetSkillForm` excerpt).
pub const SKILL_PREVIEW_CAP_BYTES: usize = 4 << 10; // 4 KiB

/// The stored instructions the `AddSkill` handler minted via the content-write
/// seam (SN-8: the ref is server-derived; the preview is display-only).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddedInstructions {
    /// The 32-byte content-store ref of the stored body.
    pub content_ref: [u8; 32],
    /// A display excerpt (UTF-8 lossy, ≤ [`SKILL_PREVIEW_CAP_BYTES`]).
    pub preview: String,
    /// True iff the body was longer than the preview cap.
    pub truncated: bool,
}

/// A stored skill's catalog/display view. The manifest bytes are opaque to
/// gateway-core; the host derives every field from the canonical JSON.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkillRecord {
    /// 16-byte SERVER-DERIVED canonical-manifest hash (display + dedup signal).
    pub skill_ref: [u8; 16],
    /// The catalog key (the manifest `name`, `[a-z0-9._-]`).
    pub name: String,
    /// Manifest version (integer string).
    pub version: String,
    /// Advisory description (never parsed for enforcement).
    pub description: String,
    /// Catalog tags.
    pub tags: Vec<String>,
    /// 64-hex content-store ref to the instructions body.
    pub instructions_ref: String,
    /// The tool grant-WISH set (`tool_id → version`); a wish, never a grant.
    pub tools: std::collections::BTreeMap<String, String>,
    /// Display excerpt of the instructions (`''` when the skill was added by ref).
    pub instructions_preview: String,
    /// True iff the stored body exceeded the preview cap.
    pub preview_truncated: bool,
}

/// The skill-catalog store seam: add / enumerate / fetch / remove a caller's
/// skills. Opaque manifest bytes cross the seam; identity + the record are
/// host-derived. A `None` seam on the service ⇒ the four RPCs return
/// `unimplemented`.
pub trait SkillCatalog: Send + Sync {
    /// Upsert the manifest under its own `name` for `principal`. The host
    /// validates + canonicalizes `manifest_json` (authority deny-keys fail
    /// closed), splices `instructions` when the handler stored a body (pack
    /// form), derives `skill_ref` + the record, and stores the canonical
    /// bytes. Returns `(record, deduplicated)` where `deduplicated` is `true`
    /// iff an identical canonical manifest was already bound to this name.
    ///
    /// # Errors
    /// [`GatewayError::InvalidArgument`] if the manifest fails validation (or
    /// carries no `instructions_ref` while `instructions` is `None`);
    /// [`GatewayError::Internal`] on a host write failure.
    fn add(
        &self,
        principal: &str,
        manifest_json: &[u8],
        instructions: Option<AddedInstructions>,
    ) -> Result<(SkillRecord, bool), GatewayError>;

    /// List `principal`'s skills in deterministic name order, paged. Returns
    /// `(records, has_more)`; `after_name` is an exclusive cursor.
    ///
    /// # Errors
    /// A host read failure ([`GatewayError::Internal`]).
    fn list(
        &self,
        principal: &str,
        limit: usize,
        after_name: Option<&str>,
    ) -> Result<(Vec<SkillRecord>, bool), GatewayError>;

    /// Fetch the record bound to `(principal, name)`, if any (caller-scoped;
    /// uniform not-found for absent OR not-owned).
    ///
    /// # Errors
    /// A host read failure ([`GatewayError::Internal`]).
    fn get(&self, principal: &str, name: &str) -> Result<Option<SkillRecord>, GatewayError>;

    /// Remove the skill bound to `(principal, name)`. Returns `true` iff a row
    /// was removed (`false` is uniform for absent OR not-owned).
    ///
    /// # Errors
    /// A host write failure ([`GatewayError::Internal`]).
    fn remove(&self, principal: &str, name: &str) -> Result<bool, GatewayError>;
}
