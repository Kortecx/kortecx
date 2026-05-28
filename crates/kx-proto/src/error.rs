//! [`ConvertError`] — the typed failure for decoding a wire (protobuf) message
//! into its canonical Rust domain type.
//!
//! Every variant is a **boundary rejection**: the wire is untrusted, so the
//! `proto -> domain` direction validates 32-byte hash lengths, rejects the
//! `*_UNSPECIFIED` enum sentinel, and requires present message fields. The
//! reverse (`domain -> proto`) is total and never errors.

/// Failure decoding a protobuf message into its canonical Rust domain type.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConvertError {
    /// A `bytes` field that must hold a 32-byte BLAKE3 hash had the wrong length.
    #[error("field `{field}`: expected 32-byte hash, got {len} bytes")]
    BadHashLength {
        /// The offending field's dotted name (e.g. `MoteDef.logic_ref`).
        field: &'static str,
        /// The actual byte length received on the wire.
        len: usize,
    },

    /// A required nested message field was absent (`None`) on the wire.
    #[error("field `{field}`: required message was absent")]
    MissingField {
        /// The offending field's dotted name (e.g. `SubmitMoteRequest.mote`).
        field: &'static str,
    },

    /// An enum field was left at its `*_UNSPECIFIED = 0` sentinel. proto3's
    /// zero-default must not silently denote a real class, so it is rejected.
    #[error("enum `{enum_name}`: value left UNSPECIFIED (must be set explicitly)")]
    UnspecifiedEnum {
        /// The enum type name (e.g. `NdClass`).
        enum_name: &'static str,
    },

    /// An enum field carried an `i32` outside the known set of wire values.
    #[error("enum `{enum_name}`: unknown wire value {value}")]
    UnknownEnum {
        /// The enum type name (e.g. `NdClass`).
        enum_name: &'static str,
        /// The unrecognized wire value.
        value: i32,
    },

    /// A scalar exceeded the domain type's range (e.g. `schema_version` is `u16`
    /// in the domain but `uint32` on the wire).
    #[error("field `{field}`: value {value} out of range for the domain type")]
    OutOfRange {
        /// The offending field's dotted name.
        field: &'static str,
        /// The out-of-range value.
        value: u64,
    },
}
