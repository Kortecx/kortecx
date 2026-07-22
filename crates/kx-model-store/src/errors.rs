// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! Typed, fail-closed errors for the model store.

use std::path::PathBuf;

/// Failure modes surfaced by descriptor validation and the registry.
///
/// Every variant is a *refusal* — there is no partial / best-effort path. A
/// malformed model file, a missing file at validation time, a duplicate
/// registration, or an over-cap registry all fail closed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelStoreError {
    /// The model file could not be opened for validation (does not exist, no
    /// permission, etc). Registration itself is lazy and does NOT require the
    /// file — this only fires from an explicit [`crate::ModelDescriptor::validate`].
    #[error("model file not readable: {path} ({reason})")]
    ModelFileNotReadable {
        /// The path that could not be read.
        path: PathBuf,
        /// OS-level reason (stringified `io::Error` kind).
        reason: String,
    },

    /// The file is not a valid GGUF model — bad magic, an unsupported version,
    /// or an absurd header count (treated as corruption / hostile input).
    #[error("invalid GGUF file {path}: {reason}")]
    InvalidGguf {
        /// The offending model path.
        path: PathBuf,
        /// What specifically failed the bounded header check.
        reason: String,
    },

    /// A descriptor for this `ModelId` is already registered. Re-registration
    /// is refused so identity never silently changes under a live cache.
    #[error("duplicate model registration: {model_id}")]
    DuplicateModel {
        /// The id that was already present.
        model_id: String,
    },

    /// The registry is at its capacity ceiling (a resource-exhaustion guard).
    #[error("model registry full: {cap} descriptors is the ceiling")]
    TooManyModels {
        /// The configured ceiling.
        cap: usize,
    },
}
