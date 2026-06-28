---
title: Reverse sync skipped gitignored secrets (git ls-files --exclude-standard)
date: 2026-06-28
category: logic-errors
module: reverse_sync
problem_type: logic_error
component: tooling
severity: high
symptoms:
  - "Reverse sync reports 'No untracked files match the configured patterns' even when the worktree holds untracked secrets that match the patterns"
  - "Gitignored files (.env, .dev.vars, .superset/magic.local.json) are silently dropped from the candidate set"
  - "The feature is a silent no-op in any real repo that gitignores its secrets"
root_cause: wrong_api
resolution_type: code_fix
tags:
  - reverse-sync
  - git-ls-files
  - gitignore
  - secrets
  - exclude-standard
  - untracked-files
  - test-isolation
---

# Reverse sync skipped gitignored secrets (git ls-files --exclude-standard)

## Problem

`ss-magic`'s reverse sync exists to push git-untracked worktree files — chiefly
secrets like `.env`, `.dev.vars`, and the gitignored `.superset/magic.local.json`
— back into the shared main checkout, since those files never reach main via a
normal git merge. The candidate probe excluded gitignored files, so in any real
repo (where those secrets are gitignored by definition) the candidate set was
always empty and the user saw "No untracked files match the configured
patterns." The feature was a silent no-op for its entire reason to exist.

## Symptoms

- Running reverse sync in a worktree containing gitignored secrets that match
  the `magic.json` patterns prints "No untracked files match the configured
  patterns" and exits without pushing anything.
- The failure is silent: no error, no hint why the candidate set is empty. An
  untracked file that is *not* gitignored (a plain new source file) would show
  up correctly — which is exactly what masked the bug during casual testing.

## What Didn't Work

The unit tests for `compute_candidates` in `src/reverse_sync.rs` *looked* like
they covered the case: they wrote secret-named files (`apps/api/.dev.vars`) into
a temp git repo and asserted those files appeared as candidates. They passed —
because the files were written **without** a matching `.gitignore` entry, so
they were untracked but *not* gitignored. That gave false confidence. The
real-world shape (secrets that are gitignored) was never exercised, so the bug
shipped green. The whole downstream secret-safety apparatus
(`ensure_gitignored_in_main`, which copies a covering `.gitignore` rule into main
so a pushed secret stays ignored) only makes sense if candidates *are*
gitignored — the probe excluding them was internally contradictory.

A second, separate failure surfaced while fixing this: six tests failed on the
**unmodified** codebase on the author's machine because the developer's global
excludes file (`~/.config/git/ignore`) listed `.env` and `.dev.vars`. The test
repos did not suppress global git config, so `git check-ignore` / `git ls-files`
inside them honored those global rules and the assertions became
environment-dependent. Setting `GIT_CONFIG_GLOBAL=/dev/null` on the *test
helper's* git wrapper did **not** fix it — the production code under test shells
out via its own `std::process::Command` invocations that do not inherit the test
wrapper's environment.

## Solution

Two changes in `src/git.rs` plus its call site in `src/reverse_sync.rs`.

**1. Drop `--exclude-standard` from the untracked-files probe.**

Before:
```rust
git(&["ls-files", "--others", "--exclude-standard", "-z"], Some(repo_root))
```

After — the signature gains a pathspec slice and the flag is gone:
```rust
pub fn untracked_files(repo_root: &Path, pathspecs: &[&str]) -> Result<Vec<PathBuf>> {
    let mut args: Vec<&str> = vec!["ls-files", "--others", "-z", "--"];
    args.extend_from_slice(pathspecs);
    let out = git(&args, Some(repo_root))?;
    // ... NUL-split parsing unchanged ...
}
```

`git ls-files --others` with no `--exclude-standard` returns **all** untracked
files — gitignored and not — which is exactly the set reverse sync must search.
`--others` still excludes tracked files (they reach main via merge), and git
lists files, never directory entries.

**2. Scope the probe with pathspecs.**

