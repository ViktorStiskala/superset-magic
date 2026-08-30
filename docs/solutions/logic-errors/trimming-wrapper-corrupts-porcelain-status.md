---
title: A blanket `.trim()` in a convenience wrapper corrupts `git status --porcelain`
date: 2026-08-30
category: logic-errors
module: git
problem_type: logic_error
component: tooling
severity: medium
symptoms:
  - "The first line of `git status --porcelain` parsed with every field shifted one column left"
  - "A file modified only in the worktree (status ` M`) read as if it were staged (`M `), and its path lost its first character"
  - "Only the FIRST line was wrong, so a multi-file test could pass while a single-file case failed"
root_cause: shared_helper_normalizes_parse_sensitive_output
resolution_type: code_fix
tags:
  - git
  - parsing
  - porcelain
  - whitespace
  - convenience-wrapper
  - plugin
  - commit-nudge
---

# A blanket `.trim()` in a convenience wrapper corrupts `git status --porcelain`

## Problem

Every git call in this crate goes through one of three helpers in
`src/git/mod.rs`: `git_raw` (returns the raw `Output`), and the two one-liners
on top of it, `git` and `git_optional`. The latter two exist because almost
every probe wants a single value with the trailing newline gone:

```rust
Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
```

That is right for `git rev-parse --show-toplevel` or `git symbolic-ref --short
HEAD`. It is wrong for anything where a **leading** character carries meaning.

The plugin's commit nudge needs to know whether a checklist file is staged, has
unstaged worktree edits, or is untracked, so it reads `git status --porcelain`.
Porcelain v1's short format is two status columns – index state, then worktree
state – a space, then the path:

```plaintext
 M docs/actions/2026-08-rollout.checklist.json
?? docs/actions/2026-08-new.checklist.json
```

The first column is a **literal space** when a file is modified in the worktree
but not staged, which is exactly the case the nudge exists to detect. Routing
that output through `git()` trims the whole string, so the leading space of the
FIRST line disappears and every field on it shifts one column left:

```plaintext
raw:      " M docs/actions/rollout.checklist.json"
trimmed:  "M docs/actions/rollout.checklist.json"
parsed:   code = "M "   path = "ocs/actions/rollout.checklist.json"
```

The status is now read as "staged, clean worktree" – the opposite of the truth –
and the path has lost its first character, so it matches nothing. The nudge
stays silent precisely when it should fire.

## Symptoms

- A checklist with unstaged edits produces no commit nudge; a staged one might.
- Paths from the first porcelain line are short by one character and never match
  a candidate.
- Subsequent lines parse correctly, because `.trim()` only touches the ends of
  the whole string. So a fixture with two or more changed files can pass while
  the one-file case – the common case – is broken.

## What Didn't Work

**Reusing `git()` because it was already there.** The helper is genuinely the
right default: it is used by a dozen probes and each of them wants the trailing
newline gone. Nothing about the call site `git(&["status", "--porcelain", ...])`
looks dangerous.

**Trimming per line instead.** `.trim()` on each line has the same defect, just
applied uniformly instead of only to the first: it eats the leading status
column on every worktree-only modification.

**Compensating in the parser** (detecting a one-character-short line and
re-inserting the space) would be guessing at the missing column from the data
that lost it, and would misfire on a path that legitimately starts where the
status column ended.

## Solution

Parse-sensitive output goes to `git_raw`, and the parser splits on line
boundaries only. `src/git/mod.rs::status_porcelain`:

```rust
pub fn status_porcelain(repo_root: &Path, pathspecs: &[&str]) -> Result<Vec<(String, PathBuf)>> {
    if pathspecs.is_empty() {
        return Ok(Vec::new());   // an empty pathspec would list the whole repo
    }
    let mut args: Vec<&str> = vec!["status", "--porcelain", "--"];
    args.extend_from_slice(pathspecs);
    // NOT the shared `git` helper: its blanket `.trim()` would eat the leading
    // space of the FIRST line, and porcelain's index column is a literal space
    // for a worktree-only modification.
    let out = git_raw(&args, Some(repo_root))?;
    ...
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let code = line.get(0..2)?.to_string();
            let path = line.get(3..)?;
            (!path.is_empty()).then(|| (code, PathBuf::from(path)))
        })
        .collect())
}
```

`str::lines()` is the load-bearing choice alongside `git_raw`: it splits on line
boundaries and strips a `\r` before an `\n`, but it never touches a line's own
leading or trailing content the way `.trim()` on the whole string does. The
non-zero-exit handling is duplicated from `git()` deliberately – that part is
two lines, and inheriting it would mean inheriting the trim.

The reason is recorded as a comment at the call site, because the next person to
see a bespoke `git_raw` call in a file full of `git()` calls will otherwise
"simplify" it back.

## Why This Works

The bug was a shared helper applying a transformation that is correct for its
usual input and destructive for this one. Removing the transformation from this
one call site is the whole fix: `git_raw` returns exactly the bytes git wrote,
and the parser indexes fixed columns (`0..2` for the code, `3..` for the path)
that are only meaningful in untouched output.

Keeping `git()`'s trim for everyone else is right – inverting the default so
that every caller trims for itself would trade one silent bug for a dozen
opportunities to forget.

## Prevention

**Lesson – parse-sensitive command output must not go through a trimming or
normalizing convenience wrapper.** Before reusing a shared `run_command`-style
helper, ask what it does to the bytes and whether any column, leading
character, or blank line carries meaning. Fixed-column formats (`git status
--porcelain`, `git check-ignore -v`, `ls -l`), anything NUL-separated, and
anything where an empty field is distinct from an absent one all need the raw
output.

Two habits catch it:

- **Test the single-item case.** This defect only corrupted the first line, so a
  fixture with several changed files hides it completely. Any parser over
  line-oriented output deserves a one-line fixture.
- **Test the space-prefixed case explicitly.** Here that means a file modified
  in the worktree only (` M`), not merely a staged one (`M `) or an untracked
  one (`??`) – the two that happen not to start with a space.

## Related Issues

- The consumer: the `PreToolUse[Bash]` commit nudge in
  `src/plugin/hook/pre_tool_use.rs` (`staleness`), which distinguishes a staged
  checklist from one with unstaged edits.
- The same "raw bytes, not a convenience wrapper" reasoning drives
  `git::parse_ls_files_z`, which reads NUL-separated `git ls-files -z` output.
- Sibling learning from the same run:
  [unlink-is-not-an-exclusive-claim.md](./unlink-is-not-an-exclusive-claim.md).
