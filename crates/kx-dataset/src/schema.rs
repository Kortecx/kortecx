//! [`ContentSchema`] — the typed-ref hook that lets the Morphic engine reason
//! over multi-modal committed content (tensors, vectors, blobs, text, and
//! forward-stubbed image/audio) without parsing the bytes.
//!
//! A [`TypedRef`] pairs a content-addressed [`ContentRef`] with its schema, so
//! transforms and critics can type-check over a corpus. The enum is extensible:
//! a new data type is one variant — the forward seam the data-platform threads
//! (multi-modal, P10) and the Lance backend (P9) build on.

use kx_content::ContentRef;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

/// The element type of a tensor payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TensorDType {
    /// 32-bit IEEE float.
    F32,
    /// 16-bit IEEE float.
    F16,
    /// bfloat16.
    BF16,
    /// 64-bit signed integer.
    I64,
    /// 32-bit signed integer.
    I32,
    /// unsigned byte.
    U8,
    /// boolean (one byte per element).
    Bool,
}

impl TensorDType {
    /// Stable u8 tag for content-addressed hashing. MUST NOT change without a
    /// schema-version bump.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            TensorDType::F32 => 0,
            TensorDType::F16 => 1,
            TensorDType::BF16 => 2,
            TensorDType::I64 => 3,
            TensorDType::I32 => 4,
            TensorDType::U8 => 5,
            TensorDType::Bool => 6,
        }
    }
}

/// The declared type of a content-addressed payload. Opaque to the byte store;
/// the data layer uses it to type-check and to route retrieval.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContentSchema {
    /// Untyped bytes.
    Blob,
    /// UTF-8 text.
    Text,
    /// A JSON document.
    Json,
    /// A dense tensor of `dtype` with the given row-major `shape`.
    Tensor {
        /// Element type.
        dtype: TensorDType,
        /// Dimensions, row-major. Empty = scalar.
        shape: SmallVec<[u64; 4]>,
    },
    /// An embedding vector of `dim` `f32`s (the retrieval/RAG payload).
    Vector {
        /// Dimensionality.
        dim: u32,
    },
    /// An image payload (forward stub for the multi-modal layer, P10).
    Image,
    /// An audio payload (forward stub for the multi-modal layer, P10).
    Audio,
}

impl ContentSchema {
    /// Fold the schema into a hasher canonically (stable tag + parameters).
    /// Used by [`crate::Dataset::id`] so a dataset's identity is a pure function
    /// of its rows.
    pub(crate) fn hash_into(&self, h: &mut blake3::Hasher) {
        match self {
            ContentSchema::Blob => {
                h.update(&[0]);
            }
            ContentSchema::Text => {
                h.update(&[1]);
            }
            ContentSchema::Json => {
                h.update(&[2]);
            }
            ContentSchema::Tensor { dtype, shape } => {
                h.update(&[3, dtype.as_u8()]);
                h.update(&(shape.len() as u64).to_le_bytes());
                for dim in shape {
                    h.update(&dim.to_le_bytes());
                }
            }
            ContentSchema::Vector { dim } => {
                h.update(&[4]);
                h.update(&dim.to_le_bytes());
            }
            ContentSchema::Image => {
                h.update(&[5]);
            }
            ContentSchema::Audio => {
                h.update(&[6]);
            }
        }
    }
}

/// A content-addressed payload tagged with its [`ContentSchema`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedRef {
    /// The content-addressed identity of the payload bytes.
    pub content_ref: ContentRef,
    /// The declared type of those bytes.
    pub schema: ContentSchema,
}
