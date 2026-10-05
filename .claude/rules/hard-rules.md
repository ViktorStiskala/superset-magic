## Secret-safety constraints (hard rules)

The unified sync engine is the ONE path that writes untracked (secret) files into
the shared main checkout, and `pack` archives the configured files, so both are
secret-leak surfaces. Two constraints are load-bearing here: violating either is a
secret leak, not a cosmetic bug. Each is backed by a `docs/solutions/` write-up of
the real incident this run fixed.

- **Determine "is this a secret?" POSITIVELY, and fail closed.** The
  gitignore-in-main gate must fire for a git-UNTRACKED worktree source, decided by
  POSITIVE tracked determination (`!git::tracked_files(...).contains(rel)`) so that
  anything NOT positively known-tracked (a non-UTF-8 / NFD-vs-NFC / otherwise
  unenumerable name) defaults to secret and runs the gate. NEVER derive
  untracked-ness by ABSENCE from an untracked set (`untracked.contains(rel)`) — a
  lookup miss then lands on the permissive side and leaks. Rule for any security
  gate: phrase the question so the UNKNOWN answer is the SAFE one. See
  [docs/solutions/logic-errors/secret-gate-positive-tracked-determination-fail-closed.md](../../docs/solutions/logic-errors/secret-gate-positive-tracked-determination-fail-closed.md).
- **Enforce a secret-excluding path filter at the point of final enumeration, not
  on an upstream list.** The excluded-trees filter (`sync::under_excluded_tree`
  over `sync::EXCLUDED_TREES` – `.superset/backups`, `.superset/.magic`,
  `.scratchpad`, `.git`) must be applied where the file set is actually
  materialized – every directory walk (`pack`'s `append_dir_excluding_trees`,
  `apply::walk_source`, `apply::copy_dir_recursive`, reverse sync's candidate
  computation) – NOT only on the
  flat match list, because a later step that re-walks the live filesystem
  (`append_dir_all`, `copy_dir_recursive`, `WalkDir`) bypasses an upstream filter.
  The trap is a directory match that is an ANCESTOR of the excluded subtree (a bare
  `.superset` pattern, a broad `**`) – and one such match can sit above SEVERAL
  excluded trees at once, since `.superset` is the ancestor of both `backups` and
  `.magic`. A comment asserting "X is never included" is a
  red flag unless the guard sits on the enumeration layer; test the directory-match
  shape, not just the leaf. (The write-up below records the incident under the
  names the code carried at the time, `under_backups_dir` /
  `append_dir_excluding_backups`; its Problem and What-Didn't-Work sections keep
  those deliberately, while its Solution and Related sections name the current
  `sync::under_excluded_tree` / `pack::append_dir_excluding_trees`.) See
  [docs/solutions/logic-errors/pack-backups-exclusion-must-guard-the-directory-walk.md](../../docs/solutions/logic-errors/pack-backups-exclusion-must-guard-the-directory-walk.md).

## Plugin constraints (hard rules)

The plugin adds three surfaces with their own failure modes. Each rule below is
backed by a `docs/solutions/` write-up of the real incident behind it, except the
last, which is backed by the eight-bypass sequence recorded in this file.

- **Never build "consume exactly once" on `unlink`'s error, and never validate
  an exclusivity property sequentially.** Measured here: 8 threads racing to
  `unlink` one path produced up to 5 successes across 20 trials, while
  sequential testing shows exactly the `ENOENT` you expect – which is what makes
  it dangerous. The one-shot bypass token (exactly the next gated Read) was
  built on it and would have leaked to several concurrent reads. The fix is
  `rename` onto a private landing file in the same directory
  (the plugin crate's `claim.rs::take`), which gave exactly one winner in every trial. See
  [docs/solutions/logic-errors/unlink-is-not-an-exclusive-claim.md](../../docs/solutions/logic-errors/unlink-is-not-an-exclusive-claim.md).
- **Parse-sensitive git output must not go through a trimming convenience
  wrapper.** The shared `git()` helper trims the whole output, which eats the
  leading space of the first `git status --porcelain` line – that column is a
  literal space when a file is modified in the worktree only – shifting every
  field and silently misreading the status. `git::status_porcelain` is written
  against `git_raw` for exactly this reason. See
  [docs/solutions/logic-errors/trimming-wrapper-corrupts-porcelain-status.md](../../docs/solutions/logic-errors/trimming-wrapper-corrupts-porcelain-status.md).
- **Phrase every plugin gate so the UNKNOWN answer is the SAFE one, and never
  let a hook fail loudly.** The two postures pull in opposite directions and
  both are load-bearing: a hook that errors, panics or times out must look
  exactly like one that decided to do nothing (`hook::run` has no non-zero exit
  path, and a handler panic is caught), while the scratchpad's ignore gate, the
  tracked-path check and the tmproot ownership check all refuse on "could not
  ask" as well as on "no". Do not "simplify" either half toward the other.

- **A path gate must perform every expansion the harness performs before it
  roots a path – or refuse to root it.** Eight bypasses of the R88 checklist deny
  came from one habit: classifying from the ACTOR (the hook's process, the
  envelope's cwd) rather than from the TARGET, and recognizing SPELLINGS rather
  than the property behind them. A symlinked ancestor, a case difference, a
  relative target, a `..` component, a `/proc/self/cwd` prefix, a leading `..`, a
  decoy symlink in an opaque path's TAIL and a leading `~` were each found and
  patched one at a time; each patch produced the next hole. The rule has three
  moves, in order: (1) perform every expansion the harness performs – or refuse
  to root the path – and reduce lexically, both before anything else looks at the
  path; (2) decide process-relativeness as a PROPERTY (`pathnorm::process_view`),
  never a prefix list, and never trust a resolution whose result depends on which
  process performs it – canonicalization may be used to ADD a denial but never to
  CLEAR one; (3) derive comparison roots from the target as well as the actor.
  Note move 1's phrasing: an earlier form said only "never trust a
  process-dependent resolution", which quantifies over resolutions the code
  PERFORMS and so cannot catch one it OMITS – which is exactly what the tilde
  was. The expansion surface is bounded BY MEASUREMENT, not assumption: `~`
  diverges (handled), `$HOME` syntax was probed and provably does not (both sides
  treat it as a literal, deliberately unhandled), `/proc` is move 2. **Do not add
  speculative expansions – probe first.** Adding spellings on suspicion is what
  produced the sequence. If a ninth bypass turns up, ask which of the three moves
  it escaped, not which spelling to add.
