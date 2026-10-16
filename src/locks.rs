//! PostgreSQL table-level lock modes and their conflict matrix.

/// Table-level lock modes, weakest first (the order matters: `max` gives the strongest).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LockMode {
    AccessShare,
    RowShare,
    RowExclusive,
    ShareUpdateExclusive,
    Share,
    ShareRowExclusive,
    Exclusive,
    AccessExclusive,
}

impl LockMode {
    /// Whether two transactions can hold these modes on the same table at once.
    pub fn conflicts_with(self, other: LockMode) -> bool {
        use LockMode::*;
        match self {
            AccessShare => other == AccessExclusive,
            RowShare => other >= Exclusive,
            RowExclusive => other >= Share,
            ShareUpdateExclusive => other >= ShareUpdateExclusive,
            Share => other == RowExclusive || (other >= ShareUpdateExclusive && other != Share),
            ShareRowExclusive => other >= RowExclusive,
            Exclusive => other >= RowShare,
            AccessExclusive => true,
        }
    }

    /// Blocks plain `SELECT`s (which take ACCESS SHARE).
    pub fn blocks_reads(self) -> bool {
        self.conflicts_with(LockMode::AccessShare)
    }

    /// Blocks `INSERT`/`UPDATE`/`DELETE` (which take ROW EXCLUSIVE).
    pub fn blocks_writes(self) -> bool {
        self.conflicts_with(LockMode::RowExclusive)
    }

    pub fn as_str(self) -> &'static str {
        use LockMode::*;
        match self {
            AccessShare => "ACCESS SHARE",
            RowShare => "ROW SHARE",
            RowExclusive => "ROW EXCLUSIVE",
            ShareUpdateExclusive => "SHARE UPDATE EXCLUSIVE",
            Share => "SHARE",
            ShareRowExclusive => "SHARE ROW EXCLUSIVE",
            Exclusive => "EXCLUSIVE",
            AccessExclusive => "ACCESS EXCLUSIVE",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LockMode::{self, *};

    const ALL: [LockMode; 8] = [
        AccessShare,
        RowShare,
        RowExclusive,
        ShareUpdateExclusive,
        Share,
        ShareRowExclusive,
        Exclusive,
        AccessExclusive,
    ];

    /// "Conflicting Lock Modes" table from the PostgreSQL docs (Explicit Locking).
    fn documented_conflicts(mode: LockMode) -> Vec<LockMode> {
        match mode {
            AccessShare => vec![AccessExclusive],
            RowShare => vec![Exclusive, AccessExclusive],
            RowExclusive => vec![Share, ShareRowExclusive, Exclusive, AccessExclusive],
            ShareUpdateExclusive => vec![
                ShareUpdateExclusive,
                Share,
                ShareRowExclusive,
                Exclusive,
                AccessExclusive,
            ],
            Share => vec![
                RowExclusive,
                ShareUpdateExclusive,
                ShareRowExclusive,
                Exclusive,
                AccessExclusive,
            ],
            ShareRowExclusive => vec![
                RowExclusive,
                ShareUpdateExclusive,
                Share,
                ShareRowExclusive,
                Exclusive,
                AccessExclusive,
            ],
            Exclusive => vec![
                RowShare,
                RowExclusive,
                ShareUpdateExclusive,
                Share,
                ShareRowExclusive,
                Exclusive,
                AccessExclusive,
            ],
            AccessExclusive => ALL.to_vec(),
        }
    }

    #[test]
    fn conflict_matrix_matches_postgres_docs() {
        for a in ALL {
            let expected = documented_conflicts(a);
            for b in ALL {
                assert_eq!(
                    a.conflicts_with(b),
                    expected.contains(&b),
                    "{} vs {}",
                    a.as_str(),
                    b.as_str()
                );
            }
        }
    }

    #[test]
    fn conflicts_are_symmetric() {
        for a in ALL {
            for b in ALL {
                assert_eq!(a.conflicts_with(b), b.conflicts_with(a));
            }
        }
    }

    #[test]
    fn only_access_exclusive_blocks_reads() {
        for m in ALL {
            assert_eq!(m.blocks_reads(), m == AccessExclusive, "{}", m.as_str());
        }
    }

    #[test]
    fn blocks_writes_set() {
        for m in ALL {
            let expected = matches!(m, Share | ShareRowExclusive | Exclusive | AccessExclusive);
            assert_eq!(m.blocks_writes(), expected, "{}", m.as_str());
        }
    }

    #[test]
    fn as_str_is_postgres_spelling() {
        assert_eq!(AccessExclusive.as_str(), "ACCESS EXCLUSIVE");
        assert_eq!(ShareUpdateExclusive.as_str(), "SHARE UPDATE EXCLUSIVE");
    }

    #[test]
    fn modes_are_ordered_weakest_first() {
        assert!(AccessShare < RowShare && Exclusive < AccessExclusive);
    }
}
