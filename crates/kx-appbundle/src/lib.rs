//! `kortecx.appbundle/v1` — the portable App archive codec.
//!
//! An **App bundle** packages an App for portability: the canonical `AppEnvelope`
//! bytes plus the base64-encoded closure of every content-store blob the App
//! references. The wire form is a single **canonical-JSON, all-strings** document
//! (sorted keys, compact) so Rust, Python, and TypeScript emit byte-identical
//! bundles — the cross-language contract is locked by `tests/golden/apps`.
//!
//! ```
//! use kx_appbundle::AppBundle;
//! use std::collections::BTreeMap;
//!
//! let mut blobs = BTreeMap::new();
//! blobs.insert("aa".repeat(32), b"hello".to_vec());
//! let bundle = AppBundle {
//!     app_digest: "bb".repeat(32),
//!     source_digest: None,
//!     envelope: br#"{"name":"x","schema":"kortecx.app/v1"}"#.to_vec(),
//!     blobs,
//! };
//! let wire = bundle.to_json().unwrap();
//! assert_eq!(AppBundle::from_json(&wire).unwrap(), bundle);
//! ```
//!
//! This crate owns the **container format only**. It validates structure — the
//! `kortecx.appbundle/v1` schema tag, 64-char lowercase-hex refs, and well-formed
//! base64 — and never cryptographic identity: the runtime re-derives every blob
//! ref (`blake3` via `PutContent`) and re-validates the envelope (`SaveApp`)
//! server-side, so a bundle is a transport hint, never a trust boundary.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::collections::BTreeMap;

use base64::Engine as _;
use serde::{Deserialize, Serialize};

/// The bundle schema/version tag. Readers fail closed on a mismatch.
pub const BUNDLE_SCHEMA: &str = "kortecx.appbundle/v1";

/// Errors from bundle (de)serialization + structural validation.
#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    /// The bytes were not valid bundle JSON.
    #[error("invalid app bundle JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// `schema` was absent or not [`BUNDLE_SCHEMA`].
    #[error("unsupported app bundle schema {got:?} (expected {expected:?})")]
    Schema {
        /// The schema tag found in the bundle.
        got: String,
        /// The schema tag this binary supports.
        expected: &'static str,
    },
    /// A structural failure (bad hex ref, bad base64, non-UTF-8 envelope, …).
    #[error("invalid app bundle: {0}")]
    Invalid(String),
}

/// The wire projection: an all-string document. Field order is irrelevant —
/// [`AppBundle::to_json`] re-serializes through `serde_json::Value` (sorted keys),
/// so the emitted bytes are canonical regardless of declaration order. Empty
/// `blobs` and absent `source_digest` are omitted (byte-invariant when unset).
#[derive(Serialize, Deserialize)]
struct Wire {
    app_digest: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    blobs: BTreeMap<String, String>,
    envelope: String,
    schema: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_digest: Option<String>,
}

/// A decoded App bundle: the canonical envelope bytes + the raw content closure,
/// named + tamper-checkable by the App's `app_digest` (verified by the runtime,
/// not here). `source_digest` is an optional lineage hint (never authenticity).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppBundle {
    /// 64-char lowercase-hex handle-free App identity (`app_digest`).
    pub app_digest: String,
    /// Optional 64-hex lineage hint — the digest this App was exported/cloned from.
    pub source_digest: Option<String>,
    /// The canonical `AppEnvelope` bytes, verbatim.
    pub envelope: Vec<u8>,
    /// The content closure: 64-hex content-store ref → raw blob bytes.
    pub blobs: BTreeMap<String, Vec<u8>>,
}

