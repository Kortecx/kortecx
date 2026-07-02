//! The [`SkillManifest`] type + its canonical (de)serialization and validation.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The manifest schema/version tag. Readers fail closed on a mismatch.
pub const SKILL_SCHEMA: &str = "kortecx.skill/v1";

/// Hard cap on the serialized manifest (`skill.json`) — enforced BEFORE any parse.
pub const MAX_SKILL_MANIFEST_BYTES: usize = 64 << 10;

/// Hard cap on the instructions body (`instructions.md`).
pub const MAX_SKILL_INSTRUCTIONS_BYTES: usize = 256 << 10;

/// Object keys that may never appear anywhere in a skill manifest (authority /
/// executable smuggling — SN-8: the artifact wishes, the server grants). The
/// check is substring-on-lowercased-key, so `toolGrants`, `client_secret`,
/// `awsCredentials`, … are all refused.
pub const DENY_KEYS: &[&str] = &["warrant", "grant", "secret", "credential", "executable"];

/// Errors from manifest (de)serialization and validation.
#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    /// The bytes were not valid JSON / did not match the manifest shape.
    #[error("invalid skill manifest JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// `schema` was absent or not [`SKILL_SCHEMA`].
    #[error("unsupported skill schema {got:?} (expected {expected:?})")]
    Schema {
        /// The schema tag found in the manifest.
        got: String,
        /// The schema tag this binary supports.
        expected: &'static str,
    },
    /// A structural / value-level validation failure (bad name, authority key, a float, …).
    #[error("invalid skill manifest: {0}")]
    Invalid(String),
    /// A pack file could not be read (missing, oversized, or not UTF-8).
    #[error("invalid skill pack: {0}")]
    Pack(String),
}

fn default_version() -> String {
    "1".to_string()
}

/// A declarative `kortecx.skill/v1` bundle: instructions (by content-store ref)
/// plus a tool grant-WISH set. Carries NO code and NO authority — the wire/bind
/// shape it produces is the existing `kx-app` `SkillRef` (`name`,
/// `instructions_ref`, `tools`), and the server intersects the wish against the
/// caller's grants plus the live broker at bind.
///
/// Two forms share this one type:
/// - **pack form** (`skill.json` in a skill pack): `instructions_ref` is EMPTY —
///   the ref is server-derived when the pack's `instructions.md` is stored.
/// - **stored form** (the catalog row / `AddSkill` result): `instructions_ref`
///   is the 64-hex content-store ref.
///
/// `deny_unknown_fields` keeps the shape closed: a manifest cannot smuggle new
/// rails past validation (the deny-key walk is defense-in-depth on top).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillManifest {
    /// Must equal [`SKILL_SCHEMA`]; readers fail closed on a mismatch.
    pub schema: String,
    /// The catalog key: 1–64 chars of `[a-z0-9._-]`.
    pub name: String,
    /// Manifest version (an integer string, e.g. `"1"`).
    #[serde(default = "default_version")]
    pub version: String,
    /// Advisory description (display only; never parsed for enforcement).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Advisory catalog tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// 64-char lowercase-hex content-store ref to the instructions body.
    /// Empty in pack form (server-derived at add time).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub instructions_ref: String,
    /// The tool grant-WISH set (`tool_id` → integer version string). A wish is
    /// never authority: the server grants only `wish ∩ caller grants ∩ fireable`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tools: BTreeMap<String, String>,
}

impl SkillManifest {
    /// Parse + validate a PACK-form manifest (`instructions_ref` must be empty).
    ///
    /// # Errors
    /// [`SkillError::Json`] on malformed JSON / unknown fields, [`SkillError::Schema`]
    /// on a schema mismatch, [`SkillError::Invalid`] on any structural violation
    /// (authority key, float, bad name/tool grammar, a non-empty `instructions_ref`).
    pub fn from_json_slice_pack(bytes: &[u8]) -> Result<Self, SkillError> {
        let m = Self::parse_common(bytes)?;
        m.validate_pack()?;
        Ok(m)
    }

