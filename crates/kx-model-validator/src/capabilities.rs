//! Capability primitives — small closed enums with stable `#[repr(u8)]`
//! discriminants for forward-compat metadata storage when capabilities land
//! in a registry, plus the [`License`] / [`LicenseConstraint`] pair the
//! validator uses to gate model selection.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// Input/output modalities a model supports.
///
/// Closed enum; new modalities require a coordinated update to the registry
/// schema. The discriminants are stable for forward-compat metadata storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Modality {
    /// Text (always present; the baseline modality).
    Text = 0,
    /// Vision input (images, frames).
    Vision = 1,
    /// Audio input (speech, music).
    Audio = 2,
    /// Generic embedding output (the model emits a fixed-dim vector rather
    /// than tokens).
    Embedding = 3,
}

/// Quantization format the model is loaded in.
///
/// Closed enum for the formats kortecx's OSS inference path (llama.cpp via
/// `kx-llamacpp`) actually loads. Cloud backends with different format
/// support (e.g., vLLM with safetensors) plug their own [`Quantization`]
/// extensions in via the registry's free-form metadata, not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Quantization {
    /// Full 32-bit floats (no quantization).
    F32 = 0,
    /// Half-precision floats.
    F16 = 1,
    /// BFloat16.
    Bf16 = 2,
    /// 8-bit integer (gguf `Q8_0`, common for accuracy-sensitive deployments).
    Q8_0 = 10,
    /// 5-bit, K-quant medium.
    Q5KM = 20,
    /// 4-bit, K-quant medium (the sweet-spot default for many local
    /// deployments).
    Q4KM = 30,
    /// 4-bit, `Q4_0` (gguf legacy/portable).
    Q4_0 = 31,
    /// 2-bit, K-quant (memory-constrained deployments; large quality loss).
    Q2K = 40,
}

/// License under which a model is distributed.
///
/// Tagged as either an SPDX identifier (`SpdxId("Apache-2.0")`) or one of the
/// common open-but-restrictive license patterns. Workflow authors declare
/// constraints via [`LicenseConstraint`]; the check is an exact-match or
/// inclusion lookup, NOT a free-form SPDX expression parser.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum License {
    /// An SPDX identifier (e.g., `"Apache-2.0"`, `"MIT"`).
    SpdxId(String),
    /// Llama-style community license (commercial use permitted with
    /// per-account caps; common for Meta models).
    LlamaCommunity,
    /// Open-weights-non-commercial (research use only; e.g., some early
    /// Llama variants).
    OpenWeightsNonCommercial,
    /// Proprietary / closed (e.g., GPT-class API access; the runtime can
    /// call but not redistribute weights).
    Proprietary,
    /// License is unknown to the registry. Treated as restrictive by default
    /// (matches no constraint that requires a specific license).
    Unknown,
}

/// Workflow-author constraint on the model's license.
///
/// Constraints compose by intersection: declaring `RequireCommercialOk` AND
/// `RequireRedistributable` requires a license satisfying both.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum LicenseConstraint {
    /// No license restriction (any model permitted).
    NoRestriction,
    /// The license must explicitly allow commercial use.
    RequireCommercialOk,
    /// The license must allow weight redistribution (e.g., shipping the
    /// model file in a container image).
    RequireRedistributable,
    /// The model's license must be one of an explicit set (e.g.,
    /// `{"Apache-2.0", "MIT"}`).
    OneOf(BTreeSet<License>),
}

impl LicenseConstraint {
    /// `true` when `license` satisfies the constraint.
    #[must_use]
    pub fn is_satisfied_by(&self, license: &License) -> bool {
        match self {
            Self::NoRestriction => true,
            Self::RequireCommercialOk => commercial_use_permitted(license),
            Self::RequireRedistributable => redistribution_permitted(license),
            Self::OneOf(allowed) => allowed.contains(license),
        }
    }
}

/// Heuristic: does this license permit commercial use?
///
/// Conservative — only returns `true` for licenses we KNOW permit commercial
/// use. Unknown licenses are treated as restrictive.
fn commercial_use_permitted(license: &License) -> bool {
    match license {
        License::SpdxId(id) => matches!(
            id.as_str(),
            "Apache-2.0" | "MIT" | "BSD-2-Clause" | "BSD-3-Clause" | "ISC"
        ),
        License::LlamaCommunity => true, // commercial use permitted with caveats
        License::Proprietary => true,    // commercial API access IS commercial use
        License::OpenWeightsNonCommercial | License::Unknown => false,
    }
}

/// Heuristic: does this license permit redistribution of the model weights?
fn redistribution_permitted(license: &License) -> bool {
    match license {
        License::SpdxId(id) => matches!(
            id.as_str(),
            "Apache-2.0" | "MIT" | "BSD-2-Clause" | "BSD-3-Clause" | "ISC"
        ),
        License::LlamaCommunity => true, // permits redistribution under the community license
        License::Proprietary | License::OpenWeightsNonCommercial | License::Unknown => false,
    }
}
