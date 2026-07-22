// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! [`ModelDescriptor`] — a model's identity, modalities, and cache key.

use std::path::{Path, PathBuf};

use kx_content::ContentRef;
use kx_mote::ModelId;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::errors::ModelStoreError;
use crate::gguf;

/// Domain separator for the `identity_digest` blake3 — versioned so the cache
/// key scheme can evolve without colliding with a future scheme.
const IDENTITY_DOMAIN: &[u8] = b"kx-model-store/identity/v1";

/// A modality a model can consume (and, in future, produce).
///
/// This is a *capability declaration*: the author states what the model accepts
/// so the backend can reject a mismatched input (a text-only model handed an
/// image, or an image-only projector handed audio — the Gemma-3-has-no-audio
/// trap) *before* reaching the FFI. The discriminants are explicit for clarity;
/// `Modality` is off the journal/identity path, so it is not a frozen tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum Modality {
    /// Text tokens (every model).
    Text = 0,
    /// Raster images (vision projector / clip).
    Image = 1,
    /// Audio waveforms (audio encoder / conformer).
    Audio = 2,
    /// Video frames (reserved; no OSS backend serves it yet).
    Video = 3,
}

/// The identity and capabilities of a single registered model.
///
/// Registration is **lazy**: constructing a descriptor does no file I/O and does
/// not require the GGUF to exist (mirrors the backend's existing laziness — the
/// file is only opened on first dispatch). Call [`validate`](Self::validate) to
/// run the fail-closed GGUF-header check explicitly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDescriptor {
    /// Pinned model identity (name + version + quantization).
    pub model_id: ModelId,
    /// Path to the model weights (GGUF).
    pub gguf_path: PathBuf,
    /// Path to the multi-modal projector (`mmproj`), if this is a multi-modal
    /// model. `None` for a text-only model.
    pub mmproj_path: Option<PathBuf>,
    /// Modalities this model accepts. Always contains [`Modality::Text`].
    pub modalities: SmallVec<[Modality; 4]>,
    /// Default context window (`n_ctx`) for this model.
    pub context_window: u32,
    /// Stable cache identity (domain-tagged blake3 of path + modalities). The
    /// loaded-model handle cache keys on this; it is NOT a hash of the weights
    /// and is never journaled. See the crate docs.
    pub identity_digest: ContentRef,
}

impl ModelDescriptor {
    /// Construct a text-only descriptor with the given default context window.
    #[must_use]
    pub fn text(model_id: ModelId, gguf_path: impl Into<PathBuf>, context_window: u32) -> Self {
        let mut modalities = SmallVec::new();
        modalities.push(Modality::Text);
        Self::new(model_id, gguf_path, None, modalities, context_window)
    }

    /// Construct an image (vision) descriptor: declares [`Modality::Text`] +
    /// [`Modality::Image`] and carries the vision projector (`mmproj`). The
    /// convenience constructor the multi-modal IMAGE path (PR-2) registers a
    /// VLM through. Identity folds the Image modality, so the same weights
    /// declared text-only vs. image-capable are distinct cache identities.
    #[must_use]
    pub fn image(
        model_id: ModelId,
        gguf_path: impl Into<PathBuf>,
        mmproj_path: impl Into<PathBuf>,
        context_window: u32,
    ) -> Self {
        let mut modalities = SmallVec::new();
        modalities.push(Modality::Text);
        modalities.push(Modality::Image);
        Self::new(
            model_id,
            gguf_path,
            Some(mmproj_path.into()),
            modalities,
            context_window,
        )
    }

    /// Construct a descriptor with an explicit modality set and optional
    /// projector. [`Modality::Text`] is added if the caller omits it (every
    /// model consumes a textual instruction alongside any media).
    #[must_use]
    pub fn new(
        model_id: ModelId,
        gguf_path: impl Into<PathBuf>,
        mmproj_path: Option<PathBuf>,
        mut modalities: SmallVec<[Modality; 4]>,
        context_window: u32,
    ) -> Self {
        if !modalities.contains(&Modality::Text) {
            modalities.insert(0, Modality::Text);
        }
        let gguf_path = gguf_path.into();
        let identity_digest = compute_identity(&gguf_path, &modalities);
        Self {
            model_id,
            gguf_path,
            mmproj_path,
            modalities,
            context_window,
            identity_digest,
        }
    }

    /// Whether this model accepts inputs of modality `m`.
    #[must_use]
    pub fn supports(&self, m: Modality) -> bool {
        self.modalities.contains(&m)
    }

    /// Whether this is a multi-modal model (declares a projector AND a non-text
    /// modality).
    #[must_use]
    pub fn is_multimodal(&self) -> bool {
        self.mmproj_path.is_some() && self.modalities.iter().any(|m| *m != Modality::Text)
    }

