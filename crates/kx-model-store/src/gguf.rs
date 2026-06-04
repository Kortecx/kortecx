// SPDX-License-Identifier: Apache-2.0
//! Fail-closed GGUF-header validation.
//!
//! A model file is **new untrusted input** (it may be corrupt, truncated, or
//! hostile). We do not parse the whole file or the full metadata KV block — we
//! read a bounded fixed-size header prefix and reject anything that is not a
//! plausible GGUF v2/v3 file. Heavy parsing is llama.cpp's job at load time;
//! this is the cheap gate that turns "garbage path" into a typed refusal instead
//! of a deep-in-FFI surprise.
//!
//! GGUF layout (little-endian), the prefix we validate:
//! ```text
//! offset 0:  u8[4]  magic            = b"GGUF"
//! offset 4:  u32    version          ∈ {2, 3}
//! offset 8:  u64    tensor_count
//! offset 16: u64    metadata_kv_count
//! ```

use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::errors::ModelStoreError;

/// The fixed GGUF prefix we read: magic(4) + version(4) + 2×u64(16) = 24 bytes.
const GGUF_HEADER_LEN: usize = 24;

/// GGUF magic bytes.
const GGUF_MAGIC: &[u8; 4] = b"GGUF";

/// GGUF versions this runtime accepts. v1 is obsolete; current llama.cpp emits
/// v3. Anything else is refused so an unknown layout cannot be mistaken for a
/// model.
const SUPPORTED_VERSIONS: [u32; 2] = [2, 3];

/// A sanity ceiling on the header counts. Real models are well under this; a
/// value above it is treated as corruption rather than trusted.
const MAX_PLAUSIBLE_COUNT: u64 = 1 << 32;

/// The validated GGUF header prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GgufHeader {
    /// GGUF container version (2 or 3).
    pub version: u32,
    /// Number of tensors declared in the file.
    pub tensor_count: u64,
    /// Number of metadata key/value pairs declared in the file.
    pub metadata_kv_count: u64,
}

/// Read and validate the GGUF header prefix of `path`, fail-closed.
///
/// # Errors
///
/// - [`ModelStoreError::ModelFileNotReadable`] if the file cannot be opened.
/// - [`ModelStoreError::InvalidGguf`] if the file is shorter than the header,
///   the magic is wrong, the version is unsupported, or a count is implausible.
pub fn validate_gguf_header(path: &Path) -> Result<GgufHeader, ModelStoreError> {
    let mut file = File::open(path).map_err(|e| ModelStoreError::ModelFileNotReadable {
        path: path.to_path_buf(),
        reason: e.kind().to_string(),
    })?;

    let mut buf = [0u8; GGUF_HEADER_LEN];
    // `read_exact` fails closed on a file shorter than the header.
    file.read_exact(&mut buf)
        .map_err(|_| ModelStoreError::InvalidGguf {
            path: path.to_path_buf(),
            reason: format!("file shorter than the {GGUF_HEADER_LEN}-byte GGUF header"),
        })?;

    if &buf[0..4] != GGUF_MAGIC {
        return Err(ModelStoreError::InvalidGguf {
            path: path.to_path_buf(),
            reason: "bad magic (not a GGUF file)".to_string(),
        });
    }

    let version = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    if !SUPPORTED_VERSIONS.contains(&version) {
        return Err(ModelStoreError::InvalidGguf {
            path: path.to_path_buf(),
            reason: format!(
                "unsupported GGUF version {version} (accepted: {SUPPORTED_VERSIONS:?})"
            ),
        });
    }

    let tensor_count = read_u64_le(&buf, 8);
    let metadata_kv_count = read_u64_le(&buf, 16);
    if tensor_count > MAX_PLAUSIBLE_COUNT || metadata_kv_count > MAX_PLAUSIBLE_COUNT {
        return Err(ModelStoreError::InvalidGguf {
            path: path.to_path_buf(),
            reason: format!(
                "implausible header counts (tensors={tensor_count}, kv={metadata_kv_count}); \
                 treating as corruption"
            ),
        });
    }

    Ok(GgufHeader {
        version,
        tensor_count,
        metadata_kv_count,
    })
}

/// Read a little-endian `u64` at `offset` from a buffer known to be long enough.
#[inline]
fn read_u64_le(buf: &[u8; GGUF_HEADER_LEN], offset: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&buf[offset..offset + 8]);
    u64::from_le_bytes(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_tmp(bytes: &[u8]) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().expect("tmp file");
        f.write_all(bytes).expect("write");
        f.flush().expect("flush");
        f
    }

    fn valid_header(version: u32) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(GGUF_MAGIC);
        v.extend_from_slice(&version.to_le_bytes());
        v.extend_from_slice(&3u64.to_le_bytes()); // tensor_count
        v.extend_from_slice(&7u64.to_le_bytes()); // metadata_kv_count
        v
    }

    #[test]
    fn accepts_valid_v3_header() {
        let f = write_tmp(&valid_header(3));
        let h = validate_gguf_header(f.path()).expect("v3 header valid");
        assert_eq!(h.version, 3);
        assert_eq!(h.tensor_count, 3);
        assert_eq!(h.metadata_kv_count, 7);
    }

    #[test]
    fn accepts_valid_v2_header() {
        let f = write_tmp(&valid_header(2));
        assert_eq!(validate_gguf_header(f.path()).unwrap().version, 2);
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = valid_header(3);
        bytes[0] = b'X';
        let f = write_tmp(&bytes);
        let err = validate_gguf_header(f.path()).unwrap_err();
        assert!(matches!(err, ModelStoreError::InvalidGguf { .. }));
    }

    #[test]
    fn rejects_unsupported_version() {
        let f = write_tmp(&valid_header(1));
        assert!(matches!(
            validate_gguf_header(f.path()).unwrap_err(),
            ModelStoreError::InvalidGguf { .. }
        ));
    }

    #[test]
    fn rejects_truncated_file() {
        let f = write_tmp(b"GGUF\x03"); // 5 bytes, shorter than the header
        assert!(matches!(
            validate_gguf_header(f.path()).unwrap_err(),
            ModelStoreError::InvalidGguf { .. }
        ));
    }

    #[test]
    fn rejects_implausible_counts() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(GGUF_MAGIC);
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&u64::MAX.to_le_bytes()); // absurd tensor_count
        bytes.extend_from_slice(&7u64.to_le_bytes());
        let f = write_tmp(&bytes);
        assert!(matches!(
            validate_gguf_header(f.path()).unwrap_err(),
            ModelStoreError::InvalidGguf { .. }
        ));
    }

    #[test]
    fn missing_file_is_not_readable() {
        let err = validate_gguf_header(Path::new("/nonexistent/model.gguf")).unwrap_err();
        assert!(matches!(err, ModelStoreError::ModelFileNotReadable { .. }));
    }
}
