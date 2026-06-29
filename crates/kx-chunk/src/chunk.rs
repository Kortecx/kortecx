// SPDX-License-Identifier: Apache-2.0
//! The deterministic recursive-character chunker.

/// The chunker algorithm version. Bumped on any change to the separator
/// hierarchy, the merge rule, or the offset semantics — it is a retrieval-index
/// fingerprint axis, so a bump forces a re-ingest rather than a silent mismatch.
pub const CHUNKER_VERSION: u32 = 1;

/// Default maximum chunk size in Unicode chars.
pub const DEFAULT_MAX_CHARS: usize = 1000;

/// Default overlap (trailing chars of chunk *i* that also begin chunk *i+1*).
pub const DEFAULT_OVERLAP_CHARS: usize = 200;

/// The separator hierarchy, highest priority first. A break is preferred at a
/// paragraph boundary, then a line, then a sentence (". "), then a word (" ");
/// the empty separator is the base case (a hard char cut) that guarantees
/// termination. Each entry is matched as a sequence of `char`s.
const SEPARATORS: &[&str] = &["\n\n", "\n", ". ", " "];

/// Chunking parameters. `max_chars` bounds each chunk; `overlap_chars` is the
/// trailing window of one chunk re-included at the head of the next (so a passage
/// straddling a boundary is still wholly present in at least one chunk).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkParams {
    /// Maximum chunk size in Unicode chars (coerced to `>= 1`).
    pub max_chars: usize,
    /// Overlap in Unicode chars (coerced to `< max_chars` to guarantee progress).
    pub overlap_chars: usize,
}

impl Default for ChunkParams {
    fn default() -> Self {
        Self {
            max_chars: DEFAULT_MAX_CHARS,
            overlap_chars: DEFAULT_OVERLAP_CHARS,
        }
    }
}

/// One chunk: its text plus its contiguous `[char_start, char_end)` range in the
/// parent document (char indices, not bytes). `text` is exactly the parent's
/// chars in that range, so `ContentRef::of(text.as_bytes())` is a stable,
/// reproducible key and the range is exact provenance for display/highlighting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    /// The chunk text — a contiguous substring of the parent.
    pub text: String,
    /// 0-based ordinal of this chunk within the parent.
    pub index: u32,
    /// Inclusive char offset where this chunk begins in the parent.
    pub char_start: usize,
    /// Exclusive char offset where this chunk ends in the parent.
    pub char_end: usize,
}

/// Split `text` into bounded, overlapping chunks. Empty/whitespace-only input
/// yields no chunks (nothing to retrieve); a short document yields a single
/// chunk; a long document is split greedily on the highest-priority separator
/// present within each window, falling back to a hard char cut.
///
/// Deterministic: fixed separators, greedy left-to-right, integer char offsets —
/// no locale, RNG, float, or model. Every chunk is `<= max_chars` chars, and the
/// chunks cover the whole document (with `overlap_chars` of overlap between
/// neighbours).
pub fn chunk(text: &str, params: ChunkParams) -> Vec<Chunk> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    if n == 0 {
        return Vec::new();
    }
    let max = params.max_chars.max(1);
    // Overlap must be strictly less than `max` so the cursor always advances.
    let overlap = params.overlap_chars.min(max.saturating_sub(1));

    if n <= max {
        return vec![Chunk {
            text: chars.iter().collect(),
            index: 0,
            char_start: 0,
            char_end: n,
        }];
    }

    let mut chunks = Vec::new();
    let mut start = 0usize;
    let mut index = 0u32;
    loop {
        let hard_end = (start + max).min(n);
        let end = if hard_end >= n {
            n
        } else {
            find_break(&chars, start, hard_end)
        };
        chunks.push(Chunk {
            text: chars[start..end].iter().collect(),
            index,
            char_start: start,
            char_end: end,
        });
        index += 1;
        if end >= n {
            break;
        }
        // Advance with overlap; `overlap < max <= end - start` guarantees progress.
        let next = end.saturating_sub(overlap);
        start = if next > start { next } else { start + 1 };
    }
    chunks
}

/// Find the greediest break point in `(start, hard_end]`: the LATEST position
/// just after the highest-priority separator that fits the window. Returns
/// `hard_end` (a hard char cut) when no separator is present. The returned index
/// is `> start` and `<= hard_end`, so the chunk is always non-empty and within
/// `max_chars`.
fn find_break(chars: &[char], start: usize, hard_end: usize) -> usize {
    for sep in SEPARATORS {
        let sep_chars: Vec<char> = sep.chars().collect();
        let len = sep_chars.len();
        // Scan break points (the index just after the separator) from the window
        // edge backwards, taking the latest that fits — a greedy full chunk.
        let mut b = hard_end;
        while b > start {
            if b >= start + len && chars[b - len..b] == sep_chars[..] {
                return b;
            }
            b -= 1;
        }
    }
    hard_end
}
