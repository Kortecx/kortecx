// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! Unit tests: determinism, contiguity, bounds, overlap, separator preference,
//! and codepoint safety.

use crate::chunk::{chunk, ChunkParams};

fn params(max: usize, overlap: usize) -> ChunkParams {
    ChunkParams {
        max_chars: max,
        overlap_chars: overlap,
    }
}

#[test]
fn empty_text_yields_no_chunks() {
    assert!(chunk("", ChunkParams::default()).is_empty());
}

#[test]
fn short_text_is_a_single_chunk_covering_all() {
    let text = "a short document";
    let cs = chunk(text, params(1000, 200));
    assert_eq!(cs.len(), 1);
    assert_eq!(cs[0].text, text);
    assert_eq!(cs[0].index, 0);
    assert_eq!(cs[0].char_start, 0);
    assert_eq!(cs[0].char_end, text.chars().count());
}

#[test]
fn chunks_are_contiguous_substrings_of_the_parent() {
    let text = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima";
    let chars: Vec<char> = text.chars().collect();
    for c in chunk(text, params(20, 5)) {
        let slice: String = chars[c.char_start..c.char_end].iter().collect();
        assert_eq!(c.text, slice, "chunk text must equal parent[start..end]");
    }
}

#[test]
fn every_chunk_is_within_max_chars() {
    let text = "x".repeat(5000);
    for c in chunk(&text, params(100, 20)) {
        assert!(
            c.text.chars().count() <= 100,
            "chunk {} exceeds max_chars",
            c.index
        );
    }
}

#[test]
fn chunks_cover_the_whole_document() {
    let text = "y".repeat(4321);
    let cs = chunk(&text, params(300, 50));
    assert_eq!(cs.first().unwrap().char_start, 0);
    assert_eq!(cs.last().unwrap().char_end, text.chars().count());
    // Indices are dense + ordered.
    for (i, c) in cs.iter().enumerate() {
        assert_eq!(c.index as usize, i);
    }
}

#[test]
fn consecutive_chunks_overlap_by_the_window() {
    let text = "z".repeat(2000);
    let cs = chunk(&text, params(500, 100));
    assert!(cs.len() >= 2);
    // The next chunk starts `overlap` chars before the previous chunk's end
    // (a hard-cut corpus has no separators, so overlap is exact).
    for w in cs.windows(2) {
        assert_eq!(w[1].char_start, w[0].char_end - 100);
    }
}

#[test]
fn prefers_paragraph_then_line_then_sentence_then_word() {
    // A window where a paragraph break, a line break, a sentence break and a word
    // break all fit: the chunker must cut at the HIGHEST-priority one (paragraph).
    let text = "aaa\n\nbbb\nccc. ddd eee fff ggg hhh iii jjj kkk lll mmm nnn ooo";
    let cs = chunk(text, params(30, 5));
    // The first chunk ends just after the paragraph break "\n\n" (greedy: the
    // latest high-priority separator within the window).
    assert!(cs[0].text.ends_with("\n\n"), "got: {:?}", cs[0].text);
}

#[test]
fn breaks_on_word_boundary_when_no_higher_separator() {
    let text = "alpha bravo charlie delta echo foxtrot golf hotel";
    let cs = chunk(text, params(18, 4));
    // No newlines/sentences → break after a space; a chunk never ends mid-word.
    for c in &cs[..cs.len() - 1] {
        assert!(
            c.text.ends_with(' '),
            "non-final chunk should end on a word boundary: {:?}",
            c.text
        );
    }
}

#[test]
fn deterministic_across_repeated_calls() {
    let text = "The quick brown fox. Jumps over.\n\nThe lazy dog. Runs away fast today.";
    let a = chunk(text, params(25, 6));
    let b = chunk(text, params(25, 6));
    assert_eq!(a, b);
}

#[test]
fn never_splits_a_multibyte_codepoint() {
    // Mixed scripts + emoji: sizes are in chars, so chunk text is always valid
    // UTF-8 and round-trips through char boundaries.
    let text = "café déjà vu — 日本語のテキスト 😀😀😀 résumé naïve coöperate";
    for c in chunk(text, params(8, 2)) {
        // Re-decoding is implicit (String), but assert the range is char-aligned.
        let chars: Vec<char> = text.chars().collect();
        let slice: String = chars[c.char_start..c.char_end].iter().collect();
        assert_eq!(c.text, slice);
    }
}

#[test]
fn overlap_clamped_below_max_guarantees_progress() {
    // overlap >= max would stall; it is clamped to max-1 so the cursor advances.
    let text = "w".repeat(1000);
    let cs = chunk(&text, params(10, 999));
    assert!(cs.len() >= 2);
    assert_eq!(cs.last().unwrap().char_end, 1000);
}

#[test]
fn max_chars_zero_is_coerced_to_one() {
    let cs = chunk("abc", params(0, 0));
    // Every chunk is 1 char; covers the whole doc.
    assert_eq!(cs.len(), 3);
    assert_eq!(cs.last().unwrap().char_end, 3);
}
