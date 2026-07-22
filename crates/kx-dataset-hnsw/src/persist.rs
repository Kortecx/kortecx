// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! The on-disk cache format: a flat, deterministic, little-endian encoding of the
//! `(content-ref, vector)` records.
//!
//! This is a REBUILDABLE CACHE, never a source of truth — the HNSW graph is
//! rebuilt from these records on open, so the format is decoupled from
//! `hnsw_rs`'s internal dump format and a break is recovered by replay (D40), not
//! a migration. The decoder is total + panic-free over arbitrary bytes and
//! treats the file as untrusted input (bounded allocation, fail-closed).

use kx_content::ContentRef;

use crate::error::HnswError;

/// Magic + format-version header. Bump the trailing digits on a format change.
pub(crate) const MAGIC: [u8; 8] = *b"KXHNSW01";

/// Decoded cache payload: one `(content-ref, embedding)` record per indexed row.
pub(crate) type Records = Vec<(ContentRef, Vec<f32>)>;

/// Encode the records into the flat cache form:
/// `MAGIC | dim:u32le | count:u32le | (ref:32B | f32le * dim) * count`.
pub(crate) fn encode_records(dim: u32, ids: &[ContentRef], vectors: &[Vec<f32>]) -> Vec<u8> {
    let count = u32::try_from(ids.len()).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(16 + ids.len() * (32 + dim as usize * 4));
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&dim.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    for (id, vector) in ids.iter().zip(vectors.iter()) {
        out.extend_from_slice(id.as_bytes());
        for x in vector {
            out.extend_from_slice(&x.to_le_bytes());
        }
    }
    out
}

/// A reading cursor that never indexes out of bounds (fail-closed loader).
fn take<'a>(bytes: &'a [u8], off: &mut usize, n: usize) -> Result<&'a [u8], HnswError> {
    let end = off
        .checked_add(n)
        .ok_or(HnswError::Corrupt("length overflow"))?;
    let slice = bytes
        .get(*off..end)
        .ok_or(HnswError::Corrupt("truncated"))?;
    *off = end;
    Ok(slice)
}

/// Decode the flat cache form. Total + panic-free over arbitrary bytes: any
/// malformed input is a graceful `Corrupt` error, never a panic, and allocation
/// is bounded by the actual byte length (a hostile `count`/`dim` cannot OOM).
pub(crate) fn decode_records(bytes: &[u8]) -> Result<(u32, Records), HnswError> {
    let mut off = 0usize;
    if take(bytes, &mut off, 8)? != MAGIC {
        return Err(HnswError::Corrupt("bad magic"));
    }
    let dim = u32::from_le_bytes(
        take(bytes, &mut off, 4)?
            .try_into()
            .map_err(|_| HnswError::Corrupt("dim"))?,
    );
    let count = u32::from_le_bytes(
        take(bytes, &mut off, 4)?
            .try_into()
            .map_err(|_| HnswError::Corrupt("count"))?,
    );
    let dim_usize = dim as usize;
    let count_usize = count as usize;
    // Bound the pre-allocation by the remaining bytes — a record is at least 32
    // bytes (the ref), so the file cannot declare more records than it can hold.
    let mut out = Vec::with_capacity(count_usize.min(bytes.len() / 32));
    for _ in 0..count_usize {
        let id_bytes: [u8; 32] = take(bytes, &mut off, 32)?
            .try_into()
            .map_err(|_| HnswError::Corrupt("ref"))?;
        let id = ContentRef::from_bytes(id_bytes);
        let mut vector = Vec::with_capacity(dim_usize.min(bytes.len() / 4));
        for _ in 0..dim_usize {
            let f = f32::from_le_bytes(
                take(bytes, &mut off, 4)?
                    .try_into()
                    .map_err(|_| HnswError::Corrupt("vector element"))?,
            );
            vector.push(f);
        }
        out.push((id, vector));
    }
    // Reject trailing garbage — the cache must be exactly the declared records.
    if off != bytes.len() {
        return Err(HnswError::Corrupt("trailing bytes"));
    }
    Ok((dim, out))
}
