//! Prompt carriage + synthesis.
//!
//! `MoteDef` carries `model_id` + `inference_params` (identity-bearing) but not
//! the prompt text — in the full runtime the prompt is assembled at context-
//! assembly time (`kx-context-assembler`) from the Mote's parents + template.
//! The harness stands in for that step by carrying the prompt in
//! `config_subset["prompt"]`, which IS folded into `MoteDef::hash` → `MoteId`.
//! So the prompt is identity-bearing **by construction**: same prompt ⇒ same
//! `MoteId` (recipe reuse, row E), different prompt ⇒ different `MoteId` (fresh
//! call). The forward seam is a real `MoteExecutor`/broker that composes an
//! `AssembledContext` and routes through `Dispatcher::dispatch_mote`.

use std::collections::BTreeMap;

use kx_inference::InferenceInput;
use kx_mote::{ConfigKey, ConfigVal, Mote};

/// `config_subset` key under which the harness carries a Mote's prompt text.
///
/// Re-exported from [`kx_mote::PROMPT_KEY`] — the single source of truth shared
/// with the workflow recipe library + the planner (no hand-mirrored copies).
pub use kx_mote::PROMPT_KEY;

/// Insert `prompt` into a `config_subset` map under [`PROMPT_KEY`]. The map is
/// part of `MoteDef`, so the prompt folds into the Mote's identity.
pub fn put_prompt(config_subset: &mut BTreeMap<ConfigKey, ConfigVal>, prompt: &str) {
    config_subset.insert(
        ConfigKey(PROMPT_KEY.to_string()),
        ConfigVal(prompt.as_bytes().to_vec()),
    );
}

/// The raw prompt text carried by `mote`, if any (`None` for a non-model Mote
/// such as a downstream PURE consumer).
#[must_use]
pub fn raw_prompt(mote: &Mote) -> Option<String> {
    mote.def
        .config_subset
        .get(&ConfigKey(PROMPT_KEY.to_string()))
        .map(|v| String::from_utf8_lossy(&v.0).into_owned())
}

/// Qwen2.5-Instruct ChatML wrapping of a user prompt. Deterministic: the same
/// `prompt` always produces the same wrapped string, so a greedy decode is
/// byte-reproducible (row D).
#[must_use]
pub fn chatml(prompt: &str) -> String {
    format!(
        "<|im_start|>system\nYou are a precise assistant. Follow the instruction exactly.<|im_end|>\n<|im_start|>user\n{prompt}<|im_end|>\n<|im_start|>assistant\n"
    )
}

/// Build the [`InferenceInput`] for `mote` — its ChatML-wrapped prompt — or
/// `None` if the Mote carries no prompt (not a model Mote).
#[must_use]
pub fn input_for(mote: &Mote) -> Option<InferenceInput> {
    raw_prompt(mote).map(|p| InferenceInput::text(chatml(&p)))
}
