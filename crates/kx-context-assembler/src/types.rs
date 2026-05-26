//! [`AssembledItem`] + [`AssembledContext`] — the value types [`crate::assemble`]
//! produces. The model reasons over `item.bytes`; everything else is
//! orchestration-side bookkeeping.

use bytes::Bytes;
use kx_content::ContentRef;
use serde::{Deserialize, Serialize};

/// A single resolved item in the assembled context. The model sees `bytes`
/// (raw content); `source_ref` and `label` are bookkeeping for replay
/// reproducibility and operator inspection respectively.
///
/// # Example
///
/// ```
/// use kx_context_assembler::AssembledItem;
/// use bytes::Bytes;
/// use kx_content::ContentRef;
///
/// let item = AssembledItem {
///     label: "parent.abc123".into(),
///     bytes: Bytes::from_static(b"resolved content"),
///     source_ref: ContentRef::of(b"resolved content"),
/// };
/// // The model reasons over `item.bytes` (NEVER a hash); `source_ref` and
/// // `label` are orchestration-side bookkeeping.
/// assert_eq!(&item.bytes[..], b"resolved content");
/// assert_eq!(item.source_ref, ContentRef::of(b"resolved content"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssembledItem {
    /// Human-readable label for this item: `"parent.<hex>"` or
    /// `"tool.<name>@<version>"`. NEVER parsed by the model; for operator
    /// inspection only.
    pub label: String,
    /// The resolved bytes. The model reasons over these. **NEVER a hash.**
    pub bytes: Bytes,
    /// The `ContentRef` the bytes came from. Carried for replay reproducibility
    /// (so the executor can journal a `ToolResolutionEvent`-shaped fact if
    /// needed). Not fed into the model.
    pub source_ref: ContentRef,
}

/// The full assembled context, in deterministic order (Data-edge parents first
/// by `MoteId` bytes; then tools by `(tool_id, tool_version)`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AssembledContext {
    /// Items in deterministic emission order.
    pub items: Vec<AssembledItem>,
}

impl AssembledContext {
    /// Total bytes across all items. Used by [`crate::assemble`] for the overflow
    /// check; exposed publicly for the executor's diagnostics.
    ///
    /// # Example
    ///
    /// ```
    /// use kx_context_assembler::{AssembledContext, AssembledItem};
    /// use bytes::Bytes;
    /// use kx_content::ContentRef;
    ///
    /// let ctx = AssembledContext { items: vec![
    ///     AssembledItem {
    ///         label: "a".into(),
    ///         bytes: Bytes::from_static(b"hello"),
    ///         source_ref: ContentRef::from_bytes([0; 32]),
    ///     },
    ///     AssembledItem {
    ///         label: "b".into(),
    ///         bytes: Bytes::from_static(b"world!"),
    ///         source_ref: ContentRef::from_bytes([1; 32]),
    ///     },
    /// ]};
    /// assert_eq!(ctx.total_bytes(), 11);
    /// ```
    #[must_use]
    pub fn total_bytes(&self) -> usize {
        self.items.iter().map(|i| i.bytes.len()).sum()
    }

    /// `true` iff there are no items.
    ///
    /// # Example
    ///
    /// ```
    /// use kx_context_assembler::AssembledContext;
    /// let ctx: AssembledContext = Default::default();
    /// assert!(ctx.is_empty());
    /// ```
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Number of items in the context.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Compute a content-addressed `ContentRef` over the assembled bytes
    /// in emission order. Useful as a cache key for cross-Mote context reuse
    /// (per D33 §2.5).
    ///
    /// `assembled_ref = blake3(concat_in_order(item.bytes))`. Note this hashes
    /// only the resolved bytes (not the labels or source_refs) so two contexts
    /// with the same content but different labels resolve to the same ref.
    ///
    /// # Example
    ///
    /// ```
    /// use kx_context_assembler::{AssembledContext, AssembledItem};
    /// use bytes::Bytes;
    /// use kx_content::ContentRef;
    ///
    /// let ctx = AssembledContext { items: vec![
    ///     AssembledItem {
    ///         label: "x".into(),
    ///         bytes: Bytes::from_static(b"deterministic"),
    ///         source_ref: ContentRef::from_bytes([0; 32]),
    ///     },
    /// ]};
    /// // Same bytes → same content_ref (idempotent).
    /// assert_eq!(ctx.content_ref(), ctx.content_ref());
    /// ```
    #[must_use]
    pub fn content_ref(&self) -> ContentRef {
        let mut hasher = blake3::Hasher::new();
        for item in &self.items {
            hasher.update(&item.bytes);
        }
        ContentRef::from_bytes(*hasher.finalize().as_bytes())
    }
}
