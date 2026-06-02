//! Verified offline schema migration (IMP-2, M2.x-E).
//!
//! [`kx_journal::migrate_to`] performs the byte-level rewrite of an older journal
//! into a fresh current-version one. This module adds the **trust-but-verify**
//! layer the enterprise upgrade story needs: after rewriting, fold *both* the
//! source (read-only, up-converting) and the destination and refuse unless their
//! committed-facts product digests are byte-identical. The product identity digest
//! is the durability law — a migration that changed it would be a bug, and this
//! catches it before the destination is trusted for resume-and-append.

use std::path::Path;

use kx_journal::{migrate_to, MigrationReport, ReplayJournal, SqliteJournal};

use crate::digest::digest_journal;
use crate::error::RuntimeError;

/// Migrate the journal at `src` into a fresh current-version journal at `dst`,
/// then verify the rewrite preserved the run's product identity.
///
/// Returns the [`MigrationReport`] on success. Returns
/// [`RuntimeError::MigrationVerificationFailed`] if the up-converted source and
/// the migrated destination fold to different committed-facts digests (a
/// migration bug — the destination is not trusted). `src` is never modified;
/// `dst` is written atomically by [`migrate_to`].
pub fn migrate_and_verify(
    src: impl AsRef<Path>,
    dst: impl AsRef<Path>,
) -> Result<MigrationReport, RuntimeError> {
    let src = src.as_ref();
    let dst = dst.as_ref();

    let report = migrate_to(src, dst)?;

    // Fold both sides and compare the committed-facts product digest. The source
    // is read through the up-converting ReplayJournal; the destination through the
    // strict current-version open(). Identity must be invariant across migration.
    let src_digest = digest_journal(&ReplayJournal::open(src)?)?;
    let dst_digest = digest_journal(&SqliteJournal::open(dst)?)?;

    if src_digest != dst_digest {
        return Err(RuntimeError::MigrationVerificationFailed {
            from_version: report.from_version,
            src_digest: src_digest.to_hex(),
            dst_digest: dst_digest.to_hex(),
        });
    }

    Ok(report)
}
