//! In-crate Jaro-Winkler similarity (the score ladder's rung 2).
//!
//! ~60 lines of pure, total code instead of a new external dependency
//! (`strsim` is not in the tree; Golden Rule 6 weighs a dep against the
//! surface it saves — here it saves none worth a deny/lockfile pass). The
//! float exists only TRANSIENTLY inside the scorer: `score.rs` projects it to
//! integer basis points with `floor` before anything compares or returns it.

/// Jaro similarity over codepoint slices, in `[0, 1]`.
#[allow(clippy::cast_precision_loss)] // string lengths ≪ 2^52 — exact in f64
fn jaro(a: &[char], b: &[char]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let window = (a.len().max(b.len()) / 2).saturating_sub(1);
    let mut b_taken = vec![false; b.len()];
    let mut matches: Vec<char> = Vec::new();

    for (i, ca) in a.iter().enumerate() {
        let lo = i.saturating_sub(window);
        let hi = (i + window + 1).min(b.len());
        for j in lo..hi {
            if !b_taken[j] && b[j] == *ca {
                b_taken[j] = true;
                matches.push(*ca);
                break;
            }
        }
    }
    if matches.is_empty() {
        return 0.0;
    }

    // Transpositions: compare the match sequences from both sides.
    let mut b_matches: Vec<char> = Vec::with_capacity(matches.len());
    for (j, taken) in b_taken.iter().enumerate() {
        if *taken {
            b_matches.push(b[j]);
        }
    }
    let transpositions = matches
        .iter()
        .zip(b_matches.iter())
        .filter(|(x, y)| x != y)
        .count();

    let m = matches.len() as f64;
    let t = (transpositions / 2) as f64;
    (m / a.len() as f64 + m / b.len() as f64 + (m - t) / m) / 3.0
}

/// Jaro-Winkler similarity in `[0, 1]`: Jaro boosted by up to 4 chars of
/// common prefix (the standard `p = 0.1` scaling).
///
/// **ADVISORY ONLY (SN-8).** A similarity score orders discovery candidates;
/// it must NEVER be an authorization input — the exact-equality
/// `tool_grants` gate in [`lower_to_workflow_def`](crate::lower_to_workflow_def)
/// is the sole authority path.
#[must_use]
#[allow(clippy::cast_precision_loss)] // prefix ≤ 4 — exact in f64
pub fn jaro_winkler(a: &str, b: &str) -> f64 {
    let ac: Vec<char> = a.chars().collect();
    let bc: Vec<char> = b.chars().collect();
    let j = jaro(&ac, &bc);
    let prefix = ac
        .iter()
        .zip(bc.iter())
        .take(4)
        .take_while(|(x, y)| x == y)
        .count() as f64;
    j + prefix * 0.1 * (1.0 - j)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The published reference vectors (Winkler 1990 / the strsim test set).
    #[test]
    fn published_vectors() {
        let close = |x: f64, y: f64| (x - y).abs() < 1e-4;
        assert!(close(jaro_winkler("MARTHA", "MARHTA"), 0.9611));
        assert!(close(jaro_winkler("DIXON", "DICKSONX"), 0.8133));
        assert!(close(jaro_winkler("identical", "identical"), 1.0));
        assert!(close(jaro_winkler("abc", "xyz"), 0.0));
        assert!(close(jaro_winkler("", ""), 1.0));
        assert!(close(jaro_winkler("a", ""), 0.0));
    }

    #[test]
    fn symmetric_and_bounded() {
        let pairs = [
            ("kortecx", "kortex"),
            ("search", "résearch"),
            ("खोज", "खोजना"),
        ];
        for (a, b) in pairs {
            let ab = jaro_winkler(a, b);
            let ba = jaro_winkler(b, a);
            assert!((ab - ba).abs() < 1e-12, "symmetry for {a}/{b}");
            assert!((0.0..=1.0).contains(&ab), "bounds for {a}/{b}");
        }
    }
}
