//! The host [`ToolScoutView`] impl (W1.A5) — advisory tool manifests from the
//! SAME registry surface the serve path resolves against, a startup-built
//! [`ToolManifestIndex`], and the lowering dry-run verdict.
//!
//! # Boundaries (load-bearing)
//!
//! - **Advisory-never-authorizes (SN-8).** The index ranks for a picker; the
//!   verdict is a dry-run of the REAL fail-closed
//!   [`kx_toolscout::lower_to_workflow_def`] gate against the SERVER react
//!   warrant — the lowered `WorkflowDef` is DISCARDED (nothing submits,
//!   nothing journals, no identity derives). The broker re-gates any future
//!   real dispatch.
//! - **Honest advertisement.** Manifests come from `Approved` registry defs
//!   only ([`kx_tool_registry::InMemoryToolRegistry::defs`]) — the same
//!   visibility rule as `lookup`. `mcp-echo@1` is listed only when its
//!   capability actually registered on the serve broker (the caller passes the
//!   matching registry).
//! - **No floats persisted.** Scores stay integer basis points end to end;
//!   the embedder is `None` at this tier (rung-3 neutral — string rungs
//!   decide), so no opaque vector enters the rank.

use std::collections::{BTreeMap, BTreeSet};

use kx_bundle::{TaskBundle, ToolMeta, TASK_BUNDLE_SCHEMA_VERSION};
use kx_dataset::InMemoryRetrievalIndex;
use kx_gateway_core::{
    BundleScoreView, BundleSpecEntry, KeywordSetEntry, LowerVerdictEntry, ManifestScoreEntry,
    ToolManifestEntry, ToolScoutView,
};
use kx_mote::{ModelId, ToolName, ToolVersion};
use kx_tool_registry::{ToolDef, ToolKind};
use kx_toolscout::{
    lower_to_workflow_def, normalize_keyword, ToolFingerprint, ToolManifestIndex,
    TOOL_FINGERPRINT_SCHEMA_VERSION,
};
use kx_warrant::WarrantSpec;

/// The curated host-side keyword table, by `tool_id`. `ToolDef` carries no
/// keywords (its `description` is free-form and NEVER enforcement-parsed), so
/// the host supplies the advisory intent vocabulary for the bundled tools. A
/// tool absent here falls back to its `tool_id` tokens (split on `-`) — every
/// manifest stays rankable.
fn curated_keywords(tool_id: &str) -> Vec<&'static str> {
    match tool_id {
        "fs-read" => vec!["file", "read", "filesystem", "disk", "load", "open"],
        "fs-write" => vec!["file", "write", "save", "filesystem", "disk", "store"],
        "text-summarize" => vec!["text", "summarize", "summary", "condense", "digest"],
        "mcp-echo" => vec!["echo", "repeat", "mirror", "ping"],
        _ => vec![],
    }
}

/// Render a [`ToolKind`] as the manifest's display kind (never parsed back).
fn kind_display(kind: &ToolKind) -> &'static str {
    match kind {
        ToolKind::Builtin => "Builtin",
        ToolKind::LocalScript { .. } => "LocalScript",
        ToolKind::External { .. } => "External",
        ToolKind::Mcp { .. } => "Mcp",
        ToolKind::SelfGenerated { .. } => "SelfGenerated",
    }
}

/// Derive a tool's [`ToolFingerprint`] from its registry def + the curated
/// keyword table (normalized through the SAME `normalize_keyword` the scorer
/// applies, so registration and query cannot drift).
fn fingerprint_from_def(def: &ToolDef) -> ToolFingerprint {
    let curated = curated_keywords(&def.tool_id.0);
    let words: BTreeSet<String> = if curated.is_empty() {
        def.tool_id
            .0
            .split('-')
            .map(normalize_keyword)
            .filter(|w| !w.is_empty())
            .collect()
    } else {
        curated.into_iter().map(normalize_keyword).collect()
    };
    ToolFingerprint {
        schema_version: TOOL_FINGERPRINT_SCHEMA_VERSION,
        tool_id: def.tool_id.clone(),
        tool_version: def.tool_version.clone(),
        description: def.description.clone(),
        keywords: BTreeMap::from([("en".to_string(), words)]),
    }
}

/// The lowering dry-run context: present exactly when this serve has a live
/// react runtime (a fit model + the bundled tool capability) — the SAME
/// conditions under which `kx/recipes/react` is seeded.
pub(crate) struct VerdictCtx {
    /// The SERVER-built react warrant (never a caller-supplied one).
    pub warrant: WarrantSpec,
    /// The resolved serve model (each lowered step's route).
    pub model_id: ModelId,
    /// The capability the lowered generator steps would dispatch through.
    pub capability: ToolName,
}

