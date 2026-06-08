//! M6.2 (D78) — **run metadata as a fold over the journal**: observability for
//! the planner, derived purely from journaled facts.
//!
//! When the planner proposes a plan, it benefits from knowing what already ran —
//! which runs were registered, which recipes were committed, how many committed /
//! failed / were repudiated. [`fold_run_metadata`] derives exactly that from the
//! journal in a **single, flat-per-entry pass** (reusing the incremental-fold
//! discipline; no `O(n²)` rescan). It is an **additive read path**: it never
//! touches the truth fold, the projection digest, or identity — so the canonical
//! product digest is unchanged.
//!
//! **Only journaled facts** are surfaced — instance ids ([`JournalEntry::RunRegistered`]),
//! recipe fingerprints (`recipe_fingerprint` + each committed `mote_def_hash`),
//! and per-outcome counts. **No verdict `is_valid` and no confidence** are read
//! here: a verdict needs the content store (a forward enrichment), and confidence
//! does not exist as a journaled fact. When/if a non-authoritative rating signal
//! is added, it is **steering-only — never the promotion gate** (D77).

use std::collections::BTreeSet;

use kx_journal::{Journal, JournalEntry, JournalError, INSTANCE_ID_LEN};

/// One registered run, derived from its [`JournalEntry::RunRegistered`] fact:
/// the run's identity (`instance_id`), the recipe it was registered for, the
/// fact's seq (ordering + a pagination cursor), and its wall-clock timestamp.
///
/// The `registered_ts` is **audit-only** — it is excluded from every hash
/// (see [`JournalEntry::RunRegistered`]'s `ts`), so surfacing it is legitimate
/// observability, never identity. This record is purely additive run metadata
/// (UI-2's `ListRuns`); like the rest of this module it is off the truth fold
/// and off the projection digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRecord {
    /// The registered run instance id.
    pub instance_id: [u8; INSTANCE_ID_LEN],
    /// The recipe fingerprint the run was registered for.
    pub recipe_fingerprint: [u8; 32],
    /// The journal seq of the `RunRegistered` fact (monotonic; the newest-first
    /// pagination cursor for `ListRuns`).
    pub registered_seq: u64,
    /// The `RunRegistered` wall-clock timestamp (unix-ms; audit-only, off every
    /// hash). `0` when the writer recorded no clock.
    pub registered_ts: u64,
}

/// A read-only, journal-derived summary of what a run (or a journal of runs) has
/// done so far. Purely additive metadata — never an identity or gate input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunMetadata {
    /// Number of registered runs (`RunRegistered` facts).
    pub runs: usize,
    /// The registered run instance ids, in journal order.
    pub instance_ids: Vec<[u8; INSTANCE_ID_LEN]>,
    /// One [`RunRecord`] per registered run, in journal order (the per-run
    /// identity + recipe + seq + wall-clock `ListRuns` enumerates). Additive in
    /// M-UI2; the existing `instance_ids`/counter fields are unchanged so
    /// [`RunMetadata::summary_bytes`] (the planner's deterministic rendering)
    /// stays byte-identical.
    pub records: Vec<RunRecord>,
    /// Distinct recipe fingerprints seen (`RunRegistered.recipe_fingerprint` +
    /// each committed Mote's `mote_def_hash`). Sorted (a `BTreeSet`) so the
    /// rendered summary is deterministic.
    pub recipe_fingerprints: BTreeSet<[u8; 32]>,
    /// Committed Motes.
    pub committed: usize,
    /// Terminal/failed attempts.
    pub failed: usize,
    /// Repudiated (invalidated) Motes.
    pub repudiated: usize,
}

impl RunMetadata {
    /// A deterministic, model-readable text rendering for the planner's context.
    /// Same metadata ⇒ byte-identical bytes (the recipe set is sorted).
    #[must_use]
    pub fn summary_bytes(&self) -> Vec<u8> {
        use std::fmt::Write as _;
        let mut s = String::new();
        // `write!` to a String is infallible; ignore the Result (no unwrap/expect).
        let _ = writeln!(
            s,
            "runs={} committed={} failed={} repudiated={} distinct_recipes={}",
            self.runs,
            self.committed,
            self.failed,
            self.repudiated,
            self.recipe_fingerprints.len()
        );
        for fp in &self.recipe_fingerprints {
            s.push_str("recipe ");
            for b in fp {
                let _ = write!(s, "{b:02x}");
            }
            s.push('\n');
        }
        s.into_bytes()
    }
}

