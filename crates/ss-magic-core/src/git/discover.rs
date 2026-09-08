//! Subprocess-free discovery of the two roots the plugin's hook pipeline
//! needs: the worktree root (`git rev-parse --show-toplevel`) and the main
//! checkout root (the parent of `git rev-parse --git-common-dir`).
//!
//! Every hook invocation used to spawn both probes before it could even read
//! `plugin.enabled`, and a `PreToolUse` hook fires on nearly every tool call.
//! This module answers the same two questions from the filesystem alone
//! (R20 – the hook path discovers both roots without spawning a process) and
//! hands back to the subprocess probes whenever the layout is anything but
//! the ordinary shapes (R21 – the decline conditions). It is wired into the
//! hook pipeline only; the CLI's own commands keep calling the probes (R22),
//! and by convention this is the ONE filesystem-only reduction of git
//! behavior in the crate – it must never grow ref, index, or write handling
//! (R23).
//!
//! ## The invariant: a fast answer is byte-equal to git's, or there is none
//!
//! The roots decide where plugin state is written and which checkout
//! `plugin.enabled` is read from, so a wrong fast answer is worse than a slow
//! one. [`discover`] therefore returns one of three things: [`Discovery::Found`]
//! with roots that equal what the probes would print, [`Discovery::NotARepository`]
//! when the walk has proven there is nothing for git to find either, or
//! [`Discovery::Undecided`] with a reason – and [`roots`] turns `Undecided` into
//! exactly the two subprocess calls the pipeline made before. The `Undecided`
//! set is deliberately wide: whenever anything is unusual, declining costs a
//! few milliseconds, while guessing could put state in the wrong tree.
//!
//! ## What the walk does (KTD8)
//!
//! 1. If any variable in [`DECLINING_ENV`] is set, decline: each changes how
//!    git resolves the repository in a way this walk does not reproduce.
//! 2. Canonicalize `cwd`. `canonicalize` owns `.` and `..` and symlinks; no
//!    lexical normalization is reimplemented here.
//! 3. Walk the ancestors `D` from `cwd` upward. At each `D`:
//!    - If `D` itself looks like a git directory (a `HEAD` beside `objects/`
//!      and `refs/`, or beside a `commondir`), decline: `cwd` is inside a git
//!      directory or a bare repository, and `--show-toplevel` fails there.
//!    - `lstat(D/.git)`. A symlink declines. A directory that is a git
//!      directory of the plain shape yields
//!      `{ worktree: D, common: D/.git, main: D }`. A regular file is a
//!      gitfile: read it (at most 4 KiB), require the `gitdir: ` prefix,
//!      resolve the path against `D` and canonicalize it, require a
//!      `commondir` beside it (a gitfile WITHOUT one is a submodule, whose
//!      main root is not a parent of anything), resolve and canonicalize that,
//!      and yield `{ worktree: D, common: C, main: canonical(parent(C)) }`.
//!      Anything else declines.
//!    - Absent: if `parent(D)` is on a different filesystem, decline (git
//!      stops at a mount point and reports an error there); otherwise continue
//!      with the parent. With no parent left, `NotARepository`.
//!
//! ## Where this is stricter than KTD8's text, and why
//!
//! KTD8 describes the `.git` directory branch as "require a regular
//! `D/.git/HEAD`". Measured against git 2.55, that alone would return a
//! `Found` in several layouts where git answers differently, so each of those
//! DECLINES instead. None of them changes a `Found` answer; they only move
//! layouts from `Found` to `Undecided`:
//!
//! - git's own test for "is this a git directory" also requires `HEAD`'s
//!   CONTENT to be a symref or an object id, and `objects/` and `refs/` to be
//!   searchable directories – and when the test fails, git walks UP to the
//!   next ancestor rather than stopping. A `.git` with a garbage `HEAD` or no
//!   `objects/` nested inside another repository therefore resolves to the
//!   OUTER repository in git. [`git_directory_shape`] applies git's test.
//! - A `.git` DIRECTORY carrying a `commondir` file is a worktree git-dir
//!   sitting where a main checkout's would; git's common dir is then
//!   elsewhere, so `main` is not `D`. Declined.
//! - A gitfile's target must itself be a git directory or git fails outright
//!   ("not a git repository"); KTD8 only required it to canonicalize.
//! - `core.worktree` moves git's worktree root to an arbitrary directory, and
//!   `core.bare = true` makes `--show-toplevel` fail even with a working tree
//!   beside `.git`; a repository format git refuses to open
//!   (`core.repositoryformatversion` above 1, an unknown extension) fails
//!   every probe. None of these is visible in the directory layout, so
//!   [`repository_format_check`] scans the repository's own config file for
//!   them – conservatively: anything it cannot follow declines.
//! - `GIT_OBJECT_DIRECTORY` (not among R21's five) replaces the `objects/`
//!   git looks for, so it joins the declining variables.
//! - git refuses a repository owned by another user ("dubious ownership")
//!   unless `safe.directory` allows it. git's check covers the worktree, its
//!   `.git` entry and – for a gitfile – the gitfile's target directory; the
//!   walk declines when any of those three is not owned by this process's
//!   effective uid, and ALSO requires every directory it lists (`objects/`,
//!   `refs/`) to be owned by it. It does not separately check the common
//!   directory or the main checkout root a gitfile leads to, because git
//!   does not either: adding those declines would only cost the fast path
//!   in a layout where git itself proceeds.
//!
//! Two of git's discovery rules are matched only by measurement, not by
//! reasoning from a spec: the exact `HEAD` content rule ([`head_content_is_valid`],
//! taken from git's `validate_headref`) and the config-format check (from
//! `check_repository_format_gently`, which reads the repository config file
//! WITHOUT include expansion, and reads the common config's `core.bare` /
//! `core.worktree` for a linked worktree only under
//! `extensions.worktreeConfig`). Both are pinned by the equivalence matrix in
//! the tests, which runs every scenario against real `git` output.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// The largest gitfile the walk will read. KTD8 caps the read at 4 KiB; a
/// gitfile larger than that is declined rather than read truncated, since a
/// truncated path could still name something that exists.
pub const GITFILE_MAX_BYTES: usize = 4096;