    /// Run the fail-closed GGUF-header validation against the weights file (and
    /// the projector file, if declared). Does real file I/O; call it when you
    /// want to fail fast at registration time rather than at first dispatch.
    ///
    /// # Errors
    ///
    /// Propagates [`ModelStoreError::ModelFileNotReadable`] /
    /// [`ModelStoreError::InvalidGguf`] from [`gguf::validate_gguf_header`].
    pub fn validate(&self) -> Result<(), ModelStoreError> {
        gguf::validate_gguf_header(&self.gguf_path)?;
        if let Some(mmproj) = &self.mmproj_path {
            // The mmproj is itself a GGUF (clip/audio projector weights).
            gguf::validate_gguf_header(mmproj)?;
        }
        Ok(())
    }
}

/// Compute the domain-tagged identity digest from the path + declared modalities.
///
/// Path-based (no weight read) so it is computable lazily and is stable: the same
/// file referenced by two `ModelId`s yields the same digest ⇒ one shared cached
/// load. Modalities are folded in so a re-declaration with different modalities
/// (e.g. adding audio) is a distinct cache identity.
fn compute_identity(gguf_path: &Path, modalities: &[Modality]) -> ContentRef {
    let mut hasher = blake3::Hasher::new();
    hasher.update(IDENTITY_DOMAIN);
    hasher.update(gguf_path.as_os_str().as_encoded_bytes());
    hasher.update(&[0xff]); // delimiter so path/modality bytes can't run together
    for m in modalities {
        hasher.update(&[*m as u8]);
    }
    ContentRef::from_bytes(*hasher.finalize().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mid(s: &str) -> ModelId {
        ModelId(s.to_string())
    }

    #[test]
    fn text_descriptor_supports_only_text() {
        let d = ModelDescriptor::text(mid("llama-3-8b-q4"), "/m/llama.gguf", 4096);
        assert!(d.supports(Modality::Text));
        assert!(!d.supports(Modality::Image));
        assert!(!d.supports(Modality::Audio));
        assert!(!d.is_multimodal());
    }

    #[test]
    fn multimodal_descriptor_adds_text_and_reports_modalities() {
        let mut mods = SmallVec::new();
        mods.push(Modality::Image);
        mods.push(Modality::Audio);
        let d = ModelDescriptor::new(
            mid("gemma-4-q4"),
            "/m/gemma4.gguf",
            Some("/m/gemma4-mmproj.gguf".into()),
            mods,
            8192,
        );
        assert!(d.supports(Modality::Text)); // auto-added
        assert!(d.supports(Modality::Image));
        assert!(d.supports(Modality::Audio));
        assert!(!d.supports(Modality::Video));
        assert!(d.is_multimodal());
    }

    #[test]
    fn image_descriptor_declares_text_image_and_projector() {
        let d = ModelDescriptor::image(
            mid("qwen2-vl-2b-q4"),
            "/m/qwen2vl.gguf",
            "/m/qwen2vl-mmproj.gguf",
            4096,
        );
        assert!(d.supports(Modality::Text));
        assert!(d.supports(Modality::Image));
        assert!(!d.supports(Modality::Audio));
        assert!(d.is_multimodal());
        assert_eq!(
            d.mmproj_path.as_deref(),
            Some(std::path::Path::new("/m/qwen2vl-mmproj.gguf"))
        );
        // An image descriptor over the same weights is a DISTINCT cache identity
        // from a text-only one (modalities fold into the digest).
        let text = ModelDescriptor::text(mid("qwen2-vl-2b-q4"), "/m/qwen2vl.gguf", 4096);
        assert_ne!(d.identity_digest, text.identity_digest);
    }

    #[test]
    fn identity_digest_is_stable_for_same_path_and_modalities() {
        let a = ModelDescriptor::text(mid("a"), "/m/x.gguf", 4096);
        let b = ModelDescriptor::text(mid("b"), "/m/x.gguf", 2048); // different id + n_ctx
        assert_eq!(
            a.identity_digest, b.identity_digest,
            "same file path + modalities ⇒ same cache identity (shared load)"
        );
    }

    #[test]
    fn identity_digest_differs_for_different_path() {
        let a = ModelDescriptor::text(mid("a"), "/m/x.gguf", 4096);
        let b = ModelDescriptor::text(mid("a"), "/m/y.gguf", 4096);
        assert_ne!(a.identity_digest, b.identity_digest);
    }

    #[test]
    fn identity_digest_differs_when_modalities_change() {
        let text = ModelDescriptor::text(mid("a"), "/m/x.gguf", 4096);
        let mut mods = SmallVec::new();
        mods.push(Modality::Image);
        let img = ModelDescriptor::new(mid("a"), "/m/x.gguf", Some("/m/p.gguf".into()), mods, 4096);
        assert_ne!(
            text.identity_digest, img.identity_digest,
            "adding a modality is a distinct cache identity"
        );
    }
}