impl AppBundle {
    /// Serialize to the canonical `kortecx.appbundle/v1` wire string (sorted keys,
    /// compact, base64-STANDARD blobs). Deterministic + cross-language byte-identical.
    ///
    /// # Errors
    /// [`BundleError::Invalid`] if the envelope bytes are not valid UTF-8;
    /// [`BundleError::Json`] on a serialization failure (never in practice).
    pub fn to_json(&self) -> Result<String, BundleError> {
        let envelope = String::from_utf8(self.envelope.clone())
            .map_err(|_| BundleError::Invalid("envelope bytes are not valid UTF-8".into()))?;
        let blobs = self
            .blobs
            .iter()
            .map(|(r, b)| {
                (
                    r.clone(),
                    base64::engine::general_purpose::STANDARD.encode(b),
                )
            })
            .collect();
        let wire = Wire {
            app_digest: self.app_digest.clone(),
            blobs,
            envelope,
            schema: BUNDLE_SCHEMA.to_string(),
            source_digest: self.source_digest.clone(),
        };
        // Route through Value so keys sort (canonical), like the envelope form.
        let value = serde_json::to_value(&wire)?;
        Ok(serde_json::to_string(&value)?)
    }

    /// Parse + structurally validate a `kortecx.appbundle/v1` wire string.
    ///
    /// Checks the schema tag, that `app_digest` / `source_digest` / every blob key
    /// is 64-char lowercase hex, and that every blob value is valid base64. Does
    /// NOT verify that a blob hashes to its ref or that the envelope is valid — the
    /// runtime re-derives + re-validates those server-side.
    ///
    /// # Errors
    /// [`BundleError::Json`] on malformed JSON; [`BundleError::Schema`] on a tag
    /// mismatch; [`BundleError::Invalid`] on a bad hex ref or bad base64.
    pub fn from_json(s: &str) -> Result<Self, BundleError> {
        let wire: Wire = serde_json::from_str(s)?;
        if wire.schema != BUNDLE_SCHEMA {
            return Err(BundleError::Schema {
                got: wire.schema,
                expected: BUNDLE_SCHEMA,
            });
        }
        check_hex("app_digest", &wire.app_digest)?;
        if let Some(sd) = &wire.source_digest {
            check_hex("source_digest", sd)?;
        }
        let mut blobs = BTreeMap::new();
        for (r, b64) in wire.blobs {
            check_hex("blob ref", &r)?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&b64)
                .map_err(|e| BundleError::Invalid(format!("blob {r}: invalid base64: {e}")))?;
            blobs.insert(r, bytes);
        }
        Ok(Self {
            app_digest: wire.app_digest,
            source_digest: wire.source_digest,
            envelope: wire.envelope.into_bytes(),
            blobs,
        })
    }

    /// Total raw byte size of the content closure (for an import ceiling).
    #[must_use]
    pub fn total_blob_bytes(&self) -> u64 {
        self.blobs
            .values()
            .map(|b| u64::try_from(b.len()).unwrap_or(u64::MAX))
            .sum()
    }

    /// Number of blobs in the content closure (for an import ceiling).
    #[must_use]
    pub fn blob_count(&self) -> usize {
        self.blobs.len()
    }
}