/// How much of `HEAD` git's `validate_headref` reads (a 256-byte buffer,
/// NUL-terminated). Reading the same amount keeps the content rule identical
/// for a `HEAD` longer than that.
const HEAD_READ_BYTES: usize = 255;

/// The environment variables whose presence makes the walk decline, each with
/// the reason recorded for it. The first five are R21's; `GIT_OBJECT_DIRECTORY`
/// is the sixth because git's "is this a git directory" test consults it in
/// place of `<gitdir>/objects`, so with it set the walk's `objects/` check no
/// longer mirrors git's.
pub const DECLINING_ENV: [(&str, &str); 6] = [
    ("GIT_DIR", "GIT_DIR is set"),
    ("GIT_WORK_TREE", "GIT_WORK_TREE is set"),
    ("GIT_COMMON_DIR", "GIT_COMMON_DIR is set"),
    ("GIT_CEILING_DIRECTORIES", "GIT_CEILING_DIRECTORIES is set"),
    (
        "GIT_DISCOVERY_ACROSS_FILESYSTEM",
        "GIT_DISCOVERY_ACROSS_FILESYSTEM is set",
    ),
    ("GIT_OBJECT_DIRECTORY", "GIT_OBJECT_DIRECTORY is set"),
];

/// The repository extensions git 2.55 knows. A key under `[extensions]` not
/// in this list makes git refuse the repository (with
/// `core.repositoryformatversion = 1`; with version 0 git ignores it, and the
/// walk declines for both rather than tracking which).
const KNOWN_EXTENSIONS: [&str; 8] = [
    "noop",
    "preciousobjects",
    "partialclone",
    "worktreeconfig",
    "objectformat",
    "compatobjectformat",
    "refstorage",
    "relativeworktrees",
];