/// Incremental run-metadata accumulator — apply one [`JournalEntry`] at a time
/// (so a caller already folding the journal can piggy-back), then [`finish`].
///
/// [`finish`]: RunMetadataFold::finish
#[derive(Debug, Default)]
pub struct RunMetadataFold {
    md: RunMetadata,
}

impl RunMetadataFold {
    /// A fresh, empty accumulator.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one journal entry into the running metadata. Constant work per entry
    /// (a counter bump + at most one set insert) — flat-per-entry by construction.
    pub fn apply(&mut self, entry: &JournalEntry) {
        match entry {
            JournalEntry::RunRegistered {
                instance_id,
                recipe_fingerprint,
                ts,
                seq,
            } => {
                self.md.runs += 1;
                self.md.instance_ids.push(*instance_id);
                self.md.recipe_fingerprints.insert(*recipe_fingerprint);
                // Additive (M-UI2): keep the per-run record (identity + recipe +
                // seq + audit-only wall-clock) so `ListRuns` can enumerate +
                // paginate. Off the truth fold and the digest (module doc).
                self.md.records.push(RunRecord {
                    instance_id: *instance_id,
                    recipe_fingerprint: *recipe_fingerprint,
                    registered_seq: *seq,
                    registered_ts: *ts,
                });
            }
            JournalEntry::Committed { mote_def_hash, .. } => {
                self.md.committed += 1;
                self.md
                    .recipe_fingerprints
                    .insert(*mote_def_hash.as_bytes());
            }
            JournalEntry::Failed { .. } => self.md.failed += 1,
            JournalEntry::Repudiated { .. } => self.md.repudiated += 1,
            // Proposed / EffectStaged / RunVersionsResolved / DigestSealed carry no
            // planner-relevant metadata for M6.2.
            _ => {}
        }
    }

    /// Consume the accumulator, yielding the [`RunMetadata`].
    #[must_use]
    pub fn finish(self) -> RunMetadata {
        self.md
    }
}