/// The host toolscout view: pre-rendered manifest entries (deterministic
/// `(tool_id, tool_version)` order), the advisory index, and the optional
/// verdict context. Built ONCE at serve startup; every RPC call is a pure read.
pub(crate) struct HostToolScout {
    entries: Vec<ToolManifestEntry>,
    index: ToolManifestIndex<InMemoryRetrievalIndex>,
    verdict: Option<VerdictCtx>,
}

impl HostToolScout {
    /// Build the view over the registry's `Approved` defs (already in
    /// deterministic order from [`InMemoryToolRegistry::defs`]).
    ///
    /// [`InMemoryToolRegistry::defs`]: kx_tool_registry::InMemoryToolRegistry::defs
    pub(crate) fn new(defs: &[ToolDef], verdict: Option<VerdictCtx>) -> Self {
        let mut index = ToolManifestIndex::new(InMemoryRetrievalIndex::new());
        let mut entries = Vec::with_capacity(defs.len());
        for def in defs {
            let fp = fingerprint_from_def(def);
            entries.push(ToolManifestEntry {
                tool_id: fp.tool_id.0.clone(),
                tool_version: fp.tool_version.0.clone(),
                description: fp.description.clone(),
                keywords: fp
                    .keywords
                    .iter()
                    .map(|(lang, words)| KeywordSetEntry {
                        lang: lang.clone(),
                        words: words.iter().cloned().collect(),
                    })
                    .collect(),
                fingerprint_hash: *fp.fingerprint_hash().as_bytes(),
                kind: kind_display(&def.kind).to_string(),
            });
            // Embedder-less tier: no vector — the string rungs rank (rung-3 neutral).
            index.insert(fp, None);
        }
        Self {
            entries,
            index,
            verdict,
        }
    }

    /// Assemble the server-side [`TaskBundle`] from a validated spec. The
    /// `schema_version` is SERVER-set; every collection lands in its canonical
    /// BTree form, keywords re-normalized through `normalize_keyword`.
    fn bundle_from_spec(spec: &BundleSpecEntry) -> TaskBundle {
        let tool_sequence: Vec<(ToolName, ToolVersion)> = spec
            .tool_sequence
            .iter()
            .map(|t| {
                (
                    ToolName(t.tool_id.clone()),
                    ToolVersion(t.tool_version.clone()),
                )
            })
            .collect();
        let tool_metadata: BTreeMap<ToolName, ToolMeta> = spec
            .tool_sequence
            .iter()
            .map(|t| {
                let keywords: BTreeMap<String, BTreeSet<String>> = t
                    .keywords
                    .iter()
                    .map(|set| {
                        (
                            set.lang.clone(),
                            set.words.iter().map(|w| normalize_keyword(w)).collect(),
                        )
                    })
                    .collect();
                (
                    ToolName(t.tool_id.clone()),
                    ToolMeta {
                        description: t.description.clone(),
                        keywords,
                    },
                )
            })
            .collect();
        TaskBundle {
            schema_version: TASK_BUNDLE_SCHEMA_VERSION,
            intent: spec.intent.clone(),
            language_tags: spec.language_tags.iter().cloned().collect(),
            tool_sequence,
            tool_metadata,
            tolerance_threshold_bp: spec.tolerance_threshold_bp,
        }
    }
}

impl ToolScoutView for HostToolScout {
    fn list_manifests(&self) -> Vec<ToolManifestEntry> {
        self.entries.clone()
    }