/// The three roots, all canonical.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roots {
    /// What `git rev-parse --show-toplevel` prints from `cwd`.
    pub worktree_root: PathBuf,
    /// What `git rev-parse --git-common-dir` prints, resolved: `D/.git` for a
    /// main checkout, the shared git directory for a linked worktree. Not read
    /// by the pipeline (which needs only the other two), but part of the
    /// answer this walk reproduces, and the matrix pins it against git.
    #[allow(dead_code)]
    pub common_dir: PathBuf,
    /// The parent of `common_dir`: the main checkout for a repository with a
    /// working tree, and – as with the probes – the parent DIRECTORY of a bare
    /// repository for a worktree of one.
    pub main_checkout_root: PathBuf,
}

/// What one walk concluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Discovery {
    /// The ordinary shapes; the roots equal the probes' answer.
    Found(Roots),
    /// The walk reached the filesystem root without seeing anything git
    /// would accept either. No subprocess is needed to confirm it.
    NotARepository,
    /// Something was unusual; the reason is a short static phrase for the
    /// heartbeat row. The caller runs the subprocess probes.
    Undecided(&'static str),
}

/// The pipeline's view: the two roots exactly as the probes would have
/// answered, plus whether the probes actually ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The worktree root, or `None` when there is none – outside any
    /// repository, or where `--show-toplevel` fails (inside a git directory).
    pub repo_root: Option<PathBuf>,
    /// The main checkout root, or `None` when git could not name one. Note the
    /// probe answers this from inside a git directory even when
    /// `--show-toplevel` fails there, and `roots` reproduces that.
    pub main_root: Option<PathBuf>,
    /// `Some(reason)` when the walk declined and the probes ran.
    pub fallback: Option<&'static str>,
}

/// Discover the roots for `cwd` from the filesystem and this process's
/// environment. Never spawns anything.
pub fn discover(cwd: &Path) -> Discovery {
    discover_with_env(cwd, &|name| std::env::var_os(name))
}

/// [`discover`] with the environment lookup injected, so a test can run the
/// walk under a controlled environment without touching the process-wide one
/// that every other test's `git` child would inherit.
pub fn discover_with_env(cwd: &Path, env: &dyn Fn(&str) -> Option<OsString>) -> Discovery {
    for (name, reason) in DECLINING_ENV {
        if env(name).is_some() {
            return Discovery::Undecided(reason);
        }
    }
    let Ok(start) = cwd.canonicalize() else {
        return Discovery::Undecided("cwd cannot be canonicalized");
    };
    let euid = effective_uid();

    let mut dir = start.as_path();
    loop {
        if looks_like_git_dir(dir) {
            return Discovery::Undecided("inside a git directory or bare repository");
        }
        match inspect_dot_git(dir, euid) {
            Step::Found(roots) => return Discovery::Found(roots),
            Step::Decline(reason) => return Discovery::Undecided(reason),
            Step::Absent => {}
        }
        let Some(parent) = dir.parent() else {
            return Discovery::NotARepository;
        };
        match same_device(dir, parent) {
            Ok(true) => {}
            Ok(false) => return Discovery::Undecided("filesystem boundary"),
            Err(_) => return Discovery::Undecided("an ancestor could not be inspected"),
        }
        dir = parent;
    }
}

