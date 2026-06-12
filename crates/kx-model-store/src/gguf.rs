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
use std::io::{BufReader, Read, Seek, SeekFrom};
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

// --- Optional GGUF metadata reader (fail-soft, bounded) --------------------
//
// `validate_gguf_header` above is the hot-path security gate (24 bytes, never
// trusts the file). `read_context_length` is a SEPARATE, opt-in convenience
// that walks the metadata KV section to recover the model's training context
// length so a caller can size its `n_ctx` to the model instead of a hardcoded
// default. It is **fail-soft**: ANY difficulty (missing key, odd value type,
// truncation, implausible counts) yields `None`, and the caller falls back to
// its own default. It never loads weights — it seeks past tensor data and the
// (potentially multi-MB) tokenizer arrays. Every read is bounded and total.

/// Hard cap on the number of metadata KV entries we will iterate (real models
/// have well under 100; anything beyond this is treated as corruption).
const MAX_KV_ENTRIES: u64 = 4096;
/// Hard cap on a GGUF string length (key or value), in bytes.
const MAX_GGUF_STRING_LEN: u64 = 1 << 20;
/// Hard cap on a GGUF array element count.
const MAX_GGUF_ARRAY_LEN: u64 = 1 << 30;

// GGUF metadata value type tags (little-endian u32 on the wire).
const T_UINT8: u32 = 0;
const T_INT8: u32 = 1;
const T_UINT16: u32 = 2;
const T_INT16: u32 = 3;
const T_UINT32: u32 = 4;
const T_INT32: u32 = 5;
const T_FLOAT32: u32 = 6;
const T_BOOL: u32 = 7;
const T_STRING: u32 = 8;
const T_ARRAY: u32 = 9;
const T_UINT64: u32 = 10;
const T_INT64: u32 = 11;
const T_FLOAT64: u32 = 12;

/// Read the model's training context length from the GGUF metadata
/// (`<arch>.context_length`, e.g. `qwen3.context_length`), fail-soft.
///
/// Returns `None` (never an error) for any difficulty so the caller cleanly
/// falls back to its own default `n_ctx`. The walk is bounded (`MAX_KV_ENTRIES`
/// entries, `MAX_GGUF_STRING_LEN` strings, `MAX_GGUF_ARRAY_LEN` arrays),
/// panic-free, and seeks past large values rather than loading them.
#[must_use]
pub fn read_context_length(path: &Path) -> Option<u32> {
    let header = validate_gguf_header(path).ok()?;
    if header.metadata_kv_count > MAX_KV_ENTRIES {
        return None;
    }
    let mut r = BufReader::new(File::open(path).ok()?);
    // Skip the 24-byte header we already validated.
    r.seek(SeekFrom::Start(GGUF_HEADER_LEN as u64)).ok()?;

    for _ in 0..header.metadata_kv_count {
        let key = read_gguf_string(&mut r)?;
        let vtype = read_u32(&mut r)?;
        if key.ends_with(".context_length") || key == "context_length" {
            return read_uint_value(&mut r, vtype);
        }
        skip_value(&mut r, vtype)?;
    }
    None
}

/// Read the model's display name from the GGUF metadata (`general.name`),
/// fail-soft — the same bounded, panic-free walk as [`read_context_length`].
/// Returns `None` for any difficulty so the caller falls back to its own
/// display default (e.g. the file stem).
#[must_use]
pub fn read_model_name(path: &Path) -> Option<String> {
    let header = validate_gguf_header(path).ok()?;
    if header.metadata_kv_count > MAX_KV_ENTRIES {
        return None;
    }
    let mut r = BufReader::new(File::open(path).ok()?);
    r.seek(SeekFrom::Start(GGUF_HEADER_LEN as u64)).ok()?;

    for _ in 0..header.metadata_kv_count {
        let key = read_gguf_string(&mut r)?;
        let vtype = read_u32(&mut r)?;
        if key == "general.name" {
            if vtype != T_STRING {
                return None;
            }
            return read_gguf_string(&mut r);
        }
        skip_value(&mut r, vtype)?;
    }
    None
}

/// Read exactly `n` bytes, or `None` on EOF / error.
fn read_n<R: Read>(r: &mut R, n: usize) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf).ok()?;
    Some(buf)
}

