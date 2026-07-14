---
title: "Building .tar.bz2 archives in Rust: pure-Rust bzip2 backend and symlink safety"
date: 2026-07-14
category: tooling-decisions
module: pack
problem_type: tooling_decision
component: tooling
severity: high
applies_when:
  - "Adding tar/bzip2 (or any tar.*) archive writing to a Rust binary distributed via cargo-dist"
  - "Choosing between a C-linked and a pure-Rust compression backend for a hermetic cross-platform build"
  - "Archiving a set of user-configured files/directories that may contain symlinks"
tags:
  - rust
  - bzip2
  - tar
  - cargo-dist
  - symlinks
  - secrets
  - hermetic-build
---

# Building `.tar.bz2` archives in Rust: pure-Rust bzip2 backend and symlink safety

Two non-obvious gotchas surfaced while adding the `ss-magic pack` command
(archive the configured files into `ss-magic-files.tar.bz2` at the git root).
Both are about the `tar` + `bzip2` crate pair, and both matter to any Rust CLI
that writes archives.

## Context

`ss-magic` is a standalone Rust CLI distributed as prebuilt binaries via
cargo-dist and self-updating from GitHub Releases. Adding archive output meant
picking a bzip2 backend and tarring a user-configured file set (`pack.rs`,
reusing the `apply::match_paths` pattern engine). Two defaults bit us.

## Guidance

### 1. `bzip2` 0.6 uses a pure-Rust backend *by default* — do not add a feature flag

The `bzip2` crate (0.6.x) compresses with the pure-Rust `libbz2-rs-sys` backend
by **default**. There is no feature flag to enable it — and passing one is an
error:

```
$ cargo add bzip2 --features libbz2-rs-sys
error: unrecognized feature for crate bzip2: libbz2-rs-sys
disabled features:
    bzip2-sys, static
```

The C backend is the **opt-in** `bzip2-sys` feature (off by default). So the
correct dependency for a hermetic, C-toolchain-free build is simply:

```toml
bzip2 = "0.6"   # pure-Rust libbz2-rs-sys backend, no features needed
tar = "0.4"
```

Verify no C backend leaked in:

```
$ cargo tree -i bzip2-sys
error: package ID specification `bzip2-sys` did not match any packages   # good — absent
```

This keeps cargo-dist's cross-platform release builds (and the from-source
install path) free of a system `libbz2` / C compiler dependency, matching the
crate's existing rustls + rust-native archive posture (`self_update` is already
configured the same way).

### 2. `tar::Builder` follows symlinks by default — a secret-leak vector

`tar::Builder` defaults to `follow_symlinks(true)`: when it encounters a
symlink (top-level via `append_path_with_name`, or nested via
`append_dir_all`), it **dereferences it and embeds the target file's bytes**
under the link's name, with no symlink marker. When you are archiving a
user-configured directory that may contain a link pointing *outside* the repo
(say `bundle/creds -> ~/.aws/credentials`), the target's secret bytes silently
land in the archive. A broken symlink additionally hard-aborts the whole
archive with a `NotFound` I/O error.

Store links as links instead:

```rust
let mut builder = tar::Builder::new(encoder);
builder.follow_symlinks(false);   // symlink entries, never dereferenced
```

This matches the repo's existing file-copy posture in `apply.rs`, which walks
with `follow_links(false)` and never follows a symlink out of the source tree.
`Path::is_file()` follows symlinks, so a top-level `abs.is_file()` guard does
**not** save you — set `follow_symlinks(false)` on the builder itself.

## Why This Matters

- **Backend choice is a distribution constraint, not a preference.** A
  C-linked bzip2 backend would require a working C toolchain on every
  cargo-dist target and for every from-source install, breaking the hermetic
  build guarantee. The default already avoids this; adding the "obvious"
  feature flag breaks the build with an unrecognized-feature error.
- **The symlink default is a real exfiltration path.** Archiving is exactly the
  operation where a dereferenced symlink turns "package my config files" into
  "package whatever my config files happen to link to," and the leaked bytes
  are invisible in the archive listing (they look like a normal file). This is
  a `high`-severity correctness/security issue caught only in review, not by
  the happy-path tests.

## When to Apply

- Any Rust binary adding `tar` + `bzip2` archive output, especially one shipped
  as a prebuilt cross-platform binary (cargo-dist, `cross`, musl static builds).
- Any archive-writing code whose input set is user-configured globs/paths that
  could resolve to symlinks — set `follow_symlinks(false)` and add a regression
  test with a link pointing outside the archived tree, asserting the target's
  bytes are absent.

## Examples

Confirm the symlink fix end-to-end (a matched directory holding a link to an
out-of-repo secret):

```
$ tar -tvjf ss-magic-files.tar.bz2
drwxr-xr-x  bundle/
lrwxr-xr-x  bundle/leak -> /tmp/.../secret.txt     # stored as a symlink, not dereferenced
-rw-r--r--  bundle/real.txt
-rw-r--r--  .env
$ tar -xOjf ss-magic-files.tar.bz2 | grep TOPSECRET    # secret bytes NOT present
$   # (no output — no leak)
```

Regression test shape (from `src/pack.rs`): create a matched dir with a symlink
to a secret outside the repo, pack, then assert the `leak` entry's
`entry_type().is_symlink()` and that no file entry's contents contain the
secret string. A second test asserts a broken symlink inside a matched
directory does not abort the pack (exit 0, the real file still packed).