/// The pipeline's entry point: `Found` becomes its roots, `NotARepository`
/// becomes two `None`s without a subprocess, and `Undecided` becomes the two
/// probe calls the pipeline made before this module existed – in the same
/// order and against the same directories, so the answer is byte-for-byte
/// what it was.
pub fn roots(cwd: &Path) -> Resolved {
    match discover(cwd) {
        Discovery::Found(roots) => Resolved {
            repo_root: Some(roots.worktree_root),
            main_root: Some(roots.main_checkout_root),
            fallback: None,
        },
        Discovery::NotARepository => Resolved {
            repo_root: None,
            main_root: None,
            fallback: None,
        },
        Discovery::Undecided(reason) => {
            let repo_root = super::cwd_repo_root(cwd).ok();
            // `config::resolve` asked for the main root at the worktree root
            // when there was one and at `cwd` itself otherwise; from inside a
            // git directory that second form still answers.
            let main_root = super::main_checkout_root(repo_root.as_deref().unwrap_or(cwd)).ok();
            Resolved {
                repo_root,
                main_root,
                fallback: Some(reason),
            }
        }
    }
}

// ── One ancestor ──────────────────────────────────────────────────────────────

/// What inspecting `D/.git` concluded.
#[derive(Debug)]
enum Step {
    Found(Roots),
    Decline(&'static str),
    Absent,
}

/// Does `dir` ITSELF look like a git directory? Deliberately looser than
/// git's own test (any `HEAD` entry, not only a valid regular one; a
/// `commondir` counts in place of `objects/` and `refs/`), because the answer
/// is only ever used to decline, and git's `--show-toplevel` fails from inside
/// any directory git takes for a bare repository or a git directory.
fn looks_like_git_dir(dir: &Path) -> bool {
    if fs::symlink_metadata(dir.join("HEAD")).is_err() {
        return false;
    }
    (dir.join("objects").is_dir() && dir.join("refs").is_dir())
        || fs::symlink_metadata(dir.join("commondir")).is_ok()
}

/// `lstat(D/.git)` and classify it.
fn inspect_dot_git(dir: &Path, euid: u32) -> Step {
    let dot_git = dir.join(".git");
    let meta = match fs::symlink_metadata(&dot_git) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Step::Absent,
        Err(_) => return Step::Decline(".git could not be inspected"),
    };
    let kind = meta.file_type();
    if kind.is_symlink() {
        return Step::Decline(".git is a symlink");
    }
    // git's "dubious ownership" refusal covers the worktree and the git
    // directory (and, for a gitfile, the gitfile itself); the walk declines
    // wherever git might refuse. A gitfile's target is checked in
    // `from_gitfile`; the common directory it names is not, matching git.
    if !owned_by(dir, euid) || meta.uid() != euid {
        return Step::Decline("repository is not owned by this user");
    }
    if kind.is_dir() {
        return from_git_directory(dir, &dot_git, euid);
    }
    if kind.is_file() {
        return from_gitfile(dir, &dot_git, euid);
    }
    Step::Decline(".git is neither a directory nor a regular file")
}

/// `D/.git` is a directory: the main-checkout shape.
fn from_git_directory(dir: &Path, dot_git: &Path, euid: u32) -> Step {
    if fs::symlink_metadata(dot_git.join("commondir")).is_ok() {
        return Step::Decline(".git directory carries a commondir");
    }
    if !git_directory_shape(dot_git, dot_git, euid) {
        return Step::Decline(".git directory has no valid HEAD, objects and refs");
    }
    if let Err(reason) = repository_format_check(dot_git, dot_git, false) {
        return Step::Decline(reason);
    }
    Step::Found(Roots {
        worktree_root: dir.to_path_buf(),
        common_dir: dot_git.to_path_buf(),
        main_checkout_root: dir.to_path_buf(),
    })
}