    /// Parse + validate a STORED-form manifest (`instructions_ref` must be 64-hex).
    ///
    /// # Errors
    /// As [`SkillManifest::from_json_slice_pack`], except `instructions_ref` must
    /// be a 64-char lowercase-hex content ref.
    pub fn from_json_slice_stored(bytes: &[u8]) -> Result<Self, SkillError> {
        let m = Self::parse_common(bytes)?;
        m.validate_stored()?;
        Ok(m)
    }

    /// Shared raw-tree checks (floats + authority keys) BEFORE the typed parse,
    /// so nothing can hide behind a shape error.
    fn parse_common(bytes: &[u8]) -> Result<Self, SkillError> {
        if bytes.len() > MAX_SKILL_MANIFEST_BYTES {
            return Err(SkillError::Invalid(format!(
                "manifest is {} bytes (cap {MAX_SKILL_MANIFEST_BYTES})",
                bytes.len()
            )));
        }
        let raw: Value = serde_json::from_slice(bytes)?;
        reject_floats(&raw)?;
        deny_authority_keys(&raw)?;
        Ok(serde_json::from_value(raw)?)
    }

    /// Structural validation shared by both forms.
    fn validate_common(&self) -> Result<(), SkillError> {
        if self.schema != SKILL_SCHEMA {
            return Err(SkillError::Schema {
                got: self.schema.clone(),
                expected: SKILL_SCHEMA,
            });
        }
        check_name("skill.name", &self.name)?;
        check_integer("skill.version", &self.version)?;
        for (id, version) in &self.tools {
            check_tool_id(id)?;
            check_integer(&format!("skill.tools[{id}]"), version)?;
        }
        for tag in &self.tags {
            if tag.is_empty() {
                return Err(SkillError::Invalid("skill.tags must be non-empty".into()));
            }
        }
        Ok(())
    }

    /// Validate the PACK form (`instructions_ref` empty — server-derived at add).
    ///
    /// # Errors
    /// [`SkillError::Schema`] / [`SkillError::Invalid`] as described on the type.
    pub fn validate_pack(&self) -> Result<(), SkillError> {
        self.validate_common()?;
        if !self.instructions_ref.is_empty() {
            return Err(SkillError::Invalid(
                "a pack manifest must not carry instructions_ref (it is server-derived when \
                 instructions.md is stored)"
                    .into(),
            ));
        }
        Ok(())
    }

    /// Validate the STORED form (`instructions_ref` is a 64-hex content ref).
    ///
    /// # Errors
    /// [`SkillError::Schema`] / [`SkillError::Invalid`] as described on the type.
    pub fn validate_stored(&self) -> Result<(), SkillError> {
        self.validate_common()?;
        check_ref("skill.instructions_ref", &self.instructions_ref)
    }

    /// Canonical bytes: keys sorted (via [`serde_json::Value`]), compact, no
    /// floats. The hashable form — the host derives `skill_ref` over it.
    ///
    /// # Errors
    /// [`SkillError::Json`] if the manifest cannot be serialized (it never can in
    /// practice — the type holds only JSON-safe fields).
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, SkillError> {
        let value = serde_json::to_value(self)?;
        Ok(serde_json::to_vec(&value)?)
    }

    /// The human export form: pretty (2-space) + sorted keys + a trailing newline.
    ///
    /// # Errors
    /// [`SkillError::Json`] if the manifest cannot be serialized.
    pub fn to_pretty_json(&self) -> Result<String, SkillError> {
        let value = serde_json::to_value(self)?;
        let mut s = serde_json::to_string_pretty(&value)?;
        s.push('\n');
        Ok(s)
    }
}

/// Re-canonicalize STORED-form manifest bytes (validates first). The gateway
/// host derives `skill_ref` over this form, so client byte-ordering never
/// affects identity.
///
/// # Errors
/// The [`SkillError`] from [`SkillManifest::from_json_slice_stored`].
pub fn canonical_json(bytes: &[u8]) -> Result<Vec<u8>, SkillError> {
    SkillManifest::from_json_slice_stored(bytes)?.to_canonical_json()
}

fn check_ref(field: &str, r: &str) -> Result<(), SkillError> {
    if r.len() != 64
        || !r
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(SkillError::Invalid(format!(
            "{field} must be 64-char lowercase hex, got {r:?}"
        )));
    }
    Ok(())
}

