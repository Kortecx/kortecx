//! Total record framing shared by the dedup and stat-bounds checks.
//!
//! [`frame`] splits raw bytes into records per a [`RecordFraming`]. It is a TOTAL
//! parser: every malformed framing (truncated length prefix, non-divisible
//! fixed-width input, zero width) yields a [`FramingError`] carrying the byte
//! offset — never a panic. Records borrow from the input (zero-copy).

use kx_critic_types::RecordFraming;
use smallvec::SmallVec;

/// A framing parse failure, localized to a byte offset.
pub(crate) struct FramingError {
    pub(crate) at_offset: u64,
}

/// Split `input` into records per `framing`. Records borrow from `input`.
///
/// # Errors
///
/// Returns [`FramingError`] (with the failing byte offset) for a truncated
/// length-prefix, a fixed-width input whose length is not a multiple of `width`,
/// or a zero `width`.
pub(crate) fn frame<'a>(
    framing: RecordFraming,
    input: &'a [u8],
) -> Result<SmallVec<[&'a [u8]; 16]>, FramingError> {
    let mut out: SmallVec<[&'a [u8]; 16]> = SmallVec::new();
    match framing {
        RecordFraming::LinesLf => {
            // LF-delimited; a trailing newline does NOT yield a final empty
            // record. An empty input yields zero records.
            let mut start = 0usize;
            for (i, &b) in input.iter().enumerate() {
                if b == b'\n' {
                    out.push(&input[start..i]);
                    start = i + 1;
                }
            }
            if start < input.len() {
                out.push(&input[start..]);
            }
        }
        RecordFraming::LengthPrefixedU32 => {
            let mut pos = 0usize;
            while pos < input.len() {
                let Some(len_bytes) = input.get(pos..pos + 4) else {
                    return Err(FramingError {
                        at_offset: pos as u64,
                    });
                };
                let len =
                    u32::from_le_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]])
                        as usize;
                let body_start = pos + 4;
                let Some(body) = input.get(body_start..body_start + len) else {
                    return Err(FramingError {
                        at_offset: body_start as u64,
                    });
                };
                out.push(body);
                pos = body_start + len;
            }
        }
        RecordFraming::FixedWidth { width } => {
            let width = width as usize;
            if width == 0 {
                return Err(FramingError { at_offset: 0 });
            }
            if !input.len().is_multiple_of(width) {
                // The first incomplete record starts at the last whole boundary.
                let at = (input.len() / width) * width;
                return Err(FramingError {
                    at_offset: at as u64,
                });
            }
            let mut pos = 0usize;
            while pos < input.len() {
                out.push(&input[pos..pos + width]);
                pos += width;
            }
        }
    }
    Ok(out)
}

/// Extract a `[start, end)` byte sub-range key from a record. Returns `None` if
/// the range is out of bounds or inverted (the caller maps that to
/// `Unparseable`). `None` range selects the whole record.
pub(crate) fn key_of(record: &[u8], range: Option<(u32, u32)>) -> Option<&[u8]> {
    match range {
        None => Some(record),
        Some((start, end)) => {
            let (start, end) = (start as usize, end as usize);
            if start > end {
                return None;
            }
            record.get(start..end)
        }
    }
}
