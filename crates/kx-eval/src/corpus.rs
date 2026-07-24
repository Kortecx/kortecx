//! Loading + content-addressing the versioned golden corpus.
//!
//! The `golden-v1` corpus is **embedded at compile time** (`include_str!`) so the gate
//! resolves it from any working directory and the binary carries the exact bytes it
//! scores. Its `suite_digest` is a blake3 over those bytes — a corpus change shifts the
//! digest, and [`crate::compare_to_baseline`] fails closed until the baseline is
//! deliberately re-captured (the measurement contract changed).

use serde::Deserialize;

use crate::error::EvalError;
use crate::report::Baseline;
use crate::scorers::FormatCase;
use crate::suite::{ExpectedToolCall, GoldenSuite};

/// The id of the v1 golden suite.
pub const GOLDEN_V1_ID: &str = "golden-v1";

const SUITE_JSON: &str = include_str!("../corpus/golden-v1/suite.json");
const FORMAT_JSON: &str = include_str!("../corpus/golden-v1/format_cases.json");
const BASELINE_JSON: &str = include_str!("../corpus/golden-v1/baseline.json");

/// The id of the v1 real-model benchmark suite.
pub const BENCH_V1_ID: &str = "bench-v1";

const BENCH_V1_SUITE_JSON: &str = include_str!("../corpus/bench-v1/suite.json");

/// The cross-format parse corpus: the grant context the cases run under + the per-format
/// raw model strings and their intended decodes.
#[derive(Debug, Clone, Deserialize)]
pub struct FormatCorpus {
    /// The tools granted while parsing the cases (the cases name these).
    pub grants: Vec<ExpectedToolCall>,
    /// The per-format cases.
    pub cases: Vec<FormatCase>,
}

/// The loaded, content-addressed golden corpus.
#[derive(Debug, Clone)]
pub struct GoldenCorpus {
    /// The golden task suite (each task carries a scripted Tier-A transcript).
    pub suite: GoldenSuite,
    /// The cross-format parse corpus.
    pub format: FormatCorpus,
    /// The content digest (hex blake3 over the embedded corpus bytes).
    pub suite_digest: String,
}

/// Load + parse the embedded `golden-v1` corpus and compute its content digest.
///
/// # Errors
/// [`EvalError::Malformed`] if either embedded corpus file is not well-formed JSON.
pub fn load_golden_v1() -> Result<GoldenCorpus, EvalError> {
    let suite: GoldenSuite =
        serde_json::from_str(SUITE_JSON).map_err(|e| EvalError::Malformed {
            what: "golden suite",
            detail: e.to_string(),
        })?;
    let format: FormatCorpus =
        serde_json::from_str(FORMAT_JSON).map_err(|e| EvalError::Malformed {
            what: "format cases",
            detail: e.to_string(),
        })?;
    Ok(GoldenCorpus {
        suite,
        format,
        suite_digest: digest_hex(&[SUITE_JSON.as_bytes(), FORMAT_JSON.as_bytes()]),
    })
}

/// The loaded, content-addressed `bench-v1` real-model benchmark suite.
///
/// Unlike [`GoldenCorpus`], a bench suite carries no format-parse corpus: it is scored
/// only from LIVE served-model transcripts (each task is real-only — `instruction` +
/// `expect`, no scripted Tier-A fixture), so there is no deterministic format matrix to
/// fold. The `suite_digest` is a blake3 over the embedded suite bytes, so a task change
/// shifts the digest and [`crate::compare_to_baseline`] fails closed until the per-engine
/// baseline is deliberately re-captured — the same drift discipline as `golden-v1`.
#[derive(Debug, Clone)]
pub struct BenchCorpus {
    /// The real-model task suite (each task carries an `instruction` + `expect`, no
    /// scripted transcript).
    pub suite: GoldenSuite,
    /// The content digest (hex blake3 over the embedded suite bytes).
    pub suite_digest: String,
}

/// Load + parse the embedded `bench-v1` real-model benchmark suite and compute its content
/// digest. The suite is driven live by the gateway benchmark harness (the only layer with
/// a served model + a client); this crate stays proto-free and only owns the corpus + the
/// scorers it is fed.
///
/// # Errors
/// [`EvalError::Malformed`] if the embedded suite file is not well-formed JSON.
pub fn load_bench_v1() -> Result<BenchCorpus, EvalError> {
    let suite: GoldenSuite =
        serde_json::from_str(BENCH_V1_SUITE_JSON).map_err(|e| EvalError::Malformed {
            what: "bench-v1 suite",
            detail: e.to_string(),
        })?;
    Ok(BenchCorpus {
        suite,
        suite_digest: digest_hex(&[BENCH_V1_SUITE_JSON.as_bytes()]),
    })
}

/// The committed `golden-v1` baseline (embedded), the gate's default yardstick. Embedded
/// so the gate runs from an INSTALLED binary, not just the source tree.
///
/// # Errors
/// [`EvalError::Malformed`] if the embedded baseline is not well-formed JSON.
pub fn embedded_baseline() -> Result<Baseline, EvalError> {
    serde_json::from_str(BASELINE_JSON).map_err(|e| EvalError::Malformed {
        what: "baseline",
        detail: e.to_string(),
    })
}

/// blake3 over the concatenated blobs, as lowercase hex.
fn digest_hex(blobs: &[&[u8]]) -> String {
    let mut hasher = blake3::Hasher::new();
    for b in blobs {
        hasher.update(b);
    }
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_v1_loads_and_is_addressed() {
        let c = load_golden_v1().expect("golden-v1 corpus parses");
        assert_eq!(c.suite.id, GOLDEN_V1_ID);
        assert!(!c.suite.tasks.is_empty(), "suite has tasks");
        assert!(!c.format.cases.is_empty(), "format matrix has cases");
        assert_eq!(c.suite_digest.len(), 64, "blake3 hex is 64 chars");
        // The digest is a pure function of the embedded bytes ⇒ stable across calls.
        let again = load_golden_v1().expect("reload");
        assert_eq!(c.suite_digest, again.suite_digest);
    }

    #[test]
    fn bench_v1_loads_addressed_and_is_real_only() {
        let c = load_bench_v1().expect("bench-v1 suite parses");
        assert_eq!(c.suite.id, BENCH_V1_ID);
        assert!(!c.suite.tasks.is_empty(), "bench suite has tasks");
        assert_eq!(c.suite_digest.len(), 64, "blake3 hex is 64 chars");
        // Every bench task is REAL-ONLY: no scripted Tier-A fixture (scored live only),
        // and every task carries a non-empty instruction to send a served model.
        for t in &c.suite.tasks {
            assert!(
                t.scripted_transcript.is_none(),
                "bench task {} must be real-only (no scripted transcript)",
                t.id
            );
            assert!(
                !t.instruction.is_empty(),
                "bench task {} has an instruction",
                t.id
            );
        }
        // Content-addressed ⇒ stable, and distinct from golden-v1's corpus digest.
        let again = load_bench_v1().expect("reload");
        assert_eq!(c.suite_digest, again.suite_digest);
        assert_ne!(
            c.suite_digest,
            load_golden_v1().expect("golden-v1").suite_digest,
            "bench-v1 is a distinct corpus"
        );
    }
}
