//! The [`TaskBundle`] type + its content-addressed fingerprint.
//!
//! Canonical-encoding discipline (the `MoteDef::hash` / `task_signature_hash`
//! precedent): every collection is a `BTreeMap`/`BTreeSet` (deterministic
//! order), every numeric field is an integer (no floats anywhere — SN-8
//! forbids float confidence on anything that could be persisted), and the
//! fingerprint is `blake3(domain-tag ‖ canonical bincode)` so two bundles
//! built in different insertion orders hash identically.

use std::collections::{BTreeMap, BTreeSet};

use kx_mote::{canonical_config, ToolName, ToolVersion};
use serde::{Deserialize, Serialize};

/// Bump on any change to the encoding bytes of [`TaskBundle`] (field add /
/// remove / reorder / type change). Encoded in the body, so a bump re-derives
/// every fingerprint.
pub const TASK_BUNDLE_SCHEMA_VERSION: u16 = 1;

/// The blake3 domain tag for [`TaskBundle::fingerprint`] — keeps bundle
/// fingerprints disjoint from every other 32-byte hash in the system.
const FINGERPRINT_DOMAIN: &[u8] = b"kx-bundle/task-bundle/v1";

/// Advisory, per-tool authoring metadata: a human description plus normalized
/// keywords grouped by BCP-47-ish language tag (e.g. `"en"`, `"hi"`, `"ja"`).
/// Display/ordering material only — never an authority input.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolMeta {
    /// A one-line human description of why this tool is in the sequence.
    pub description: String,
    /// Normalized intent keywords, per language tag (sorted, deduplicated).
    pub keywords: BTreeMap<String, BTreeSet<String>>,
}

/// The 32-byte content-addressed identity of a [`TaskBundle`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskBundleFingerprint(pub [u8; 32]);

impl TaskBundleFingerprint {
    /// Lowercase-hex rendering (for logs / display; never parsed back).
    #[must_use]
    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for b in self.0 {
            use std::fmt::Write;
            let _ = write!(s, "{b:02x}");
        }
        s
    }
}

/// A reusable, content-addressed multi-tool task template.
///
/// `tool_sequence` is the ORDERED list of `(name, version)` pairs the lowered
/// workflow will run as a chain; it MUST be a subset of the executing
/// warrant's `tool_grants` — `kx-toolscout::lower_to_workflow_def` refuses
/// otherwise (exact equality, the planner-IMP-5 gate). `tolerance_threshold_bp`
/// is the advisory ranking cut in basis points (`0..=10_000`) — an integer by
/// construction so no float confidence can ever be persisted (SN-8).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskBundle {
    /// See [`TASK_BUNDLE_SCHEMA_VERSION`] (encoded → identity-bearing).
    pub schema_version: u16,
    /// The task's instruction (becomes each lowered step's prompt config).
    pub intent: String,
    /// Language tags the intent/keywords are expressed in (advisory).
    pub language_tags: BTreeSet<String>,
    /// The ordered tools the lowered workflow runs (exact identity pairs).
    pub tool_sequence: Vec<(ToolName, ToolVersion)>,
    /// Advisory per-tool metadata (keyed by name — one entry per tool name).
    pub tool_metadata: BTreeMap<ToolName, ToolMeta>,
    /// The advisory ranking cut, basis points `0..=10_000` (never a float).
    pub tolerance_threshold_bp: u16,
}

impl TaskBundle {
    /// The content-addressed identity: `blake3(domain-tag ‖ canonical bincode)`.
    ///
    /// A computed method, not a stored field (the [`kx_catalog`-style]
    /// `task_signature_hash` shape): a stored self-hash would be
    /// self-referential and could silently drift from the bytes.
    ///
    /// [`kx_catalog`-style]: https://docs.rs/kx-catalog
    ///
    /// # Panics
    ///
    /// Never in practice: the canonical bincode encode of this struct is
    /// infallible (no floats, no non-encodable types — the `MoteDef::hash`
    /// precedent); the `expect` documents that invariant.
    #[must_use]
    pub fn fingerprint(&self) -> TaskBundleFingerprint {
        #[allow(clippy::expect_used)] // SAFETY: no floats, no maps with non-Ord
        // keys, no non-encodable types — canonical bincode of this struct is
        // infallible (the MoteDef::hash precedent).
        let bytes = bincode::serde::encode_to_vec(self, canonical_config())
            .expect("TaskBundle serialization is infallible (no floats, no non-encodable types)");
        let mut hasher = blake3::Hasher::new();
        hasher.update(FINGERPRINT_DOMAIN);
        hasher.update(&bytes);
        TaskBundleFingerprint(*hasher.finalize().as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle() -> TaskBundle {
        let mut keywords = BTreeMap::new();
        keywords.insert(
            "en".to_string(),
            ["echo", "repeat"]
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        );
        TaskBundle {
            schema_version: TASK_BUNDLE_SCHEMA_VERSION,
            intent: "echo the topic back".to_string(),
            language_tags: ["en".to_string()].into_iter().collect(),
            tool_sequence: vec![(
                ToolName("mcp-echo".to_string()),
                ToolVersion("1".to_string()),
            )],
            tool_metadata: BTreeMap::from([(
                ToolName("mcp-echo".to_string()),
                ToolMeta {
                    description: "deterministic echo".to_string(),
                    keywords,
                },
            )]),
            tolerance_threshold_bp: 6_000,
        }
    }

    #[test]
    fn fingerprint_is_deterministic_and_field_sensitive() {
        let a = bundle();
        let b = bundle();
        assert_eq!(a.fingerprint(), b.fingerprint(), "same bytes, same hash");

        let mut c = bundle();
        c.intent.push('!');
        assert_ne!(
            a.fingerprint(),
            c.fingerprint(),
            "intent is identity-bearing"
        );

        let mut d = bundle();
        d.tolerance_threshold_bp = 6_001;
        assert_ne!(
            a.fingerprint(),
            d.fingerprint(),
            "threshold is identity-bearing"
        );
    }

    #[test]
    fn insertion_order_cannot_move_the_fingerprint() {
        // BTree collections canonicalize: build the metadata map in the
        // opposite insertion order and the encoding must be byte-identical.
        let mut x = bundle();
        x.tool_metadata
            .insert(ToolName("a-first".to_string()), ToolMeta::default());
        let mut y = bundle();
        let prior = y.tool_metadata.clone();
        y.tool_metadata = BTreeMap::new();
        y.tool_metadata
            .insert(ToolName("a-first".to_string()), ToolMeta::default());
        y.tool_metadata.extend(prior);
        assert_eq!(x.fingerprint(), y.fingerprint());
    }
}
