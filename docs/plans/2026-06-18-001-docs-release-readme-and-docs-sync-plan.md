---
title: "docs: release-ready README installation + docs/CLAUDE.md sync"
type: docs
date: 2026-06-18
---

# docs: release-ready README installation + docs/CLAUDE.md sync

## Summary

Make `README.md` read like a shipped v1: lead the **Install** section with the
end-user released-binary path (the cargo-dist shell installer pulled from
GitHub Releases, plus a manual prebuilt-binary download and a from-source
fallback), and state supported platforms. While there, fix one factual drift
the Tier 2 review surfaced — the Self-update section still claims SHA-256
verification against the GitHub asset digest, which the code does **not** do
(integrity is TLS + cargo-dist checksums) — and confirm `CLAUDE.md` is
consistent with the shipped reality. Docs-only; no behavior changes.

All paths are relative to the repo root (the standalone
`ViktorStiskala/superset-magic` repo). Work stays on the current `initial`
branch (PR #1).

## Problem Frame

The README was grown incrementally across the rewrite and now has two release
gaps:

1. **Install is source-only.** The single documented path is `make install`
   (`cargo install --path .`), which assumes a cloned repo + Rust toolchain.
   An end user installing a released `ss-magic` has no instructions for the
   actual distribution channel — the cargo-dist shell installer and the
   GitHub Releases archives the project now publishes (the `magic.sh` wrapper
   already points users at `…/releases/latest/download/ss-magic-installer.sh`,
   but the README never documents it).
2. **A factual drift.** `README.md` Self-update says the updater "verifies the
   SHA-256 against the GitHub asset digest." The Tier 2 code review established
   that `self_update` does not expose that check; integrity rests on TLS +
   cargo-dist's per-archive checksums. `CLAUDE.md` and `src/update/apply.rs`
   were corrected during the review, but the README still carries the old
   claim — a user-facing inaccuracy about a security-relevant path.

"Sync docs" here means README + `CLAUDE.md` consistency with the shipped code
(the only user-facing docs; `docs/` otherwise holds historical CE pipeline
artifacts that are not rewritten). "Final release" means the README front-door
(title → install → commands) should serve someone who has never seen the repo.

## Requirements

- R1. The README **Install** section leads with the released-binary install
  (cargo-dist shell installer via `curl … | sh` from the latest GitHub
  Release), with a manual prebuilt-binary + checksum option, and a from-source
  fallback (`cargo install --git` and the existing `make install`). It states
  supported platforms (macOS arm64/x86_64, Linux x86_64/arm64; Windows not in
  the release matrix) and notes that self-update keeps an installed binary
  current.
- R2. The README **Self-update** section accurately describes integrity:
  TLS-fetched download from GitHub Releases verified by cargo-dist's published
  checksums, with no SHA-256-vs-asset-digest check (matching
  `src/update/apply.rs` KTD5 notes and `CLAUDE.md`). No other README claim
  contradicts the shipped code.
- R3. `CLAUDE.md` is consistent with the shipped reality: the Build section
  acknowledges the released-binary install path (so a contributor knows the
  binary is also distributed, not only built from source), and nothing in
  `CLAUDE.md` contradicts the code or the corrected README.
- R4. No stale `superset-setup` binary/command references remain in either
  user-facing doc (already verified absent — guard against reintroduction).

## Key Technical Decisions

- KTD1. **Installer URL is the cargo-dist convention already wired into
  `assets/magic.sh`:** `https://github.com/ViktorStiskala/superset-magic/releases/latest/download/ss-magic-installer.sh`.
  Reuse that exact URL in the README so the wrapper's install hint and the
  README never drift. Only the shell installer exists (U12 set
  `installers = ["shell"]`; the PowerShell installer was dropped because the
  release matrix has no Windows target) — document shell + from-source only,
  not a PowerShell one-liner.
- KTD2. **Not on crates.io yet**, so the from-source path is `cargo install
  --git https://github.com/ViktorStiskala/superset-magic` (or `--path .` from a
  clone via `make install`), not `cargo install ss-magic`. Publishing to
  crates.io is deferred open-source-release work (out of scope here; tracked
  separately).
- KTD3. **Describe integrity as it is, not as planned.** Match the language in
  `src/update/apply.rs` and `CLAUDE.md`: download over TLS from GitHub
  Releases, verified by cargo-dist's published per-archive checksums; binary
  signing / SHA-256-vs-digest is a documented future item, not a current
  guarantee. Do not overstate the security posture in user docs.

## Implementation Units

### U1. Rewrite the README Install section for a released binary

- **Goal:** Replace the source-only Install block with release-first install
  instructions an end user can follow without cloning.
- **Requirements:** R1.
- **Dependencies:** none.
- **Files:** `README.md`.
- **Approach:** Restructure the `## Install` section into ordered paths,
  released-binary first:
  1. **Shell installer (recommended, macOS/Linux):** the `curl -sSfL
     …/releases/latest/download/ss-magic-installer.sh | sh` one-liner (the URL
     from KTD1), with a one-line note that it installs the latest release and
     that `ss-magic` self-updates thereafter.
  2. **Manual download:** grab the platform archive + its `.sha256` from the
     GitHub Releases page, verify, extract, put `ss-magic` on `PATH`.
  3. **From source:** `cargo install --git
     https://github.com/ViktorStiskala/superset-magic` for users with a Rust
     toolchain; keep `make install` / `cargo install --path .` as the
     clone-and-build path (the existing `## Make targets` block stays).
  Add a short **Supported platforms** line (macOS arm64 + x86_64, Linux x86_64
  + arm64; Windows not in the release matrix). Keep the prose tight and the
  existing self-update cross-reference.
- **Patterns to follow:** the README's existing terse, fenced-command style;
  the installer URL already in `assets/magic.sh`.
- **Test scenarios:** Test expectation: none — documentation only, no
  behavioral change. Verification is the review checklist in U-level
  Verification below.
- **Verification:** the Install section names all three paths in order, the
  installer URL byte-matches `assets/magic.sh`, supported platforms are stated,
  and a reader who has never cloned the repo could install `ss-magic`.

### U2. Correct the README Self-update integrity claim + drift scan

- **Goal:** Make the README's self-update description factually match the code.
- **Requirements:** R2, R4.
- **Dependencies:** none (independent of U1).
- **Files:** `README.md`.
- **Approach:** In the `## Self-update` section, replace the "verifies the
  SHA-256 against the GitHub asset digest" clause with accurate language:
  the download runs over TLS from GitHub Releases and is verified against
  cargo-dist's published per-archive checksums; there is no
  SHA-256-vs-asset-digest check (mirror `src/update/apply.rs` KTD5 wording and
  `CLAUDE.md`). Keep the rest of the section (cache, ETag, 5 s timeout, lock,
  atomic swap, re-exec, escape hatches) as-is. Then scan the whole README for
  any other claim that contradicts the shipped code (e.g., the PowerShell
  installer, crates.io install, or `setup.sh`-era references) and fix or
  remove.
- **Patterns to follow:** the corrected integrity wording already in
  `CLAUDE.md` (`## …update/…` bullet) and `src/update/apply.rs` module docs.
- **Test scenarios:** Test expectation: none — documentation correctness.
- **Verification:** `grep -n 'SHA-256\|digest' README.md` no longer asserts a
  digest check as a current guarantee; the README's integrity description
  matches `CLAUDE.md` and `src/update/apply.rs`; no contradicting claim remains.

### U3. Sync CLAUDE.md Build section with the release install path

- **Goal:** Keep the contributor-facing build doc consistent with the shipped
  distribution channel.
- **Requirements:** R3, R4.
- **Dependencies:** U1 (so the README install paths exist to point at).
- **Files:** `CLAUDE.md`.
- **Approach:** In the `## Build` section, add a one-line note that release
  binaries are published to GitHub Releases via cargo-dist and that end-user
  install instructions live in `README.md` (so a contributor doesn't assume
  source build is the only path). Confirm the rest of `CLAUDE.md` is already
  consistent with the shipped code (the self-update integrity bullet was
  corrected during the Tier 2 review — verify, don't re-edit). Keep `CLAUDE.md`
  scoped to build/architecture; do not duplicate the full README install
  instructions into it.
- **Patterns to follow:** the existing terse `CLAUDE.md` Build/architecture
  style; the already-corrected self-update bullet.
- **Test scenarios:** Test expectation: none — documentation only.
- **Verification:** `CLAUDE.md` Build section references the released-binary
  channel + points to `README.md`; no `CLAUDE.md` statement contradicts the
  code or the updated README; no stale `superset-setup` reference.

## Scope Boundaries

### Deferred to follow-up work

- Publishing `ss-magic` to crates.io (needs `authors`/`license`/`description`
  metadata + a LICENSE file + name availability) and the broader open-source
  release mechanics (public README polish beyond install, repo move). Tracked
  as the open-source-release follow-up; not part of this docs sync.

### Outside this work

- Any code change. This plan is docs-only — `README.md` and `CLAUDE.md`.
- Rewriting historical `docs/plans/` or `docs/brainstorms/` artifacts — those
  are CE pipeline records, not user-facing docs.
- Adding a LICENSE file or a real release tag/run (no live release is cut here).

## Sources / Research

- `README.md` — current Install (source-only) and Self-update (stale SHA-256
  claim) sections; the established terse command-fenced doc style.
- `CLAUDE.md` — already-corrected self-update integrity bullet + Build section
  to extend.
- `src/update/apply.rs` — KTD5 conformance notes: integrity is TLS +
  cargo-dist checksums, no SHA-256-vs-digest.
- `assets/magic.sh` — the canonical installer URL
  (`…/releases/latest/download/ss-magic-installer.sh`) to reuse verbatim.
- `dist-workspace.toml` — `installers = ["shell"]`, four-target macOS/Linux
  matrix (no Windows) → shell-installer-only, platform list.