`compute_candidates` passes the already-matched paths as explicit pathspecs after
`--`, so git does a bounded index lookup on those paths instead of recursively
walking every ignored directory:

```rust
let matched = apply::match_paths(worktree_root, &cfg.files)?;
if matched.is_empty() {
    return Ok(Vec::new());
}
let pathspecs: Vec<&str> = matched.iter().filter_map(|p| p.to_str()).collect();
let untracked = git::untracked_files(worktree_root, &pathspecs)?;
// intersect untracked ∩ matched_set, then is_safe_rel filter ...
```

The retained `matched_set` intersection preserves directory-match semantics: a
directory pathspec expands to its untracked inner files, but only exact paths in
`matched_set` (which holds the directory path, not the inner files) survive, so a
directory pattern still contributes nothing — reverse sync copies single files.

## Why This Works

`--exclude-standard` tells git to apply `.gitignore`, `.git/info/exclude`, and
the global excludes file before reporting untracked files, so gitignored files
are omitted. The files reverse sync pushes are gitignored *by design* (that is
the whole point — they cannot travel via merge), so the intersection
`matched_paths ∩ untracked_files` was guaranteed empty. Removing the flag makes
the probe return the full untracked set, so the intersection is non-empty
whenever matching secrets exist.

The pathspec scoping is a behavior-preserving performance fix. Without it, the
broadened probe forces git to enumerate every untracked path under `target/`,
`node_modules/`, and friends on each reverse sync — and critically, for
*literal* patterns the pattern matcher skips its own tree walk, so the git probe
becomes the **sole** full-tree cost (a multi-second latency hit on large repos).
Passing the matched paths as pathspecs keeps git to an index lookup and verified
(empirically) to still list gitignored files while not descending into unrelated
ignored trees.

## Prevention

**Lesson 1 — Test the realistic shape: gitignore the secret, and assert it.**

A test for a "find untracked secrets" probe that writes a secret *without*
gitignoring it is testing the wrong scenario. Gitignore the file and assert the
precondition so the test cannot pass vacuously on the non-gitignored path the
original bug hid behind:

```rust
write(&wt, ".gitignore", "**/.dev.vars\n");
git_run(&["add", ".gitignore"], &wt);
write_magic(&wt, &["**/.dev.vars"]);
write(&wt, "apps/api/.dev.vars", "SECRET=1\n");

// Precondition: prove it is REALLY gitignored — otherwise the assertion
// below would pass on the shape that shipped the bug.
assert!(git::is_ignored(&wt, Path::new("apps/api/.dev.vars")).unwrap());

let cands = compute_candidates(&wt).unwrap();
assert!(cands.iter().any(|p| p.ends_with("apps/api/.dev.vars")));
```

**Lesson 2 — Neutralize the global excludes file in every git test repo, via repo-local config.**

A developer's `~/.config/git/ignore` (anything in `core.excludesFile`) leaks into
every git command run inside a test repo. Suppress it as **repo-local** config,
not as an environment variable on the test helper's git wrapper — the production
code under test runs its own `std::process::Command` shell-outs that do not
inherit the wrapper's env, whereas repo-local config is observed by *both*:

```rust
/// Call immediately after `git init` in every test-repo helper.
pub fn neutralize_global_excludes(repo_root: &Path) {
    // Repo-local: seen by the test helper AND the production code's own
    // git shell-outs into this repo. /dev/null is an empty excludes source
    // (Unix-only, like the rest of this suite).
    git_run(&["config", "--local", "core.excludesFile", "/dev/null"], repo_root);
}
```

Linked git worktrees share the common config, so setting this on the main repo
also covers worktrees created from it.

## Related Issues

- Originating plan: [docs/plans/2026-06-28-001-fix-reverse-sync-gitignored-candidates-plan.md](../../plans/2026-06-28-001-fix-reverse-sync-gitignored-candidates-plan.md)
- `src/gitignore.rs` — `find_covering_rule` / `ensure_entry` are the downstream
  consumers that assume candidates are gitignored; this fix makes that true in
  practice, not just in theory.