fn read_u32<R: Read>(r: &mut R) -> Option<u32> {
    let b = read_n(r, 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_u64<R: Read>(r: &mut R) -> Option<u64> {
    let b = read_n(r, 8)?;
    let arr: [u8; 8] = b.try_into().ok()?;
    Some(u64::from_le_bytes(arr))
}

/// Read a GGUF string: u64 length (capped) + that many UTF-8 bytes.
fn read_gguf_string<R: Read>(r: &mut R) -> Option<String> {
    let len = read_u64(r)?;
    if len > MAX_GGUF_STRING_LEN {
        return None;
    }
    let bytes = read_n(r, usize::try_from(len).ok()?)?;
    String::from_utf8(bytes).ok()
}

/// Fixed byte size of a scalar GGUF value type (`None` for string/array).
fn scalar_size(t: u32) -> Option<u64> {
    match t {
        T_UINT8 | T_INT8 | T_BOOL => Some(1),
        T_UINT16 | T_INT16 => Some(2),
        T_UINT32 | T_INT32 | T_FLOAT32 => Some(4),
        T_UINT64 | T_INT64 | T_FLOAT64 => Some(8),
        _ => None,
    }
}

/// Read an integer-typed value as a `u32`. `None` if the type isn't an integer
/// we recognize or the value overflows `u32` (implausible for a context length).
fn read_uint_value<R: Read>(r: &mut R, vtype: u32) -> Option<u32> {
    match vtype {
        T_UINT32 | T_INT32 => read_u32(r),
        T_UINT16 => {
            let b = read_n(r, 2)?;
            Some(u32::from(u16::from_le_bytes([b[0], b[1]])))
        }
        T_UINT64 | T_INT64 => u32::try_from(read_u64(r)?).ok(),
        _ => None,
    }
}

/// Seek past one metadata value of type `vtype` (used for keys we don't want).
fn skip_value<R: Read + Seek>(r: &mut R, vtype: u32) -> Option<()> {
    match vtype {
        T_STRING => {
            let len = read_u64(r)?;
            if len > MAX_GGUF_STRING_LEN {
                return None;
            }
            skip_n(r, len)
        }
        T_ARRAY => {
            let elem_t = read_u32(r)?;
            let count = read_u64(r)?;
            if count > MAX_GGUF_ARRAY_LEN {
                return None;
            }
            if elem_t == T_STRING {
                for _ in 0..count {
                    let len = read_u64(r)?;
                    if len > MAX_GGUF_STRING_LEN {
                        return None;
                    }
                    skip_n(r, len)?;
                }
                Some(())
            } else if elem_t == T_ARRAY {
                None // nested arrays are not a valid GGUF shape
            } else {
                let sz = scalar_size(elem_t)?;
                skip_n(r, count.checked_mul(sz)?)
            }
        }
        other => skip_n(r, scalar_size(other)?),
    }
}

/// Seek forward `n` bytes (discarding `BufReader`'s buffer as needed).
fn skip_n<R: Seek>(r: &mut R, n: u64) -> Option<()> {
    r.seek(SeekFrom::Current(i64::try_from(n).ok()?)).ok()?;
    Some(())
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

    // -- read_context_length (fail-soft metadata reader) --------------------

    fn gguf_str(s: &str) -> Vec<u8> {
        let mut v = (s.len() as u64).to_le_bytes().to_vec();
        v.extend_from_slice(s.as_bytes());
        v
    }

    fn kv_string(key: &str, val: &str) -> Vec<u8> {
        let mut v = gguf_str(key);
        v.extend_from_slice(&T_STRING.to_le_bytes());
        v.extend_from_slice(&gguf_str(val));
        v
    }

    fn kv_u32(key: &str, val: u32) -> Vec<u8> {
        let mut v = gguf_str(key);
        v.extend_from_slice(&T_UINT32.to_le_bytes());
        v.extend_from_slice(&val.to_le_bytes());
        v
    }

    fn kv_string_array(key: &str, vals: &[&str]) -> Vec<u8> {
        let mut v = gguf_str(key);
        v.extend_from_slice(&T_ARRAY.to_le_bytes());
        v.extend_from_slice(&T_STRING.to_le_bytes());
        v.extend_from_slice(&(vals.len() as u64).to_le_bytes());
        for s in vals {
            v.extend_from_slice(&gguf_str(s));
        }
        v
    }

    fn gguf_with_kvs(kvs: &[Vec<u8>]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(GGUF_MAGIC);
        v.extend_from_slice(&3u32.to_le_bytes()); // version
        v.extend_from_slice(&0u64.to_le_bytes()); // tensor_count
        v.extend_from_slice(&(kvs.len() as u64).to_le_bytes()); // kv_count
        for kv in kvs {
            v.extend_from_slice(kv);
        }
        v
    }

    #[test]
    fn reads_context_length_after_skipping_string_and_array() {
        // A realistic ordering: arch string, a big tokenizer string-array, then
        // the arch context_length. Proves both the string-skip and array-skip
        // paths, and that we recover the value behind them.
        let f = write_tmp(&gguf_with_kvs(&[
            kv_string("general.architecture", "qwen3"),
            kv_string_array("tokenizer.ggml.tokens", &["a", "bb", "ccc"]),
            kv_u32("qwen3.context_length", 40960),
        ]));
        assert_eq!(read_context_length(f.path()), Some(40960));
    }

    #[test]
    fn missing_context_length_key_is_none() {
        let f = write_tmp(&gguf_with_kvs(&[
            kv_string("general.architecture", "qwen3"),
            kv_u32("qwen3.block_count", 36),
        ]));
        assert_eq!(read_context_length(f.path()), None);
    }

    #[test]
    fn corrupt_metadata_is_none_not_panic() {
        // kv_count claims 2 entries but the bytes end after the header → the
        // bounded reader hits EOF and fails soft.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(GGUF_MAGIC);
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&2u64.to_le_bytes()); // claims 2 KV, provides 0
        let f = write_tmp(&bytes);
        assert_eq!(read_context_length(f.path()), None);
    }

    #[test]
    fn bad_header_context_length_is_none() {
        let f = write_tmp(b"not a gguf file at all");
        assert_eq!(read_context_length(f.path()), None);
    }
}
