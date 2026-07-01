//! [`GrammarSpec`] — the tagged constraint carrier the off-digest
//! `kx_mote::Grammar.raw` holds.
//!
//! RC2 carried a bare [`ToolEnvelopeSpec`] (tool-call constraint). RC4c adds a
//! second constrained-output use — the listwise-rerank [`PermutationSpec`] — so the
//! carrier becomes a tagged enum. `kx_inference::cache` arms a [`Self::ToolEnvelope`]
//! as a LAZY GBNF (triggered on the `{"tool_call"` opener, so prose answers flow
//! free); a [`Self::Permutation`] is NOT GBNF-constrained on llama.cpp (its sampler
//! crashes on a digit-array constraint — see [`PermutationSpec`]) and instead relies
//! on the fail-closed parser. The Ollama backend renders the permutation as a strict
//! whole-response `format`.
//!
//! ## Back-compatibility (load-bearing)
//! `#[serde(untagged)]`: an EXISTING RC2 carrier raw (`{"tools":[…]}`) still decodes
//! as [`Self::ToolEnvelope`]. The two variants have DISJOINT required keys —
//! `ToolEnvelopeSpec` requires `tools`, `PermutationSpec` requires `n` — so untagged
//! deserialization is unambiguous and order-independent. The carrier is injected
//! fresh per dispatch and NEVER persisted (off-digest, D108.2), so there is no
//! on-disk migration; a malformed carrier still fails the dispatch CLOSED.

use serde::{Deserialize, Serialize};

use crate::error::GrammarError;
use crate::permutation::PermutationSpec;
use crate::spec::ToolEnvelopeSpec;

/// The constrained-generation carrier: EITHER a tool-call envelope constraint (RC2)
/// OR a listwise-rerank permutation constraint (RC4c).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GrammarSpec {
    /// Constrain a tool-eligible `ReAct` turn to a grant-pinned tool-call envelope.
    ToolEnvelope(ToolEnvelopeSpec),
    /// Constrain a rerank turn to a permutation array over the retrieved candidates.
    Permutation(PermutationSpec),
}

impl GrammarSpec {
    /// Serialize into the opaque `kx_mote::Grammar.raw` carrier (canonical JSON).
    ///
    /// # Errors
    /// [`GrammarError::Malformed`] only if serialization fails (not reachable for
    /// the closed spec types, but surfaced rather than panicking).
    pub fn to_raw(&self) -> Result<String, GrammarError> {
        serde_json::to_string(self).map_err(|e| GrammarError::Malformed {
            diagnostic: e.to_string(),
        })
    }

    /// Recover a carrier from the opaque `kx_mote::Grammar.raw`. An existing RC2
    /// `{"tools":[…]}` raw decodes as [`Self::ToolEnvelope`] (back-compat).
    ///
    /// # Errors
    /// [`GrammarError::Malformed`] if `raw` is not a serialized carrier — the engine
    /// leg MUST fail the dispatch CLOSED on this (never silently unconstrain).
    pub fn from_raw(raw: &str) -> Result<Self, GrammarError> {
        serde_json::from_str(raw).map_err(|e| GrammarError::Malformed {
            diagnostic: e.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::ToolSpec;

    #[test]
    fn existing_tool_envelope_raw_still_decodes_as_tool_envelope() {
        // An RC2 carrier raw (what `build_tool_grammar` writes today).
        let raw = ToolEnvelopeSpec::new(vec![ToolSpec::new("mcp-echo/echo", "1")])
            .to_raw()
            .unwrap();
        assert!(raw.contains("\"tools\""));
        match GrammarSpec::from_raw(&raw).unwrap() {
            GrammarSpec::ToolEnvelope(spec) => {
                assert_eq!(spec.tools.len(), 1);
                assert_eq!(spec.tools[0].name, "mcp-echo/echo");
            }
            GrammarSpec::Permutation(_) => {
                panic!("a tool-envelope raw must decode as ToolEnvelope")
            }
        }
    }

    #[test]
    fn strict_defaults_false_skip_serialized_and_round_trips() {
        // RC4c-2c: the DEFAULT (non-strict) carrier is BYTE-IDENTICAL to pre-RC4c-2c —
        // `strict:false` is skip-serialized, so no `"strict"` key appears.
        let raw = ToolEnvelopeSpec::new(vec![ToolSpec::new("retrieve", "1")])
            .to_raw()
            .unwrap();
        assert!(
            !raw.contains("strict"),
            "default carrier must omit strict: {raw}"
        );
        // An old `{"tools":[…]}` carrier decodes with strict = false (back-compat).
        match GrammarSpec::from_raw(&raw).unwrap() {
            GrammarSpec::ToolEnvelope(spec) => assert!(!spec.strict),
            GrammarSpec::Permutation(_) => panic!("must decode as ToolEnvelope"),
        }
        // The OPT-IN strict carrier serializes + round-trips with strict = true.
        let strict_raw = ToolEnvelopeSpec::new(vec![ToolSpec::new("retrieve", "1")])
            .with_strict(true)
            .to_raw()
            .unwrap();
        assert!(
            strict_raw.contains("\"strict\":true"),
            "strict carrier must carry it"
        );
        match GrammarSpec::from_raw(&strict_raw).unwrap() {
            GrammarSpec::ToolEnvelope(spec) => assert!(spec.strict),
            GrammarSpec::Permutation(_) => panic!("must decode as ToolEnvelope"),
        }
    }

    #[test]
    fn permutation_raw_decodes_as_permutation() {
        let raw = GrammarSpec::Permutation(PermutationSpec::new(8))
            .to_raw()
            .unwrap();
        assert!(raw.contains("\"n\""));
        match GrammarSpec::from_raw(&raw).unwrap() {
            GrammarSpec::Permutation(p) => assert_eq!(p.n, 8),
            GrammarSpec::ToolEnvelope(_) => panic!("a permutation raw must decode as Permutation"),
        }
    }

    #[test]
    fn disjoint_keys_make_untagged_unambiguous() {
        // `{"n":N}` has no `tools` key ⇒ never matches ToolEnvelope.
        assert!(matches!(
            GrammarSpec::from_raw(r#"{"n":3}"#).unwrap(),
            GrammarSpec::Permutation(PermutationSpec { n: 3 })
        ));
        // `{"tools":[]}` has no `n` key ⇒ matches ToolEnvelope (empty grant set).
        assert!(matches!(
            GrammarSpec::from_raw(r#"{"tools":[]}"#).unwrap(),
            GrammarSpec::ToolEnvelope(_)
        ));
    }

    #[test]
    fn a_garbage_carrier_fails_closed() {
        assert!(GrammarSpec::from_raw("not json").is_err());
        // An object matching NEITHER variant's required key is malformed.
        assert!(GrammarSpec::from_raw(r#"{"foo":1}"#).is_err());
    }

    #[test]
    fn round_trips_both_variants() {
        for spec in [
            GrammarSpec::Permutation(PermutationSpec::new(16)),
            GrammarSpec::ToolEnvelope(ToolEnvelopeSpec::new(vec![ToolSpec::new("retrieve", "1")])),
        ] {
            let raw = spec.to_raw().unwrap();
            assert_eq!(GrammarSpec::from_raw(&raw).unwrap(), spec);
        }
    }
}