fn check_name(field: &str, name: &str) -> Result<(), SkillError> {
    if name.is_empty() || name.len() > 64 {
        return Err(SkillError::Invalid(format!(
            "{field} must be 1-64 chars, got {} chars",
            name.len()
        )));
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err(SkillError::Invalid(format!(
            "{field} may only contain [a-z0-9._-], got {name:?}"
        )));
    }
    Ok(())
}

/// A wished tool id: one or two non-empty `[a-z0-9._-]` segments, e.g.
/// `retrieve` (a bundled capability) or `gmail/search` (a connector tool).
fn check_tool_id(id: &str) -> Result<(), SkillError> {
    let segments: Vec<&str> = id.split('/').collect();
    let valid = id.len() <= 128
        && (1..=2).contains(&segments.len())
        && segments.iter().all(|s| {
            !s.is_empty()
                && s.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
                })
        });
    if !valid {
        return Err(SkillError::Invalid(format!(
            "skill.tools key must be a tool id ('name' or 'server/name' of [a-z0-9._-]), got {id:?}"
        )));
    }
    Ok(())
}

fn check_integer(field: &str, v: &str) -> Result<(), SkillError> {
    if v.parse::<u64>().is_err() {
        return Err(SkillError::Invalid(format!(
            "{field} must be an integer string, got {v:?}"
        )));
    }
    Ok(())
}

/// Walk a JSON value and reject any non-integer number (SN-8 — no floats on identity).
fn reject_floats(v: &Value) -> Result<(), SkillError> {
    match v {
        Value::Number(n) => {
            if !n.is_i64() && !n.is_u64() {
                return Err(SkillError::Invalid(format!("floats are not allowed: {n}")));
            }
            Ok(())
        }
        Value::Array(a) => a.iter().try_for_each(reject_floats),
        Value::Object(o) => o.values().try_for_each(reject_floats),
        _ => Ok(()),
    }
}

