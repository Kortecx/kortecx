//! [`GrammarError`] — the fail-closed error for deserializing a carried spec.

use std::fmt;

/// Why a [`crate::ToolEnvelopeSpec`] could not be recovered from the opaque
/// `kx_mote::Grammar.raw` carrier. An engine leg that hits this MUST fail the
/// dispatch closed (never silently fall back to unconstrained generation), so a
/// corrupt carrier can never quietly disable the constraint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrammarError {
    /// The carrier bytes did not deserialize into a [`crate::ToolEnvelopeSpec`].
    Malformed {
        /// A short, non-secret diagnostic.
        diagnostic: String,
    },
}

impl fmt::Display for GrammarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GrammarError::Malformed { diagnostic } => {
                write!(f, "malformed grammar spec carrier: {diagnostic}")
            }
        }
    }
}

impl std::error::Error for GrammarError {}
