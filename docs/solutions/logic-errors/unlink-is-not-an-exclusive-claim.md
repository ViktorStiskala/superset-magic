---
title: "`unlink` is not an exclusive claim under a real race, and sequential testing hides it"
date: 2026-08-30
category: logic-errors
module: plugin/claim
problem_type: logic_error
component: security
severity: high
symptoms:
  - "A one-shot bypass token intended for exactly the next gated Read would have opened the gate for several concurrent reads"
  - "8 threads racing to unlink one path produced up to 5 successes across 20 trials"
  - "Sequential testing showed exactly the expected single success plus ENOENT, so the bug was invisible in the test suite"
root_cause: non_atomic_claim_primitive
resolution_type: code_fix
tags:
  - plugin
  - concurrency
  - exactly-once
  - atomicity
  - rename
  - unlink
  - one-shot-token
  - test-methodology
---

# `unlink` is not an exclusive claim under a real race, and sequential testing hides it

## Problem

The plugin's Read gate has a one-shot escape hatch: `ss-magic plugin bypass
<FILE>` records a claim, and **exactly the next** gated `Read` of that file goes
through. The same shape backs `expect-artifact`, where taking the pending
declaration is what guarantees a subagent's stop is blocked at most once.

Both are "consume exactly once" problems, and the obvious primitive is delete:

```rust
// Wrong: treat a successful unlink as having won the claim.
match fs::remove_file(&claim) {
    Ok(()) => Some(claim_won()),   // "I deleted it, so it was mine"
    Err(_) => None,                // "someone else got there first"
}
```

The reasoning is that a file can only be deleted once, so the caller that gets
`Ok` is the single winner and everyone else gets `ENOENT`. That is what
sequential testing shows, and it is wrong.

`unlink` is not specified to be an exclusive claim. Concurrent `unlink` calls on
the same path can both return success: the kernel's path resolution and the
directory-entry removal are not one atomic step from the caller's point of view,
and the result is racy in practice, not merely in theory.

Measured on macOS with 8 threads racing to `unlink` one path, repeated 20 times:
**up to 5 threads got `Ok` in the same trial.** So a bypass token meant to admit
one `Read` would have admitted every concurrent read that raced it – a gate
opened wider than the user asked for, silently.

## Symptoms

- A one-shot token is consumed by more than one caller when several hooks fire
  at once (which is normal: hooks on one event run concurrently).
- Nothing errors. The extra consumers each believe they hold the only claim.
- The unit tests pass, because they exercise the claim one caller at a time.

## What Didn't Work

**Deleting and trusting the error.** `fs::remove_file(&path).is_ok()` as the
claim predicate. It reads as obviously exclusive and is not.

**Testing it sequentially.** A test that records a claim, consumes it, then
consumes it again and asserts the second call returns `None` passes against the
broken implementation. Exclusivity is a property about *simultaneous* callers,
so a sequential test cannot observe it at all – it only observes that the file
is gone afterwards, which the broken version also achieves.

**A lock around the delete** would work, but it is a heavier primitive than the
problem needs and adds a failure mode (a held lock, a lock file to clean up) to
a path that must never fail a session.

## Solution

Claim by `rename`, not by `unlink`. `rename` requires its source to exist, and
moving a directory entry to a new name is the single atomic step `unlink` is
not, so exactly one caller can move a given path.

`src/plugin/claim.rs`:

```rust
pub fn take(dir: &Path, path: &Path) -> Option<Claimed> {
    // The landing file is created in the SAME directory as the claim, so the
    // rename never crosses a filesystem boundary (a cross-device rename fails).
    let landing = Builder::new()
        .prefix(".taken-")
        .suffix(".tmp")
        .tempfile_in(dir)
        .ok()?;
    // Exactly one caller's rename can succeed: the others find no source.
    fs::rename(path, landing.path()).ok()?;
    Some(Claimed { landing })
}
```

The winner receives a `Claimed` holding the landing file, so it can still read
the claim's contents; dropping it unlinks the landing file and retires the
claim. Both one-shot stores (`plugin/bypass.rs`, `plugin/expect_artifact.rs`)
go through it, so there is one implementation of the property rather than two.

Re-running the same 8-thread, 20-trial race against the `rename` version gave
**exactly one winner in every trial.**

## Why This Works

`rename(2)` is required to be atomic with respect to other callers, and its
source must exist. Two callers racing to rename the same path therefore cannot
both succeed: whichever the kernel orders second finds no source and fails. The
claim and its consumption are the same syscall, so there is no window between
"check that it exists" and "take it" for a second caller to slip into.

Renaming into the same directory matters twice over: a cross-device rename would
fail outright, and a landing file elsewhere could be on a filesystem with
different semantics.

## Prevention

**Lesson 1 – never build "consume exactly once" on `unlink`'s error.** If the
correctness of a token, lock, or claim rests on "only one caller can delete it",
the primitive is wrong. Use `rename` onto a private name, an `O_EXCL` create, or
a real lock. The tell is a comment of the form "a file can only be deleted
once".

**Lesson 2 – never validate an exclusivity property sequentially.** A test that
calls the claim twice in a row proves the *state machine*, not the *exclusion*.
Exclusivity claims need a test that actually races: spawn N threads, have them
all attempt the claim, and assert the number of winners is exactly 1 – repeated
enough times that a rare interleaving shows up. The `unlink` version passes
every sequential test and fails this one immediately, which is precisely why the
sequential suite was the dangerous thing here rather than the missing one.

This generalizes past filesystems: the same trap applies to "delete the row and
see if we deleted it" and to any check-then-act pair dressed up as a single
operation.

## Related Issues

- The gate the token opens: `src/plugin/hook/pre_tool_use.rs`, which calls
  `bypass::consume` for exactly one `Read`.
- The other consumer of the same primitive:
  `src/plugin/expect_artifact.rs::take_oldest`, where taking the record IS the
  one-shot flag that keeps a subagent's stop blocked at most once.
- Same family of "phrase the question so the unknown answer is safe" reasoning
  as [secret-gate-positive-tracked-determination-fail-closed.md](./secret-gate-positive-tracked-determination-fail-closed.md).