/// Refuse any object key that smells like authority, ANYWHERE in the raw tree.
/// The top-level `tools` map is the wish set and is not descended (its keys are
/// tool IDS, validated against the tool-id grammar instead — mirroring the
/// `kx-app` `secret_leak` `NO_DESCEND` posture for the same rail).
fn deny_authority_keys(root: &Value) -> Result<(), SkillError> {
    fn walk(v: &Value, at_root: bool) -> Result<(), SkillError> {
        match v {
            Value::Object(o) => {
                for (k, child) in o {
                    if at_root && k == "tools" {
                        continue;
                    }
                    let lk = k.to_ascii_lowercase();
                    if let Some(hit) = DENY_KEYS.iter().find(|d| lk.contains(**d)) {
                        return Err(SkillError::Invalid(format!(
                            "authority key {k:?} is forbidden in a skill manifest \
                             (matched {hit:?}); a skill wishes, the server grants"
                        )));
                    }
                    walk(child, false)?;
                }
                Ok(())
            }
            Value::Array(a) => a.iter().try_for_each(|c| walk(c, false)),
            _ => Ok(()),
        }
    }
    walk(root, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack_manifest_json() -> String {
        r#"{
            "schema": "kortecx.skill/v1",
            "name": "email-triage",
            "version": "1",
            "description": "Triage an inbox.",
            "tags": ["email", "gmail"],
            "tools": { "gmail/search": "1", "gmail/read": "1" }
        }"#
        .to_string()
    }

    #[test]
    fn pack_form_parses_and_validates() {
        let m = SkillManifest::from_json_slice_pack(pack_manifest_json().as_bytes()).unwrap();
        assert_eq!(m.name, "email-triage");
        assert_eq!(m.tools.len(), 2);
        assert!(m.instructions_ref.is_empty());
    }

    #[test]
    fn canonical_json_is_idempotent_and_sorted() {
        let mut m = SkillManifest::from_json_slice_pack(pack_manifest_json().as_bytes()).unwrap();
        m.instructions_ref = "a".repeat(64);
        let c1 = m.to_canonical_json().unwrap();
        let re = SkillManifest::from_json_slice_stored(&c1).unwrap();
        let c2 = re.to_canonical_json().unwrap();
        assert_eq!(c1, c2, "canonicalization must be idempotent");
        let s = String::from_utf8(c1).unwrap();
        assert!(s.starts_with("{\"description\""), "keys sorted: {s}");
    }

    #[test]
    fn schema_mismatch_fails_closed() {
        let bad = pack_manifest_json().replace("kortecx.skill/v1", "kortecx.skill/v2");
        let err = SkillManifest::from_json_slice_pack(bad.as_bytes()).unwrap_err();
        assert!(matches!(err, SkillError::Schema { .. }), "{err}");
    }

    #[test]
    fn authority_keys_are_refused_at_any_depth() {
        for smuggle in [
            r#""warrant": {"x": 1}"#,
            r#""tool_grants": []"#,
            r#""clientSecret": "sh""#,
            r#""awsCredentials": "x""#,
            r#""executable": "sh""#,
        ] {
            let bad = pack_manifest_json().replace(
                r#""description": "Triage an inbox.","#,
                &format!(r#""description": "Triage an inbox.", {smuggle},"#),
            );
            let err = SkillManifest::from_json_slice_pack(bad.as_bytes()).unwrap_err();
            assert!(
                err.to_string().contains("forbidden"),
                "{smuggle} must be refused, got {err}"
            );
        }
    }

    #[test]
    fn unknown_fields_are_refused() {
        let bad = pack_manifest_json().replace(
            r#""version": "1","#,
            r#""version": "1", "context_files": [],"#,
        );
        assert!(SkillManifest::from_json_slice_pack(bad.as_bytes()).is_err());
    }

    #[test]
    fn floats_are_refused_anywhere() {
        let bad = pack_manifest_json().replace(r#""version": "1""#, r#""version": 1.5"#);
        let err = SkillManifest::from_json_slice_pack(bad.as_bytes()).unwrap_err();
        assert!(err.to_string().contains("float"), "{err}");
    }

    #[test]
    fn tool_id_grammar_accepts_bare_and_namespaced_only() {
        let mut m = SkillManifest::from_json_slice_pack(pack_manifest_json().as_bytes()).unwrap();
        m.tools.insert("retrieve".into(), "1".into());
        m.validate_pack().unwrap();
        for bad in ["Gmail/Search", "a/b/c", "/x", "x/", "", "a b"] {
            let mut n = m.clone();
            n.tools.insert(bad.into(), "1".into());
            assert!(n.validate_pack().is_err(), "{bad:?} must be refused");
        }
        let mut nonint = m.clone();
        nonint.tools.insert("retrieve".into(), "latest".into());
        assert!(nonint.validate_pack().is_err(), "non-integer version");
    }

    #[test]
    fn name_grammar_is_enforced() {
        for bad in ["", "UPPER", "has space", "a@b", &"x".repeat(65)] {
            let m = SkillManifest {
                schema: SKILL_SCHEMA.into(),
                name: (*bad).into(),
                version: "1".into(),
                ..SkillManifest::default()
            };
            assert!(m.validate_pack().is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn pack_form_refuses_a_carried_instructions_ref_and_stored_requires_it() {
        let mut m = SkillManifest::from_json_slice_pack(pack_manifest_json().as_bytes()).unwrap();
        assert!(m.validate_stored().is_err(), "stored form needs the ref");
        m.instructions_ref = "a".repeat(64);
        m.validate_stored().unwrap();
        assert!(m.validate_pack().is_err(), "pack form must not carry a ref");
        m.instructions_ref = "A".repeat(64);
        assert!(m.validate_stored().is_err(), "uppercase hex refused");
    }

    #[test]
    fn oversized_manifest_is_refused_before_parse() {
        let huge = format!(
            r#"{{"schema":"kortecx.skill/v1","name":"x","description":"{}"}}"#,
            "d".repeat(MAX_SKILL_MANIFEST_BYTES)
        );
        let err = SkillManifest::from_json_slice_pack(huge.as_bytes()).unwrap_err();
        assert!(err.to_string().contains("cap"), "{err}");
    }
}
