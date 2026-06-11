//! [`ToolFingerprint`] — the multilingual, content-addressed tool manifest the
//! advisory index ranks against.

use std::collections::{BTreeMap, BTreeSet};

use kx_content::ContentRef;
use kx_mote::{canonical_config, ToolName, ToolVersion};
use serde::{Deserialize, Serialize};

/// Bump on any change to the encoding bytes of [`ToolFingerprint`].
pub const TOOL_FINGERPRINT_SCHEMA_VERSION: u16 = 1;

/// The blake3 domain tag for [`ToolFingerprint::fingerprint_hash`].
const FINGERPRINT_DOMAIN: &[u8] = b"kx-toolscout/tool-fingerprint/v1";

/// Normalize a keyword for matching: trim, ASCII-lowercase, collapse internal
/// whitespace runs to one space.
///
/// Deliberately dependency-free: NO Unicode case folding / NFC normalization
/// (that would be a new dependency for marginal gain at this tier) — so
/// non-ASCII keywords (e.g. Hindi, Japanese) match by exact codepoints after
/// whitespace collapse. Registrars store keywords pre-normalized in whatever
/// form their language needs; this function is the single normal form both
/// sides (registration + query) pass through, so they cannot drift.
#[must_use]
pub fn normalize_keyword(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for c in s.trim().chars() {
        if c.is_whitespace() {
            pending_space = true;
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

/// A tool's advisory manifest: identity + description + normalized intent
/// keywords per language tag. Ranking material ONLY — the broker never reads
/// this type (advisory-never-authorizes is structural: `kx-capability` has no
/// dependency on this crate).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolFingerprint {
    /// See [`TOOL_FINGERPRINT_SCHEMA_VERSION`] (encoded → identity-bearing).
    pub schema_version: u16,
    /// The tool's exact name (the grant-set identity half).
    pub tool_id: ToolName,
    /// The tool's exact version (the other identity half).
    pub tool_version: ToolVersion,
    /// A one-line human description (rung-2 fuzzy material).
    pub description: String,
    /// Normalized keywords per language tag (sorted, deduplicated). Store
    /// values pre-passed through [`normalize_keyword`].
    pub keywords: BTreeMap<String, BTreeSet<String>>,
}

impl ToolFingerprint {
    /// The content-addressed key this manifest indexes under —
    /// `blake3(domain-tag ‖ canonical bincode)` as a [`ContentRef`], so it
    /// slots straight into [`kx_dataset::RetrievalIndex`] (whose deterministic
    /// tiebreak orders by ascending `ContentRef`).
    ///
    /// # Panics
    ///
    /// Never in practice: the canonical bincode encode of this struct is
    /// infallible (no floats, no non-encodable types — the `MoteDef::hash`
    /// precedent); the `expect` documents that invariant.
    #[must_use]
    pub fn fingerprint_hash(&self) -> ContentRef {
        #[allow(clippy::expect_used)] // SAFETY: no floats, no non-encodable
        // types — canonical bincode of this struct is infallible (the
        // MoteDef::hash precedent).
        let bytes = bincode::serde::encode_to_vec(self, canonical_config()).expect(
            "ToolFingerprint serialization is infallible (no floats, no non-encodable types)",
        );
        let mut hasher = blake3::Hasher::new();
        hasher.update(FINGERPRINT_DOMAIN);
        hasher.update(&bytes);
        ContentRef::from_bytes(*hasher.finalize().as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_trims_lowercases_and_collapses() {
        assert_eq!(normalize_keyword("  Web   Search "), "web search");
        assert_eq!(normalize_keyword("ECHO"), "echo");
        // Non-ASCII passes through by codepoint (documented limitation).
        assert_eq!(normalize_keyword(" खोज "), "खोज");
    }

    #[test]
    fn hash_is_deterministic_and_version_sensitive() {
        let fp = ToolFingerprint {
            schema_version: TOOL_FINGERPRINT_SCHEMA_VERSION,
            tool_id: ToolName("mcp-echo".to_string()),
            tool_version: ToolVersion("1".to_string()),
            description: "echo".to_string(),
            keywords: BTreeMap::new(),
        };
        assert_eq!(fp.fingerprint_hash(), fp.fingerprint_hash());

        let mut v2 = fp.clone();
        v2.tool_version = ToolVersion("2".to_string());
        assert_ne!(fp.fingerprint_hash(), v2.fingerprint_hash());
    }
}
