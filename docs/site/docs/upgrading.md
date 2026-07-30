---
id: upgrading
title: Upgrading
sidebar_label: Upgrading
description: What happens to your data when you upgrade kortecx — what is preserved, what is refused, and the one command to run if a new binary will not start.
---

# Upgrading

kortecx keeps what you authored. An upgrade may change how data is stored, but it never
deletes your apps, workflows, branches, triggers, skills or secret names to make itself
fit.

Two rules cover everything on this page:

- **An upgrade preserves.** If a newer release changes a store's shape, the existing file
  is set aside and its contents brought forward.
- **A downgrade refuses.** An older binary cannot know what a newer format meant, so it
  stops and says so rather than starting with an empty or half-read catalog.

## Normal upgrade

Install the new version and start it. Nothing else is required:

```bash
kx serve --dev-allow-local
```

Your state directory (`~/.kortecx`, or `$KX_DATA_DIR`) is carried forward in place. If a
catalog needed reshaping, the previous copy is kept beside it as `<name>.db.v<N>.bak` —
untouched, and safe to delete once you are satisfied. A `kortecx-version` file in the
state directory records the release that last opened it.

## If the server will not start

A new binary can refuse a run journal written by an older one. The message names the
remedy:

```
schema version mismatch: file has 16, this binary expects 17
  — run `kx migrate --journal <path>` to upgrade it
```

Run exactly that:

```bash
kx migrate --journal ~/.kortecx/kx.db
```

The rewrite is **verified, not trusted**: kortecx folds both the old and the new journal
and refuses the migration unless they produce byte-identical results. An upgrade can
never quietly change what your past runs produced. The original is preserved beside the
migrated file.

Useful variants:

| Command | Effect |
| --- | --- |
| `kx migrate --journal <path> --dry-run` | Report what would happen; write nothing. |
| `kx migrate --journal <path> --out <dest>` | Write the upgraded journal elsewhere; leave the source untouched. |
| `kx migrate --journal <path> --json` | Machine-readable report. |

Running it on an already-current journal is a no-op, so it is safe to run defensively
after every upgrade.

## Going back to an older version

Don't, without moving your state directory aside first. An older binary refuses data a
newer one wrote — deliberately, because the alternative is starting up and silently
ignoring what it cannot read. To run an older release, point it at a different
`$KX_DATA_DIR`.

## Before you upgrade

The [CHANGELOG](https://github.com/Kortecx/kortecx/blob/main/CHANGELOG.md) carries an
upgrade note for any release that changes stored formats. Releases that need no action
say nothing, so anything written there is worth reading.

Your state directory is a directory of ordinary files. Copying it is a complete backup.
