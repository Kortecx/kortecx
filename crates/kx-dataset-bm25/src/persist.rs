// SPDX-License-Identifier: Apache-2.0
//! The on-disk cache format: a flat, deterministic encoding of the
//! `(content-ref, text)` records.
//!
//! This is a REBUILDABLE CACHE, never a source of truth — the inverted index is
//! rebuilt by re-tokenizing these records on open, so the format is decoupled from
//! the tokenizer internals and a break is recovered by replay (D40), not a
//! migration. The decoder is total + panic-free over arbitrary bytes and treats
//! the file as untrusted input (bounded allocation, fail-closed).

use kx_content::ContentRef;

use crate::error::Bm25Error;
use crate::tokenize::TOKENIZER_VERSION;

/// Magic + format-version header. Bump the trailing digits on a format change.
pub(crate) const MAGIC: [u8; 8] = *b"KXBM2501";

/// Decoded cache payload: one `(content-ref, text)` record per indexed document.
pub(crate) type Records = Vec<(ContentRef, String)>;

/// Encode the records into the flat cache form:
/// `MAGIC | tokenizer_version:u32le | count:u32le | (ref:32B | text_len:u32le | utf8) * count`.
pub(crate) fn encode_records(ids: &[ContentRef], texts: &[String]) -> Vec<u8> {
    let count = u32::try_from(ids.len()).unwrap_or(u32::MAX);
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&TOKENIZER_VERSION.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    for (id, text) in ids.iter().zip(texts.iter()) {
        let tb = text.as_bytes();
        let tlen = u32::try_from(tb.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(id.as_bytes());
        out.extend_from_slice(&tlen.to_le_bytes());
        out.extend_from_slice(tb);
    }
    out
}

/// A reading cursor that never indexes out of bounds (fail-closed loader).
fn take<'a>(bytes: &'a [u8], off: &mut usize, n: usize) -> Result<&'a [u8], Bm25Error> {
    let end = off
        .checked_add(n)
        .ok_or(Bm25Error::Corrupt("length overflow"))?;
    let slice = bytes
        .get(*off..end)
        .ok_or(Bm25Error::Corrupt("truncated"))?;
    *off = end;
    Ok(slice)
}

/// Decode the flat cache form. Total + panic-free over arbitrary bytes: any
/// malformed input (bad magic, truncation, bad UTF-8, trailing garbage) is a
/// graceful `Corrupt` error, and allocation is bounded by the actual byte length
/// (a hostile `count`/`text_len` cannot OOM).
pub(crate) fn decode_records(bytes: &[u8]) -> Result<Records, Bm25Error> {
    let mut off = 0usize;
    if take(bytes, &mut off, 8)? != MAGIC {
        return Err(Bm25Error::Corrupt("bad magic"));
    }
    // The tokenizer version is informational: the index is re-tokenized with the
    // CURRENT tokenizer on open, so a version mismatch is recovered by rebuild.
    let _tok_version = u32::from_le_bytes(
        take(bytes, &mut off, 4)?
            .try_into()
            .map_err(|_| Bm25Error::Corrupt("tokenizer version"))?,
    );
    let count = u32::from_le_bytes(
        take(bytes, &mut off, 4)?
            .try_into()
            .map_err(|_| Bm25Error::Corrupt("count"))?,
    );
    // A record is at least 36 bytes (32-byte ref + 4-byte length), so the file
    // cannot declare more records than it can hold.
    let mut out = Vec::with_capacity((count as usize).min(bytes.len() / 36));
    for _ in 0..count {
        let id_bytes: [u8; 32] = take(bytes, &mut off, 32)?
            .try_into()
            .map_err(|_| Bm25Error::Corrupt("ref"))?;
        let id = ContentRef::from_bytes(id_bytes);
        let tlen = u32::from_le_bytes(
            take(bytes, &mut off, 4)?
                .try_into()
                .map_err(|_| Bm25Error::Corrupt("text length"))?,
        ) as usize;
        let tbytes = take(bytes, &mut off, tlen)?;
        let text =
            String::from_utf8(tbytes.to_vec()).map_err(|_| Bm25Error::Corrupt("text utf8"))?;
        out.push((id, text));
    }
    // Reject trailing garbage — the cache must be exactly the declared records.
    if off != bytes.len() {
        return Err(Bm25Error::Corrupt("trailing bytes"));
    }
    Ok(out)
}