    fn score_bundle(&self, spec: &BundleSpecEntry) -> BundleScoreView {
        let bundle = Self::bundle_from_spec(spec);
        // Rank EVERY manifest (k = len), embedder-less (rung-3 neutral).
        let ranked = self
            .index
            .rank(&bundle, None, self.index.len())
            .into_iter()
            .filter_map(|(key, score_bp)| {
                self.index.manifest(&key).map(|fp| ManifestScoreEntry {
                    tool_id: fp.tool_id.0.clone(),
                    tool_version: fp.tool_version.0.clone(),
                    score_bp,
                    fingerprint_hash: *key.as_bytes(),
                })
            })
            .collect();
        // The dry-run verdict: the REAL gate, output DISCARDED (SN-8 — a
        // preview can say "would lower"; only the normal admission/broker path
        // ever executes anything).
        let verdict = match &self.verdict {
            None => LowerVerdictEntry::Unavailable,
            Some(ctx) => {
                match lower_to_workflow_def(&bundle, &ctx.warrant, &ctx.model_id, &ctx.capability) {
                    Ok(_discarded) => LowerVerdictEntry::WouldLower,
                    Err(err) => LowerVerdictEntry::Refused(err.to_string()),
                }
            }
        };
        BundleScoreView {
            bundle_fingerprint: bundle.fingerprint().0,
            ranked,
            verdict,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kx_gateway_core::BundleToolSpecEntry;
    use kx_tool_registry::InMemoryToolRegistry;

    fn host(verdict: Option<VerdictCtx>) -> HostToolScout {
        HostToolScout::new(&InMemoryToolRegistry::with_builtins().defs(), verdict)
    }

    fn spec(tools: &[(&str, &str)]) -> BundleSpecEntry {
        BundleSpecEntry {
            intent: "read a file from disk".to_string(),
            language_tags: vec!["en".to_string()],
            tool_sequence: tools
                .iter()
                .map(|(id, ver)| BundleToolSpecEntry {
                    tool_id: (*id).to_string(),
                    tool_version: (*ver).to_string(),
                    description: String::new(),
                    keywords: vec![],
                })
                .collect(),
            tolerance_threshold_bp: 6_000,
        }
    }

    fn react_ctx(tool: (&str, &str)) -> VerdictCtx {
        let pair = (
            ToolName(tool.0.to_string()),
            ToolVersion(tool.1.to_string()),
        );
        VerdictCtx {
            warrant: crate::provision::react_warrant(
                kx_warrant::ExecutorClass::MacOsSandbox,
                &ModelId("test-model".to_string()),
                &pair,
            ),
            model_id: ModelId("test-model".to_string()),
            capability: pair.0,
        }
    }

    #[test]
    fn manifests_cover_the_builtins_in_deterministic_order() {
        let manifests = host(None).list_manifests();
        let ids: Vec<&str> = manifests.iter().map(|m| m.tool_id.as_str()).collect();
        assert_eq!(ids, vec!["fs-read", "fs-write", "text-summarize"]);
        assert!(manifests.iter().all(|m| m.kind == "Builtin"));
        assert!(manifests.iter().all(|m| !m.description.is_empty()));
        // Each manifest carries the curated, normalized "en" keywords.
        let fs_read = &manifests[0];
        assert_eq!(fs_read.keywords.len(), 1);
        assert_eq!(fs_read.keywords[0].lang, "en");
        assert!(fs_read.keywords[0].words.contains(&"read".to_string()));
    }

    #[test]
    fn an_exact_intent_keyword_scores_the_matching_tool_at_the_ceiling() {
        let view = host(None);
        let score = view.score_bundle(&spec(&[("fs-read", "1")]));
        assert_eq!(score.ranked.len(), 3, "every manifest is ranked");
        let top = &score.ranked[0];
        assert_eq!(top.tool_id, "fs-read");
        assert_eq!(
            top.score_bp,
            kx_toolscout::SCORE_MAX_BP,
            "the intent's 'read'/'file'/'disk' words exact-hit fs-read's curated keywords"
        );
        assert_eq!(score.bundle_fingerprint.len(), 32);
    }

    #[test]
    fn the_same_spec_scores_byte_identically_twice() {
        let view = host(None);
        let a = view.score_bundle(&spec(&[("fs-read", "1")]));
        let b = view.score_bundle(&spec(&[("fs-read", "1")]));
        assert_eq!(a.bundle_fingerprint, b.bundle_fingerprint);
        let pairs = |v: &BundleScoreView| -> Vec<(String, u16)> {
            v.ranked
                .iter()
                .map(|r| (r.tool_id.clone(), r.score_bp))
                .collect()
        };
        assert_eq!(pairs(&a), pairs(&b));
    }

    #[test]
    fn no_react_runtime_means_an_unavailable_verdict() {
        let score = host(None).score_bundle(&spec(&[("fs-read", "1")]));
        assert!(matches!(score.verdict, LowerVerdictEntry::Unavailable));
    }

    #[test]
    fn the_real_gate_passes_a_granted_sequence_and_refuses_an_ungranted_one() {
        // The ctx grants EXACTLY mcp-echo@1 (the server react warrant shape).
        let view = host(Some(react_ctx(("mcp-echo", "1"))));

        let granted = view.score_bundle(&spec(&[("mcp-echo", "1")]));
        assert!(
            matches!(granted.verdict, LowerVerdictEntry::WouldLower),
            "the granted tool lowers (and the WorkflowDef was discarded)"
        );

        let ungranted = view.score_bundle(&spec(&[("fs-read", "1")]));
        match ungranted.verdict {
            LowerVerdictEntry::Refused(detail) => {
                assert!(
                    detail.contains("fs-read"),
                    "the refusal names the ungranted tool: {detail}"
                );
            }
            other => panic!("expected Refused, got {other:?}"),
        }
    }
}