/// Validate a 64-char lowercase-hex string (a content ref or a digest).
fn check_hex(field: &str, s: &str) -> Result<(), BundleError> {
    if s.len() != 64
        || !s
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(BundleError::Invalid(format!(
            "{field} must be 64-char lowercase hex, got {s:?}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn hexref(seed: u8) -> String {
        format!("{seed:02x}").repeat(32)
    }

    fn sample() -> AppBundle {
        let mut blobs = BTreeMap::new();
        blobs.insert(hexref(0x02), b"prompt body".to_vec());
        blobs.insert(hexref(0x01), vec![0u8, 1, 2, 255, 128]); // arbitrary bytes
        AppBundle {
            app_digest: hexref(0xaa),
            source_digest: Some(hexref(0xbb)),
            envelope: br#"{"name":"x","schema":"kortecx.app/v1"}"#.to_vec(),
            blobs,
        }
    }

    #[test]
    fn to_json_is_canonical_sorted() {
        let s = sample().to_json().unwrap();
        // Top-level keys in sorted order: app_digest, blobs, envelope, schema, source_digest.
        assert!(s.starts_with(r#"{"app_digest":"#), "got {s}");
        let ai = s.find("\"app_digest\"").unwrap();
        let bi = s.find("\"blobs\"").unwrap();
        let ei = s.find("\"envelope\"").unwrap();
        let sci = s.find("\"schema\"").unwrap();
        let sdi = s.find("\"source_digest\"").unwrap();
        assert!(
            ai < bi && bi < ei && ei < sci && sci < sdi,
            "keys must sort: {s}"
        );
        // The schema tag is present + correct.
        assert!(s.contains(r#""schema":"kortecx.appbundle/v1""#));
    }

    #[test]
    fn round_trips_and_to_json_is_idempotent() {
        let b = sample();
        let json = b.to_json().unwrap();
        let parsed = AppBundle::from_json(&json).unwrap();
        assert_eq!(parsed, b);
        assert_eq!(parsed.to_json().unwrap(), json);
    }

    #[test]
    fn blob_insert_order_does_not_change_bytes() {
        let mut a = AppBundle {
            app_digest: hexref(0xaa),
            source_digest: None,
            envelope: b"{}".to_vec(),
            blobs: BTreeMap::new(),
        };
        let mut b = a.clone();
        a.blobs.insert(hexref(0x01), b"one".to_vec());
        a.blobs.insert(hexref(0x02), b"two".to_vec());
        b.blobs.insert(hexref(0x02), b"two".to_vec());
        b.blobs.insert(hexref(0x01), b"one".to_vec());
        assert_eq!(a.to_json().unwrap(), b.to_json().unwrap());
    }

    #[test]
    fn empty_closure_omits_blobs_and_source_digest() {
        let b = AppBundle {
            app_digest: hexref(0xaa),
            source_digest: None,
            envelope: b"{}".to_vec(),
            blobs: BTreeMap::new(),
        };
        let s = b.to_json().unwrap();
        assert!(!s.contains("blobs"), "empty blobs omitted: {s}");
        assert!(
            !s.contains("source_digest"),
            "None source_digest omitted: {s}"
        );
        assert_eq!(AppBundle::from_json(&s).unwrap(), b);
    }

    #[test]
    fn total_bytes_and_count() {
        let b = sample();
        assert_eq!(b.blob_count(), 2);
        assert_eq!(b.total_blob_bytes(), (b"prompt body".len() + 5) as u64);
    }

    #[test]
    fn from_json_rejects_bad_schema() {
        let s = r#"{"app_digest":"aa","envelope":"{}","schema":"kortecx.appbundle/v2"}"#;
        assert!(matches!(
            AppBundle::from_json(s),
            Err(BundleError::Schema { .. })
        ));
    }

    #[test]
    fn from_json_rejects_bad_hex_ref() {
        let s = r#"{"app_digest":"NOPE","envelope":"{}","schema":"kortecx.appbundle/v1"}"#;
        assert!(matches!(
            AppBundle::from_json(s),
            Err(BundleError::Invalid(_))
        ));
    }

    #[test]
    fn from_json_rejects_bad_base64() {
        let s = format!(
            r#"{{"app_digest":"{}","blobs":{{"{}":"!!!not-base64!!!"}},"envelope":"{{}}","schema":"kortecx.appbundle/v1"}}"#,
            hexref(0xaa),
            hexref(0x01),
        );
        assert!(matches!(
            AppBundle::from_json(&s),
            Err(BundleError::Invalid(_))
        ));
    }

    prop_compose! {
        fn hex_ref()(bytes in prop::array::uniform32(any::<u8>())) -> String {
            use std::fmt::Write as _;
            let mut s = String::with_capacity(64);
            for b in bytes {
                let _ = write!(s, "{b:02x}");
            }
            s
        }
    }

    proptest! {
        /// build → parse → build is byte-identical over arbitrary envelopes + blobs;
        /// parse recovers the exact bytes (SN-4 v2 #5, over the arbitrary input space).
        #[test]
        fn round_trips_byte_identically(
            app_digest in hex_ref(),
            source_digest in prop::option::of(hex_ref()),
            envelope in any::<String>(),
            blobs in prop::collection::btree_map(
                hex_ref(),
                prop::collection::vec(any::<u8>(), 0..80),
                0..6,
            ),
        ) {
            let bundle = AppBundle {
                app_digest,
                source_digest,
                envelope: envelope.into_bytes(),
                blobs,
            };
            let json = bundle.to_json().unwrap();
            let parsed = AppBundle::from_json(&json).unwrap();
            prop_assert_eq!(&parsed, &bundle);
            prop_assert_eq!(parsed.to_json().unwrap(), json);
        }
    }
}
