//! Hash newtypes: [`crate::MoteId`], [`crate::MoteDefHash`], [`crate::InputDataId`], [`crate::LogicRef`],
//! [`crate::PromptTemplateHash`]. All five wrap a 32-byte BLAKE3 hash and carry
//! identical Debug/Display impls (hex). The `Hash32` alias is a crate-local
//! convenience.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Hash newtypes
// ---------------------------------------------------------------------------

/// A 32-byte BLAKE3 hash. Common substrate for all hash newtypes in the crate.
type Hash32 = [u8; 32];

/// The stable 32-byte identity of a Mote (see crate-level docs).
///
/// Derived purely from the workflow definition, the committed inputs the Mote
/// consumes, and its position in the DAG — never from clock, host, PID, or
/// attempt number. Two workers attempting the same logical work derive the
/// same `MoteId`; the journal dedupes them to one committed fact.
///
/// # Examples
///
/// ```
/// use kx_mote::MoteId;
///
/// let a = MoteId::from_bytes([0xaa; 32]);
/// let b = MoteId::from_bytes([0xaa; 32]);
/// assert_eq!(a, b, "MoteId equality is by-bytes");
/// assert_eq!(a.as_bytes(), &[0xaa; 32]);
///
/// // Display + Debug both render the 64-char lowercase hex form.
/// let hex = format!("{}", a);
/// assert_eq!(hex.len(), 64);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct MoteId(pub Hash32);

impl MoteId {
    /// Construct a `MoteId` from raw 32 bytes.
    #[inline]
    #[must_use]
    pub const fn from_bytes(bytes: Hash32) -> Self {
        Self(bytes)
    }

    /// Borrow the underlying 32 bytes.
    #[inline]
    #[must_use]
    pub const fn as_bytes(&self) -> &Hash32 {
        &self.0
    }
}

impl std::fmt::Debug for MoteId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MoteId({})", blake3::Hash::from_bytes(self.0).to_hex())
    }
}

impl std::fmt::Display for MoteId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", blake3::Hash::from_bytes(self.0).to_hex())
    }
}

/// The 32-byte canonical hash of a [`crate::MoteDef`].
///
/// Computed by [`crate::MoteDef::hash`]: serialize `MoteDef` with [`crate::canonical_config`]
/// (a frozen bincode configuration), then BLAKE3 the resulting bytes. Identifies
/// a Mote's *kind of work* in the journal; the poison-cascade (definition-level
/// repudiation) queries committed entries by this hash.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct MoteDefHash(pub Hash32);

impl MoteDefHash {
    /// Construct a `MoteDefHash` from raw 32 bytes.
    #[inline]
    #[must_use]
    pub const fn from_bytes(bytes: Hash32) -> Self {
        Self(bytes)
    }

    /// Borrow the underlying 32 bytes.
    #[inline]
    #[must_use]
    pub const fn as_bytes(&self) -> &Hash32 {
        &self.0
    }
}

impl std::fmt::Debug for MoteDefHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "MoteDefHash({})",
            blake3::Hash::from_bytes(self.0).to_hex()
        )
    }
}

impl std::fmt::Display for MoteDefHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", blake3::Hash::from_bytes(self.0).to_hex())
    }
}

/// The 32-byte identity of the actual upstream inputs a Mote consumes.
///
/// Derived from the `result_ref` content hashes of the Mote's committed parents
/// (executor-owned derivation in P1.9). For zero-parent (entrypoint) Motes, it
/// is the BLAKE3 of a per-run workflow-input seed; this crate stores the
/// pre-computed bytes and never invents the derivation.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct InputDataId(pub Hash32);

impl InputDataId {
    /// Construct an `InputDataId` from raw 32 bytes.
    #[inline]
    #[must_use]
    pub const fn from_bytes(bytes: Hash32) -> Self {
        Self(bytes)
    }

    /// Borrow the underlying 32 bytes.
    #[inline]
    #[must_use]
    pub const fn as_bytes(&self) -> &Hash32 {
        &self.0
    }
}

impl std::fmt::Debug for InputDataId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "InputDataId({})",
            blake3::Hash::from_bytes(self.0).to_hex()
        )
    }
}

/// The 32-byte hash of the compiled artifact backing a Mote's logic (its `logic_ref`).
///
/// The reproducible-build discipline (workspace-level, P1.1) ensures this hash is
/// stable across machines and CI runs. Component of [`crate::MoteDef::logic_ref`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LogicRef(pub Hash32);

impl LogicRef {
    /// Construct a `LogicRef` from raw 32 bytes.
    #[inline]
    #[must_use]
    pub const fn from_bytes(bytes: Hash32) -> Self {
        Self(bytes)
    }

    /// Borrow the underlying 32 bytes.
    #[inline]
    #[must_use]
    pub const fn as_bytes(&self) -> &Hash32 {
        &self.0
    }
}

impl std::fmt::Debug for LogicRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LogicRef({})", blake3::Hash::from_bytes(self.0).to_hex())
    }
}

/// The 32-byte hash of a Mote's system/prompt template.
///
/// A change in prompt template materially changes what the Mote commits; this
/// hash is part of [`crate::MoteDef`] so the change flows through `mote_def_hash`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PromptTemplateHash(pub Hash32);

impl PromptTemplateHash {
    /// Construct a `PromptTemplateHash` from raw 32 bytes.
    #[inline]
    #[must_use]
    pub const fn from_bytes(bytes: Hash32) -> Self {
        Self(bytes)
    }

    /// Borrow the underlying 32 bytes.
    #[inline]
    #[must_use]
    pub const fn as_bytes(&self) -> &Hash32 {
        &self.0
    }
}

impl std::fmt::Debug for PromptTemplateHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PromptTemplateHash({})",
            blake3::Hash::from_bytes(self.0).to_hex()
        )
    }
}