/// `D/.git` is a regular file: the linked-worktree shape (or a submodule,
/// which declines).
fn from_gitfile(dir: &Path, dot_git: &Path, euid: u32) -> Step {
    let body = match read_capped(dot_git, GITFILE_MAX_BYTES) {
        Ok(Some(body)) => body,
        Ok(None) => return Step::Decline("gitfile is larger than 4 KiB"),
        Err(_) => return Step::Decline("gitfile could not be read"),
    };
    let Some(target) = body.strip_prefix(b"gitdir: ") else {
        return Step::Decline("gitfile lacks the gitdir: prefix");
    };
    // git strips trailing CR/LF from the path and nothing else: a trailing
    // space is part of the path (and then fails to resolve, as it does here).
    let target = trim_trailing_newlines(target);
    if target.is_empty() {
        return Step::Decline("gitfile names no path");
    }
    let Ok(git_dir) = resolve_against(target, dir) else {
        return Step::Decline("gitfile target cannot be canonicalized");
    };
    if !owned_by(&git_dir, euid) {
        return Step::Decline("repository is not owned by this user");
    }

    let commondir_file = git_dir.join("commondir");
    let common_target = match read_capped(&commondir_file, GITFILE_MAX_BYTES) {
        Ok(Some(body)) => body,
        Ok(None) => return Step::Decline("commondir is larger than 4 KiB"),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Step::Decline("gitfile target has no commondir");
        }
        Err(_) => return Step::Decline("commondir could not be read"),
    };
    let Ok(common) = resolve_against(trim_trailing_newlines(&common_target), &git_dir) else {
        return Step::Decline("commondir cannot be canonicalized");
    };

    if !git_directory_shape(&git_dir, &common, euid) {
        return Step::Decline("gitfile target is not a git directory");
    }
    if let Err(reason) = repository_format_check(&git_dir, &common, true) {
        return Step::Decline(reason);
    }
    let Some(main) = common.parent() else {
        return Step::Decline("commondir has no parent");
    };
    let Ok(main) = main.canonicalize() else {
        return Step::Decline("main checkout root cannot be canonicalized");
    };
    Step::Found(Roots {
        worktree_root: dir.to_path_buf(),
        common_dir: common,
        main_checkout_root: main,
    })
}

// ── git's "is this a git directory" test ─────────────────────────────────────

/// git's `is_git_directory(gitdir)`: `gitdir/HEAD` must be a regular file
/// whose content passes [`head_content_is_valid`], and `objects/` and `refs/`
/// under the COMMON directory (which is `gitdir` itself for a main checkout)
/// must be searchable directories.
///
/// A symlinked `HEAD` is a valid repository to git (an old-style symref) but
/// fails here – declining, never answering differently. "Searchable" is
/// git's `access(X_OK)`; with the directory already required to be owned by
/// this uid, the owner's execute bit is what decides that.
fn git_directory_shape(git_dir: &Path, common: &Path, euid: u32) -> bool {
    let head = git_dir.join("HEAD");
    let Ok(meta) = fs::symlink_metadata(&head) else {
        return false;
    };
    if !meta.file_type().is_file() {
        return false;
    }
    let Ok(Some(content)) = read_capped(&head, HEAD_READ_BYTES) else {
        return false;
    };
    if !head_content_is_valid(&content) {
        return false;
    }
    searchable_dir(&common.join("objects"), euid) && searchable_dir(&common.join("refs"), euid)
}

/// git's `validate_headref`, on the bytes it reads: either `ref:`, any run of
/// C whitespace, then `refs/`; or 40 hex digits at the start (a 64-digit
/// sha256 id begins with 40). Nothing after either prefix is examined.
fn head_content_is_valid(bytes: &[u8]) -> bool {
    if let Some(rest) = bytes.strip_prefix(b"ref:") {
        let skipped = rest.iter().take_while(|b| is_c_space(**b)).count();
        if rest[skipped..].starts_with(b"refs/") {
            return true;
        }
    }
    bytes.len() >= 40 && bytes[..40].iter().all(u8::is_ascii_hexdigit)
}

/// C's `isspace` in the "C" locale.
fn is_c_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// A directory this uid owns and may search. `metadata` follows a symlink, as
/// git's `access` does.
fn searchable_dir(path: &Path, euid: u32) -> bool {
    fs::metadata(path)
        .map(|meta| meta.is_dir() && meta.uid() == euid && meta.mode() & 0o100 != 0)
        .unwrap_or(false)
}

