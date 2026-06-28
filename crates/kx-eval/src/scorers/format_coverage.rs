//! Cross-format tool-call parse coverage — the committed "before" baseline.
//!
//! Different OSS models emit tool calls in different shapes (JSON envelope, Gemma
//! native-brace, Gemma paren / `T-GEMMA-PAREN`, Llama python-tag, Qwen/Hermes XML,
//! markerless). This scorer runs the runtime's REAL `kx_toolcall::parse_tool_call` over
//! a corpus of raw per-format model strings under a fixed grant set and measures the
//! fraction it decodes as intended. It is the single number that quantifies today's
//! parse fragility — RC2 (grammar-constrained decoding) raises it, and RC1 commits this
//! "before" value so the improvement is provable.
//!
//! Pure + deterministic: `parse_tool_call` is total over arbitrary bytes, so this scorer
//! cannot flake.

use std::collections::BTreeSet;

use kx_mote::{ModelId, ToolName, ToolVersion};
use kx_toolcall::{max_args_bytes, parse_tool_call};
use kx_warrant::{ModelRoute, ToolGrant, WarrantSpec};
use serde::{Deserialize, Serialize};

use crate::scorers::{ScoreOutput, PER_MILLE};
use crate::suite::ExpectedToolCall;

/// What a raw per-format model string should decode to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum FormatExpectation {
    /// The string should decode to this granted tool call.
    Call {
        /// The expected decoded tool id.
        tool_id: String,
        /// The expected decoded tool version.
        tool_version: String,
    },
    /// The string is a normal completion (`Ok(None)`) — no tool call.
    NoCall,
    /// The string should be REFUSED (a `DecodeError` — e.g. an ungranted/ambiguous name).
    Refused,
}

/// One raw per-format model string and what it should decode to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatCase {
    /// A label for the emitting shape (e.g. `"gemma_paren"`) — for the human report.
    pub format: String,
    /// The raw model output bytes (as UTF-8).
    pub raw: String,
    /// The intended decode.
    pub expect: FormatExpectation,
}

/// Build the fixed grant context the parse cases run under (a minimal warrant granting
/// exactly the listed tools, with enough `max_output_tokens` headroom that realistic
/// args are not size-refused). Mirrors `kx-toolcall`'s own test warrant.
#[must_use]
pub(crate) fn warrant_for_grants(grants: &[ExpectedToolCall]) -> WarrantSpec {
    let mut tool_grants = BTreeSet::new();
    for g in grants {
        tool_grants.insert(ToolGrant {
            tool_id: ToolName(g.tool_id.clone()),
            tool_version: ToolVersion(g.tool_version.clone()),
        });
    }
    WarrantSpec {
        tool_grants,
        model_route: ModelRoute {
            model_id: ModelId("eval".into()),
            max_input_tokens: 1024,
            max_output_tokens: 256,
            max_calls: 8,
        },
        ..Default::default()
    }
}

/// Score the parse coverage of `cases` under the grant set `grants`. Returns a Gate
/// per-mille = fraction of cases that decoded as intended.
#[must_use]
pub fn score_format_coverage(grants: &[ExpectedToolCall], cases: &[FormatCase]) -> ScoreOutput {
    if cases.is_empty() {
        return ScoreOutput::gate("format_coverage", PER_MILLE, "no format cases");
    }
    let warrant = warrant_for_grants(grants);
    let cap = max_args_bytes(&warrant);

    let mut correct = 0usize;
    for c in cases {
        let got = parse_tool_call(c.raw.as_bytes(), &warrant, cap);
        let ok = match (&c.expect, &got) {
            (
                FormatExpectation::Call {
                    tool_id,
                    tool_version,
                },
                Ok(Some(tc)),
            ) => tc.name.0 == *tool_id && tc.version.0 == *tool_version,
            (FormatExpectation::NoCall, Ok(None)) | (FormatExpectation::Refused, Err(_)) => true,
            _ => false,
        };
        if ok {
            correct += 1;
        }
    }
    let per_mille = u32::try_from(correct * PER_MILLE as usize / cases.len()).unwrap_or(0);
    ScoreOutput::gate(
        "format_coverage",
        per_mille,
        format!("{correct}/{} format cases decoded as intended", cases.len()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant(id: &str, ver: &str) -> ExpectedToolCall {
        ExpectedToolCall {
            tool_id: id.into(),
            tool_version: ver.into(),
        }
    }

    #[test]
    fn json_envelope_decodes() {
        let grants = vec![grant("mcp-echo", "1")];
        let cases = vec![FormatCase {
            format: "json_envelope".into(),
            raw: r#"{"tool_call":{"name":"mcp-echo","version":"1","args":{"text":"hi"}}}"#.into(),
            expect: FormatExpectation::Call {
                tool_id: "mcp-echo".into(),
                tool_version: "1".into(),
            },
        }];
        let s = score_format_coverage(&grants, &cases);
        assert_eq!(s.gate_per_mille(), Some(PER_MILLE));
    }

    #[test]
    fn plain_prose_is_no_call() {
        let grants = vec![grant("mcp-echo", "1")];
        let cases = vec![FormatCase {
            format: "prose".into(),
            raw: "The answer is 42.".into(),
            expect: FormatExpectation::NoCall,
        }];
        assert_eq!(
            score_format_coverage(&grants, &cases).gate_per_mille(),
            Some(PER_MILLE)
        );
    }

    #[test]
    fn mixed_cases_average() {
        // one correct, one mismatched ⇒ 500 per-mille.
        let grants = vec![grant("mcp-echo", "1")];
        let cases = vec![
            FormatCase {
                format: "json_envelope".into(),
                raw: r#"{"tool_call":{"name":"mcp-echo","version":"1","args":{"text":"hi"}}}"#
                    .into(),
                expect: FormatExpectation::Call {
                    tool_id: "mcp-echo".into(),
                    tool_version: "1".into(),
                },
            },
            FormatCase {
                // expects a call but the string is prose ⇒ counts wrong
                format: "prose".into(),
                raw: "no tool here".into(),
                expect: FormatExpectation::Call {
                    tool_id: "mcp-echo".into(),
                    tool_version: "1".into(),
                },
            },
        ];
        assert_eq!(
            score_format_coverage(&grants, &cases).gate_per_mille(),
            Some(500)
        );
    }
}
