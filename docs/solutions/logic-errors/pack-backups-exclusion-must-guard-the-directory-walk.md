---
title: A leaf-only path exclusion leaks secrets when a later step walks the live tree (pack)
date: 2026-07-17
category: logic-errors
module: pack
problem_type: logic_error
component: security
severity: high
symptoms:
  - "ss-magic pack archived recovered secret bytes from .superset/backups/ despite a filter meant to exclude them"
  - "A magic.json pattern matching a directory (a bare `.superset`, or a broad `**` / `.*`) walked the live filesystem and packed the backups subtree"
  - "The whole test suite passed with the leak present because no test drove a directory match at or above .superset/backups"
root_cause: incomplete_guard
resolution_type: code_fix
tags:
  - pack
  - secrets
  - gitignore
  - backups
  - append_dir_all
  - walkdir
  - exclusion-filter
  - directory-recursion
---

# A leaf-only path exclusion leaks secrets when a later step walks the live tree (pack)

## Problem

Reverse sync writes timestamped backups of overwritten secret files under a
gitignored `.superset/backups/<ts>/{worktree,main}/…`. To stop `ss-magic pack`
from ever archiving one of those recovered secrets, a filter was added to the
flat list of pattern matches:

```rust
rels.retain(|r| !is_pack_archive_rel(r) && !is_repo_root_rel(r) && !under_backups_dir(r));
```

That looks sufficient, and a comment even claimed "a recovered secret is never
packed." It is not sufficient. `under_backups_dir(rel)` needs BOTH a `.superset`
and a `backups` path component, so it only matches LEAF paths already inside the
backups tree. A directory match that is an **ancestor** of `.superset/backups`
survives the filter, and `write_archive` then hands it to `tar`'s
`append_dir_all`, which walks the **live filesystem** and archives every
descendant — including `.superset/backups/**`.

A literal `magic.json` pattern of `.superset` (valid, always exists) expands to
the single rel `.superset`; `under_backups_dir(".superset")` is `false`, so it is
retained; `append_dir_all(".superset", …)` packs the backups. Broad globs
(`**`, `.*`) hit the same hole via the bare `.superset` component.

## Symptoms

- An archive produced by `ss-magic pack` contains files under
  `.superset/backups/<ts>/…` — recovered secret bytes that a later `pack` shared
  or uploaded.
- Only reproducible when a pattern resolves to a directory at/above
  `.superset/backups`; specific patterns like `.env` / `**/.dev.vars` never
  trigger it, which is why it hid.

## What Didn't Work

The flat `rels.retain(... && !under_backups_dir(r))` filter. It correctly drops a
LEAF match (`**/.env` also matching `.superset/backups/…/.env`), so it read as
complete, and the accompanying comment overclaimed total protection. But the
exclusion lived at the wrong layer: the retain list is not where the file set is
finally enumerated. `append_dir_all` re-enumerates the tree from disk, blind to
the retain filter. The suite passed because no test drove a directory match at or
above the backups tree — the exact shape the guard missed.

## Solution

Enforce the exclusion at the point of enumeration — the directory walk itself —
not on the flat match list. Replace the blind `append_dir_all` with a guarded
`walkdir` that prunes the backups subtree wherever it appears, keyed on each
entry's repo-relative path:

```rust
fn append_dir_excluding_backups<W: Write>(
    builder: &mut tar::Builder<W>, root: &Path, rel: &Path, abs: &Path,
) -> Result<()> {
    let walker = WalkDir::new(abs).follow_links(false).into_iter()
        .filter_entry(|e| match e.path().strip_prefix(root) {
            Ok(r) => !crate::sync::reverse_sync::under_backups_dir(r), // prunes the subtree
            Err(_) => true,
        });
    for entry in walker {
        let entry = entry?;
        let name = entry.path().strip_prefix(root).unwrap_or(rel);
        let ft = entry.file_type();
        if ft.is_dir() { builder.append_dir(name, entry.path())?; }
        else if ft.is_symlink() || ft.is_file() { builder.append_path_with_name(entry.path(), name)?; }
    }
    Ok(())
}
```

`filter_entry` returning `false` for a directory PRUNES it from the walk, so
`.superset/backups/` is never descended, no matter how the ancestor match entered
`rels`. The flat `under_backups_dir` retain is kept too (it cheaply drops leaf
matches before the walk), but it is no longer the only line of defense.

## Why This Works

Secret exclusion has to be enforced where the file set is finally materialized.
`retain` filters the list of *match roots*; `append_dir_all` re-derives the actual
file set by walking disk. Any filter applied only to the former is bypassed by the
latter. Moving the guard into the walk (`filter_entry` prune) makes it
enumeration-time, so it holds regardless of which broad/ancestor pattern produced
the directory root.

## Prevention

**Lesson — an exclusion filter must live at the point of final enumeration, not
on an upstream list that a later step re-expands.** Whenever a filtered set is
handed to something that re-walks the filesystem (`append_dir_all`,
`copy_dir_recursive`, `WalkDir`, a shell `cp -r`), the exclusion has to be applied
INSIDE that walk. A comment claiming "X is never included" is a red flag unless
the guard sits on the same layer that enumerates.

Add the regression test in the shape that hid the bug — a **directory** match at
the ancestor of the excluded subtree, not just a leaf:

```rust
#[test]
fn excludes_backups_subtree_from_ancestor_directory_match() {
    let repo = init_repo();
    write_magic(repo.path(), &[".superset"]);          // directory match, ancestor of backups
    write_file(repo.path(), ".superset/config.json", "{}\n");
    write_file(repo.path(), ".superset/backups/20260101-000000/main/.env", "RECOVERED=1\n");
    pack_core(repo.path(), |_| {}).unwrap();
    let entries = archive_entries(repo.path());
    assert!(entries.contains(".superset/config.json"));                 // normal files still pack
    assert!(!entries.iter().any(|e| e.contains(".superset/backups/"))); // the subtree is pruned
}
```

## Related Issues

- The complementary secret gate learned the same run: [secret-gate-positive-tracked-determination-fail-closed.md](./secret-gate-positive-tracked-determination-fail-closed.md)
- `sync/reverse_sync.rs::under_backups_dir` is the shared predicate; `pack.rs::write_archive` is the enumeration site that must honor it.