// ── git's repository-format check ────────────────────────────────────────────

/// What a scan of one config file found. Every field is a reason to decline;
/// `worktree_config` additionally changes which file's `core.*` git honors.
#[derive(Debug, Default)]
struct FormatScan {
    /// A line the scanner could not follow with certainty. git may reject
    /// the file, or read it differently from a line-at-a-time scan.
    unparseable: bool,
    /// `core.repositoryformatversion` other than 0 or 1, or an extension
    /// git does not know.
    unsupported_format: bool,
    /// `core.bare` other than `false`, or `core.worktree` at all.
    core_override: bool,
    /// `extensions.worktreeConfig` is mentioned.
    worktree_config: bool,
}

/// Mirror `check_repository_format_gently`: read `<common>/config` (absent
/// is fine – git treats a missing config as a plain repository) and
/// `<gitdir>/config.worktree` when present, and decline on anything that
/// would move git's roots or make git refuse the repository.
///
/// `has_common` is whether `git_dir` has a `commondir`, i.e. this is a
/// linked worktree. git reads `core.bare` and `core.worktree` from the
/// common config only when there is NO commondir – except under
/// `extensions.worktreeConfig`, when it honors them again (and reads
/// `config.worktree` on top). The scan is conservative in the direction
/// git's own rule is not: it treats any mention of the extension as
/// enabled, and reads `config.worktree` whenever the file exists.
fn repository_format_check(
    git_dir: &Path,
    common: &Path,
    has_common: bool,
) -> Result<(), &'static str> {
    let scan = match fs::read(common.join("config")) {
        Ok(bytes) => scan_format(&String::from_utf8_lossy(&bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => FormatScan::default(),
        Err(_) => return Err("repository config could not be read"),
    };
    if scan.unparseable {
        return Err("repository config could not be scanned");
    }
    if scan.unsupported_format {
        return Err("repository config declares an unsupported format");
    }
    let honors_core = !has_common || scan.worktree_config;
    if honors_core && scan.core_override {
        return Err("repository config sets core.bare or core.worktree");
    }

    match fs::read(git_dir.join("config.worktree")) {
        Ok(bytes) => {
            let scan = scan_format(&String::from_utf8_lossy(&bytes));
            if scan.unparseable {
                return Err("repository config could not be scanned");
            }
            if scan.core_override {
                return Err("repository config sets core.bare or core.worktree");
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err("repository config could not be read"),
    }
    Ok(())
}

/// A conservative, line-at-a-time scan of a git config file for the handful
/// of keys that change discovery. It is not a config parser: whenever a line
/// is not plainly `[section]`, `key = value`, a comment or blank, the scan
/// reports `unparseable` and the caller declines. The one subtlety it does
/// reproduce is that git parses a key on the SAME line as its section header
/// (`[core] bare = true`), which a header-only line check would miss.
///
/// Keys are matched in every section, not only `[core]`: `bare` and
/// `worktree` mean nothing elsewhere, and matching them anyway can only add
/// declines. The extension list is the exception, since it is a rule about
/// `[extensions]` specifically.
fn scan_format(text: &str) -> FormatScan {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut scan = FormatScan::default();
    let mut section: Option<String> = None;

    for raw in text.split('\n') {
        let mut line = raw.trim();
        if let Some(after_bracket) = line.strip_prefix('[') {
            let Some(close) = after_bracket.find(']') else {
                scan.unparseable = true;
                return scan;
            };
            let name = after_bracket[..close]
                .split(|c: char| c == ' ' || c == '\t' || c == '.')
                .next()
                .unwrap_or("");
            section = Some(name.to_ascii_lowercase());
            line = after_bracket[close + 1..].trim();
        }
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let Some(section) = section.as_deref() else {
            // A key before any section header is a config error to git.
            scan.unparseable = true;
            return scan;
        };
        let (key, value) = match line.split_once('=') {
            Some((key, value)) => (key.trim(), Some(value.trim())),
            None => (line, None),
        };
        let key = key.to_ascii_lowercase();
        let key_shape_ok = key.bytes().next().is_some_and(|b| b.is_ascii_alphabetic())
            && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if !key_shape_ok {
            scan.unparseable = true;
            return scan;
        }
        // A value ending in a backslash continues on the next line, which
        // could hide a header or a key from a line-at-a-time scan.
        if value.is_some_and(|v| v.ends_with('\\')) {
            scan.unparseable = true;
            return scan;
        }

        match key.as_str() {
            "bare" => {
                if !value.is_some_and(|v| v.eq_ignore_ascii_case("false")) {
                    scan.core_override = true;
                }
            }
            "worktree" => scan.core_override = true,
            "repositoryformatversion" => {
                if !matches!(value, Some("0") | Some("1")) {
                    scan.unsupported_format = true;
                }
            }
            _ => {}
        }
        if section == "extensions" {
            if key == "worktreeconfig" {
                scan.worktree_config = true;
            }
            if !KNOWN_EXTENSIONS.contains(&key.as_str()) {
                scan.unsupported_format = true;
            }
        }
    }
    scan
}

// ── Small filesystem helpers ──────────────────────────────────────────────────

/// Read `path` whole, up to `cap` bytes: `Ok(None)` when the file is larger.
fn read_capped(path: &Path, cap: usize) -> io::Result<Option<Vec<u8>>> {
    let mut file = fs::File::open(path)?;
    let mut buf = Vec::with_capacity(cap.min(512) + 1);
    file.by_ref().take(cap as u64 + 1).read_to_end(&mut buf)?;
    if buf.len() > cap {
        return Ok(None);
    }
    Ok(Some(buf))
}

/// git's trailing-newline strip for gitfile and commondir contents: every
/// trailing `\n` or `\r`, and nothing else.
fn trim_trailing_newlines(bytes: &[u8]) -> &[u8] {
    let end = bytes
        .iter()
        .rposition(|b| *b != b'\n' && *b != b'\r')
        .map_or(0, |i| i + 1);
    &bytes[..end]
}

/// Resolve a path from a gitfile or commondir against the directory it is
/// relative to, then canonicalize. A path that is not valid UTF-8 is not
/// something this walk will name.
fn resolve_against(bytes: &[u8], base: &Path) -> io::Result<PathBuf> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "non-UTF-8 path"))?;
    let path = Path::new(text);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    joined.canonicalize()
}