/// Fold a whole journal into [`RunMetadata`] in one flat-per-entry pass.
///
/// # Errors
///
/// Propagates any [`JournalError`] from reading the journal.
pub fn fold_run_metadata(journal: &dyn Journal) -> Result<RunMetadata, JournalError> {
    let current = journal.current_seq()?;
    let mut fold = RunMetadataFold::new();
    for entry in journal.read_entries_by_seq(1..(current + 1))? {
        fold.apply(&entry);
    }
    Ok(fold.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kx_journal::InMemoryJournal;
    use kx_mote::MoteDefHash;
    use smallvec::SmallVec;

    fn run_registered(instance: u8, recipe: u8) -> JournalEntry {
        run_registered_at(instance, recipe, 0, 0)
    }

    fn run_registered_at(instance: u8, recipe: u8, ts: u64, seq: u64) -> JournalEntry {
        JournalEntry::RunRegistered {
            instance_id: [instance; INSTANCE_ID_LEN],
            recipe_fingerprint: [recipe; 32],
            ts,
            seq,
        }
    }

    fn committed(mote: u8, def: u8) -> JournalEntry {
        JournalEntry::Committed {
            mote_id: kx_mote::MoteId::from_bytes([mote; 32]),
            idempotency_key: [mote; 32],
            seq: 0,
            nondeterminism: kx_mote::NdClass::Pure,
            result_ref: kx_content::ContentRef::from_bytes([mote; 32]),
            parents: SmallVec::new(),
            warrant_ref: kx_content::ContentRef::from_bytes([0; 32]),
            mote_def_hash: MoteDefHash::from_bytes([def; 32]),
        }
    }

    #[test]
    fn folds_runs_recipes_and_outcomes() {
        let j = InMemoryJournal::new();
        j.append(run_registered(0x01, 0xAA)).unwrap();
        j.append(committed(0x10, 0xAA)).unwrap(); // same recipe as the run
        j.append(committed(0x11, 0xBB)).unwrap(); // distinct recipe
        let md = fold_run_metadata(&j).unwrap();
        assert_eq!(md.runs, 1);
        assert_eq!(md.committed, 2);
        assert_eq!(md.failed, 0);
        assert_eq!(md.instance_ids, vec![[0x01; INSTANCE_ID_LEN]]);
        // {0xAA (run + commit), 0xBB (commit)} = 2 distinct recipes.
        assert_eq!(md.recipe_fingerprints.len(), 2);
    }

    #[test]
    fn summary_is_deterministic_and_sorted() {
        let mut a = RunMetadataFold::new();
        a.apply(&committed(0x10, 0xBB));
        a.apply(&committed(0x11, 0xAA));
        let mut b = RunMetadataFold::new();
        b.apply(&committed(0x21, 0xAA));
        b.apply(&committed(0x20, 0xBB));
        // Different insertion order, same recipe set ⇒ identical sorted summary
        // lines (the `recipe …` lines are sorted; the count lines match).
        let sa = a.finish().summary_bytes();
        let sb = b.finish().summary_bytes();
        assert_eq!(sa, sb);
        // The 0xAA recipe sorts before 0xBB in the rendering.
        let text = String::from_utf8(sa).unwrap();
        let aa = text.find("recipe aaaa").unwrap();
        let bb = text.find("recipe bbbb").unwrap();
        assert!(aa < bb, "recipes render in sorted order");
    }

    #[test]
    fn empty_journal_is_empty_metadata() {
        let j = InMemoryJournal::new();
        let md = fold_run_metadata(&j).unwrap();
        assert_eq!(md, RunMetadata::default());
        assert!(md.records.is_empty(), "no runs ⇒ no per-run records");
    }

    // --- UI-2: the additive per-run `records` fold (the `ListRuns` backing) ---

    #[test]
    fn records_capture_identity_recipe_seq_and_ts_per_run() {
        let mut fold = RunMetadataFold::new();
        fold.apply(&run_registered_at(0x01, 0xAA, 1_000, 7));
        // A committed Mote between runs must not perturb the per-run records.
        fold.apply(&committed(0x10, 0xAA));
        fold.apply(&run_registered_at(0x02, 0xBB, 2_000, 19));
        let md = fold.finish();

        assert_eq!(md.runs, 2);
        assert_eq!(md.records.len(), 2, "one record per RunRegistered");
        // Journal order preserved (the gateway sorts newest-first for the wire).
        assert_eq!(
            md.records[0],
            RunRecord {
                instance_id: [0x01; INSTANCE_ID_LEN],
                recipe_fingerprint: [0xAA; 32],
                registered_seq: 7,
                registered_ts: 1_000,
            }
        );
        assert_eq!(
            md.records[1],
            RunRecord {
                instance_id: [0x02; INSTANCE_ID_LEN],
                recipe_fingerprint: [0xBB; 32],
                registered_seq: 19,
                registered_ts: 2_000,
            }
        );
        // The records track identity 1:1 with `instance_ids` (no double count).
        assert_eq!(
            md.records.iter().map(|r| r.instance_id).collect::<Vec<_>>(),
            md.instance_ids
        );
    }

    #[test]
    fn records_preserve_journal_order_and_use_the_journal_assigned_seq() {
        // `seq` is JOURNAL-authoritative (the journal's `set_seq` overwrites the
        // entry's seq to its 1-based append position); `ts` is the writer's clock
        // and is preserved. `ListRuns` paginates on the journal seq, so the fold
        // must surface that, not a client-injected value.
        let j = InMemoryJournal::new();
        for i in 0..16u8 {
            j.append(run_registered_at(i, i, u64::from(i) * 10, 0))
                .unwrap();
        }
        let md = fold_run_metadata(&j).unwrap();
        assert_eq!(md.records.len(), 16);
        for (i, rec) in md.records.iter().enumerate() {
            let i = u8::try_from(i).unwrap();
            assert_eq!(rec.instance_id, [i; INSTANCE_ID_LEN]);
            assert_eq!(
                rec.registered_seq,
                u64::from(i) + 1,
                "registered_seq is the 1-based journal seq"
            );
            assert_eq!(rec.registered_ts, u64::from(i) * 10, "ts is preserved");
        }
        // Strictly increasing seqs ⇒ a valid newest-first pagination cursor.
        for w in md.records.windows(2) {
            assert!(w[0].registered_seq < w[1].registered_seq);
        }
    }

    #[test]
    fn summary_bytes_unchanged_by_the_additive_records() {
        // The planner's deterministic rendering must stay byte-identical to the
        // pre-UI-2 contract — `records` is additive, never in `summary_bytes`.
        let mut fold = RunMetadataFold::new();
        fold.apply(&run_registered_at(0x01, 0xAA, 1_234, 1));
        fold.apply(&committed(0x10, 0xBB));
        let md = fold.finish();
        let text = String::from_utf8(md.summary_bytes()).unwrap();
        assert_eq!(
            text,
            "runs=1 committed=1 failed=0 repudiated=0 distinct_recipes=2\n\
             recipe aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n\
             recipe bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n",
        );
    }
}
