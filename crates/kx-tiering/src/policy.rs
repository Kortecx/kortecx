//! The simulated memory-pressure budget applied to resident PURE payloads.

/// A simulated memory-pressure budget over the resident PURE payload footprint.
///
/// Only PURE payloads are subject to a budget — they are the sole recomputable
/// class (`mote.md` §6). READ-ONLY-NONDET and WORLD-MUTATING payloads are never
/// counted toward usage and never evicted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TieringBudget {
    /// Keep at most this many resident PURE payload objects.
    MaxObjects(usize),
    /// Keep at most this many resident PURE payload bytes.
    MaxBytes(u64),
}

/// The resident PURE-payload footprint measured during a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ResidentUsage {
    /// Number of resident PURE payload objects.
    pub objects: usize,
    /// Total bytes of resident PURE payloads.
    pub bytes: u64,
}

impl TieringBudget {
    /// `true` when `usage` is within budget — no eviction needed.
    #[must_use]
    pub fn is_satisfied(self, usage: ResidentUsage) -> bool {
        match self {
            TieringBudget::MaxObjects(n) => usage.objects <= n,
            TieringBudget::MaxBytes(b) => usage.bytes <= b,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_objects_under_and_over_budget() {
        let b = TieringBudget::MaxObjects(2);
        assert!(b.is_satisfied(ResidentUsage {
            objects: 0,
            bytes: 999
        }));
        assert!(b.is_satisfied(ResidentUsage {
            objects: 2,
            bytes: 999
        }));
        assert!(!b.is_satisfied(ResidentUsage {
            objects: 3,
            bytes: 0
        }));
    }

    #[test]
    fn max_bytes_under_and_over_budget() {
        let b = TieringBudget::MaxBytes(100);
        assert!(b.is_satisfied(ResidentUsage {
            objects: 999,
            bytes: 0
        }));
        assert!(b.is_satisfied(ResidentUsage {
            objects: 999,
            bytes: 100
        }));
        assert!(!b.is_satisfied(ResidentUsage {
            objects: 0,
            bytes: 101
        }));
    }

    #[test]
    fn zero_budget_is_only_satisfied_when_empty() {
        let objs = TieringBudget::MaxObjects(0);
        assert!(objs.is_satisfied(ResidentUsage::default()));
        assert!(!objs.is_satisfied(ResidentUsage {
            objects: 1,
            bytes: 0
        }));

        let bytes = TieringBudget::MaxBytes(0);
        assert!(bytes.is_satisfied(ResidentUsage::default()));
        assert!(!bytes.is_satisfied(ResidentUsage {
            objects: 0,
            bytes: 1
        }));
    }
}
