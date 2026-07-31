# Frozen REAL state dirs

These directories were **written by released `kx` binaries**. They were not
constructed in a test, and that is the entire point.

## Why they exist

`schema_evolution.rs` used to build its only old-version fixture by writing a
CURRENT journal and downgrading it with raw SQL. That fixture can only contain
what today's writer knows how to produce, so it cannot catch the class of defect
where an old binary wrote something today's reader does not expect.

Two boot-path defects shipped past 469 green suites for exactly this reason:
nothing in the tree had a pre-existing `bodies.db` or a pre-existing sidecar. The
suites were green because there was nothing old to be wrong about.

A synthesized approximation would be worse than nothing here — it would restore
the confidence without restoring the coverage.

## Provenance

| dir | binary | sha256 (from the release's own `checksums.txt`) |
| --- | --- | --- |
| `v0.1.1/` | `kx-aarch64-apple-darwin` @ tag `v0.1.1` | `fa089f2b864668d6399f64da020504cc537b256f3311230ea9af89a7ca291f8b` |
| `v0.2.0-rc.1/` | `kx-aarch64-apple-darwin` @ tag `v0.2.0-rc.1` | `50b98d8799336d7b122cbbf75ab0fa54762578201c375c7e4d63dccbec64b64a` |

Downloaded from the GitHub release, verified against the published
`checksums.txt` **before being executed**, then run as an ordinary `kx serve` on
dedicated loopback ports. Preferring the published artifact over a local rebuild
of the tag is deliberate: a rebuild is a re-derivation, and what makes these
fixtures worth having is that a *released* binary wrote them.

## What each contains

**`v0.1.1/`** — journal `schema_version = 8`, 3 entries, one real committed
`kx/recipes/echo` run and its content blob. The catalog dir has only the stores
that existed then (`bodies`, `capture`, `catalog`, `grants`, `members`,
`versions`); there were no App / trigger / branch verbs to author with.

**`v0.2.0-rc.1/`** — journal `schema_version = 16`, 3 entries, plus a genuinely
populated catalog authored through the released CLI:

| store | rows |
| --- | --- |
| `apps.db` | 1 App (`fixtures/frozen/reporter`) |
| `triggers.db` | 1 cron trigger targeting that App |
| `branches.db` | 1 branch (`fixtures/frozen/proj`) |
| `bundles.db` | 1 context bundle |
| `tools.db` | 3 rows, under a **`metadata`** table — see below |

### The `tools.db` detail is the most valuable byte in here

`tools.db` in this fixture stamps its version in a table called **`metadata`**,
not `meta`. The previous PR moved `tools.db` under the sidecar upgrade policy and
argued in its CHANGELOG that existing databases would be unaffected *because* the
old opener used a different table name, so the policy finds nothing, takes the
fresh-file arm, and every statement is `CREATE TABLE IF NOT EXISTS`.

That was an argument. This fixture is the artifact it was an argument about.

## Portability

Both directories were scanned for absolute host paths (`/Users/`,
`/private/tmp/`, and the literal capture directory) before being committed —
**zero hits**, which is what lets a macOS-captured fixture boot on the Linux
runner. Re-run that scan if you ever recapture: a fixture that only works on the
machine that made it is a fixture that gets quietly disabled later.

WAL/SHM sidecars were checkpointed with `PRAGMA wal_checkpoint(TRUNCATE)` and
removed, so each `.db` is self-contained.

## Recapturing

Do not hand-edit these files. To refresh or add a tag:

1. `gh release download <tag> -R Kortecx/kortecx -p 'kx-<target>' -p 'checksums.txt'`
2. verify the sha256 against `checksums.txt` **before** running it
3. `kx serve --journal … --content … --catalog-dir … --listen 127.0.0.1:<free>`
   on ports nothing else is using
4. author entities through that binary's OWN CLI — never by writing rows
5. stop the serve, checkpoint the WALs, scan for host paths, update this file

A new tag is a new directory plus a row in the table above. Deleting one is a
decision to stop testing that upgrade path, and should be argued for.