/// Whether `path` (followed) is owned by `euid`.
fn owned_by(path: &Path, euid: u32) -> bool {
    fs::metadata(path)
        .map(|meta| meta.uid() == euid)
        .unwrap_or(false)
}

/// Whether `a` and `b` are on the same filesystem.
fn same_device(a: &Path, b: &Path) -> io::Result<bool> {
    Ok(fs::metadata(a)?.dev() == fs::metadata(b)?.dev())
}

/// This process's effective uid, from the raw `geteuid` – shared with the
/// plugin's temp-root validation (`tmproot`), which checks every managed
/// component is owned by exactly this uid. Not shelled out to `id -u`: both
/// callers sit on paths that run on every hook invocation, and a subprocess
/// there is slower and one more thing that can fail (`id` missing or
/// behaving unexpectedly) for a fact the process already knows about itself.
/// `geteuid` takes no arguments, cannot fail, and has no side effects, and
/// every platform these binaries target (Linux, macOS) already links the
/// libc that defines it as part of the Rust standard library's own runtime –
/// so this needs no new crate dependency, just the raw C declaration.
pub fn effective_uid() -> u32 {
    // SAFETY: `geteuid()` is a pure, argument-free POSIX call with no
    // preconditions and no failure mode.
    unsafe { geteuid() }
}

extern "C" {
    fn geteuid() -> u32;
}

#[cfg(test)]
mod tests;
