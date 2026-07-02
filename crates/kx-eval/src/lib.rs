//! kortecx **agentic-evaluation harness** (RC1, D172) — the measure-first yardstick.
//!
//! The OSS RC reframes the runtime toward *agentic excellence on local OSS models*
//! (D172). "Outcome-driven" requires the metric **before** any tuning, so this crate
//! lands FIRST: it scores how well an agent run actually performed and becomes the
//! regression ratchet (`just eval >= baseline`) every later RC PR must hold — grammar
//! (RC2), prompts/context (RC3), RAG (RC4), durable memory (RC5) are each scored here
//! and gated against silent regression.
//!
//! ## The one idea: every scorer is a pure function of a [`Transcript`]
//! A [`Transcript`] is the engine-agnostic, model-agnostic record of one run — its
//! ordered turns (each a [`Branch`]), the committed final answer bytes, and the ordered
//! retrieved RAG docs. Where the transcript *comes from* is the only thing that differs
//! between the two evaluation tiers:
//! - **Tier A (deterministic, CI-required):** the transcript is a fixed scripted
//!   fixture from the committed corpus — no model, no gateway, no clock. The scorers
//!   are byte-deterministic, so the gate cannot flake.
//! - **Tier B (advisory, real-model):** the transcript is built from a LIVE run on
//!   Gemma-4 / Qwen3 (mapped from the gateway's `ListReactTurns` + committed content by
//!   the caller). Its numbers are recorded as Spike metrics for the private trend, never
//!   a hard CI assertion.
//!
//! ## A Gate verdict is an integer, never a float
//! Each Gate scorer returns an integer **per-mille** score in `0..=1000`
//! ([`ScoreValue::Gate`]). A pass/fail decision is therefore an exact integer
//! comparison — no float on the decision path (SN-8; mirrors `kx-critic`'s no-float
//! discipline). Only [`ScoreValue::Spike`] (absolute latency) carries an `f64`, and a
//! Spike is *recorded*, never *gated*.
//!
//! ## Invariants
//! - **Off the journal + identity path + frozen trio** — eval is a measurement leaf
//!   that *reads* committed facts and scores them; it never writes a fact, never feeds
//!   the canonical projection digest `7d22d4bd`, and the frozen trio never depends on
//!   it. FFI-free (no `kx-llamacpp`/`kx-inference` edge ever).
//! - **The gating baseline is a COMMITTED corpus** (`corpus/golden-v1/baseline.json`),
//!   not the gitignored `docs/benchmarks/` (SN-2). RC1 commits it capturing the current
//!   (`T-GEMMA-PAREN`) parse coverage as the "before"; RC2 raises it in-PR.
//! - **The corpus is content-addressed** — every [`EvalReport`] records the
//!   `suite_digest`, so a corpus change is visible and forces a deliberate re-baseline.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown,
    clippy::return_self_not_must_use
)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod corpus;
mod error;
mod report;
mod run_quality;
mod runner;
mod scorers;
mod suite;
mod transcript;

pub use crate::corpus::{
    embedded_baseline, load_golden_v1, FormatCorpus, GoldenCorpus, GOLDEN_V1_ID,
};
pub use crate::error::EvalError;
pub use crate::report::{
    aggregate, compare_to_baseline, Baseline, BaselineComparison, EvalReport, GateValue,
    Regression, SpikeMetric, TaskScore, GATE_UNIT, SCHEMA_VERSION,
};
pub use crate::run_quality::{analyze_run, RunQuality};
pub use crate::runner::{score_corpus, score_golden_v1, score_golden_v1_family};
pub use crate::scorers::{
    score_format_coverage, score_transcript, FormatCase, FormatExpectation, ScoreInput,
    ScoreOutput, ScoreValue, TRANSCRIPT_SCORER_IDS,
};
pub use crate::suite::{Expectation, ExpectedTerminal, ExpectedToolCall, GoldenSuite, GoldenTask};
pub use crate::transcript::{Branch, ToolKey, Transcript, TurnRecord};
