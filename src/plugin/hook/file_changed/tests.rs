//! Drives [`handle_with`] directly against a constructed [`HookContext`], the
//! way `pre_compact/tests.rs` does — the gates in front of every handler are
//! already covered by `hook/tests.rs`, so these tests are about what THIS
//! handler decides once it is reached.
//!
//! ## No test here executes repository-authored shell, by construction
//!
//! This is the unit's binding constraint, so it is worth being precise about
//! how it is met rather than asserting it once and hoping.
//!
//! The only code path in this module that could ever evaluate an `.envrc` is
//! `direnv export`, and `handle_with` takes the program it spawns as a
//! parameter. Every test passes [`shim`]'s stand-in script — never the real
//! `direnv` — and that script only ever `cat`s a canned answer. There is no
//! configuration of these tests in which real direnv runs.
//!
//! On top of that structural guarantee, every `.envrc` written here contains a
//! line that would create a witness file if anything ever sourced it, and
//! [`Shim::assert_nothing_executed`] checks that the witness is absent. So the
//! claim is an assertion about observed behaviour, not only about wiring. The
//! same helper asserts that no invocation ever asked direnv to *grant* trust,
//! by scanning the recorded argv of every call for the trust-granting verbs —
//! the same technique `scripts/test-bootstrap.sh` uses to prove a URL was
//! never composed.

use std::cell::RefCell;
use std::fs;
use std::os::unix::fs::PermissionsExt;

use tempfile::TempDir;

use super::*;
use crate::plugin::config::PluginConfig;
use crate::plugin::hook::event::{Common, Envelope, FileChanged as FileChangedPayload, Response};
use crate::plugin::HookEvent;

const NOW: u64 = 1_788_091_200; // 2026-08-30 12:00:00 UTC, arbitrary and fixed.

const EVENT: HookEvent = HookEvent::FileChanged;

/// Written into every `.envrc` these tests create. If anything ever sourced
/// one, the witness file appears and [`Shim::assert_nothing_executed`] fails.
const WITNESS: &str = "EXECUTED";

// ── Fixtures ─────────────────────────────────────────────────────────────────

/// A stand-in `direnv`: it answers `status` and `export` from canned files and
/// records every argv it was called with.
struct Shim {
    dir: TempDir,
}

impl Shim {
    /// `allowed` is the code direnv reports for the rc file it found, or
    /// `None` to answer as if it found no rc file at all. `export` is the
    /// shell text `direnv export bash` hands back.
    fn new(allowed: Option<i64>, export: &str) -> Self {
        Self::build(allowed, export, false)
    }

    /// A direnv that is installed but fails whatever it is asked.
    fn failing() -> Self {
        Self::build(Some(ALLOWED), "", true)
    }

    fn build(allowed: Option<i64>, export: &str, fail: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let found = match allowed {
            Some(code) => format!(r#"{{"allowed":{code},"path":"/rc/.envrc"}}"#),
            None => "null".to_string(),
        };
        fs::write(
            dir.path().join("status.json"),
            format!(r#"{{"state":{{"foundRC":{found},"loadedRC":null}}}}"#),
        )
        .unwrap();
        fs::write(dir.path().join("export.sh"), export).unwrap();

        let base = dir.path().display();
        // Records argv first so a refused call is still visible in the log,
        // then answers from the canned files. It never reads, sources or even
        // looks at the .envrc it is being asked about.
        let body = format!(
            "#!/bin/sh\n\
             printf '%s\\n' \"$*\" >> \"{base}/calls.log\"\n\
             {fail_line}\
             case \"$1\" in\n\
             status) cat \"{base}/status.json\" ;;\n\
             export) cat \"{base}/export.sh\" ;;\n\
             *) exit 9 ;;\n\
             esac\n",
            fail_line = if fail { "exit 1\n" } else { "" },
        );
        let program = dir.path().join("direnv");
        fs::write(&program, body).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        Self { dir }
    }

    fn program(&self) -> String {
        self.dir.path().join("direnv").display().to_string()
    }

    fn calls(&self) -> String {
        fs::read_to_string(self.dir.path().join("calls.log")).unwrap_or_default()
    }

    /// The two things that must never happen, checked together because they
    /// are the point of the whole unit.
    fn assert_nothing_executed(&self, worktree: &Path) {
        assert!(
            !worktree.join(WITNESS).exists(),
            "an .envrc was executed: the witness file exists"
        );
        for verb in ["allow", "permit", "grant"] {
            assert!(
                !self.calls().contains(verb),
                "direnv was asked to {verb}; trust must only ever come from the user"
            );
        }
    }

    fn assert_never_exported(&self) {
        assert!(
            !self.calls().contains("export"),
            "direnv export ran even though the rc file was not allowed"
        );
    }
}

/// A directory standing in for a worktree, holding an `.envrc` that would
/// announce itself if it were ever run.
fn worktree_with_rc(name: &str) -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(
        root.join(name),
        format!(
            "touch {}\nexport SECRET=hunter2\n",
            root.join(WITNESS).display()
        ),
    )
    .unwrap();
    (dir, root)
}

/// Somewhere outside the worktree for the harness's env file to live — which
/// is where R92 requires it to be.
fn outside_target(name: &str) -> (TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().canonicalize().unwrap().join(name);
    let display = path.display().to_string();
    (dir, display)
}

fn envelope_for(cwd: &Path, changed: Option<&str>, verb: &str) -> Envelope {
    Envelope {
        common: Common {
            session_id: "sess-1".to_string(),
            transcript_path: String::new(),
            cwd: cwd.to_string_lossy().into_owned(),
            hook_event_name: "FileChanged".to_string(),
            prompt_id: None,
        },
        payload: Payload::FileChanged(FileChangedPayload {
            file_path: changed.map(str::to_string),
            file_paths: Vec::new(),
            event: Some(verb.to_string()),
        }),
        raw: serde_json::json!({}),
    }
}

fn ctx_for<'a>(
    envelope: &'a Envelope,
    repo_root: Option<PathBuf>,
    config: &'a PluginConfig,
) -> HookContext<'a> {
    let config_root = repo_root.clone().unwrap_or_else(|| PathBuf::from("/"));
    HookContext {
        event: &EVENT,
        envelope,
        repo_root,
        config_root,
        config,
        now: NOW,
        diagnostics: RefCell::new(Vec::new()),
    }
}

/// The whole flow, with the two pieces of outside world named: which direnv,
/// and which env file the harness supplied.
fn run(shim: &Shim, worktree: &Path, changed: &Path, verb: &str, target: &str) -> Outcome {
    let envelope = envelope_for(worktree, Some(&changed.display().to_string()), verb);
    let config = PluginConfig::default();
    let ctx = ctx_for(&envelope, Some(worktree.to_path_buf()), &config);
    handle_with(&ctx, &shim.program(), target).unwrap()
}

fn detail(outcome: &Outcome) -> String {
    outcome.detail.clone().unwrap_or_default()
}

// ── Routing ──────────────────────────────────────────────────────────────────

/// U11 wires `route()` off the event name alone, so this is the one place that
/// proves the wiring landed on this module — and that R63 still classifies the
/// event as not state-writing, because it only ever writes the harness's file.
#[test]
fn file_changed_routes_through_this_handler() {
    let route = crate::plugin::hook::route(&HookEvent::FileChanged).unwrap();
    assert_eq!(route.handler as *const (), handle as *const ());
    assert!(
        !route.writes_state,
        "file-changed writes only the harness-supplied env file"
    );
}

// ── AE77: an un-allowed .envrc is never executed and never exported ──────────

/// The scenario the whole unit exists for: a repository was just cloned, its
/// `.envrc` has never been allowed, and a session opens on it. Nothing may be
/// executed, nothing may be exported, and the refusal must be recorded.
#[test]
fn ae77_an_un_allowed_envrc_is_never_executed_and_never_exported() {
    let shim = Shim::new(Some(1), "export SECRET=hunter2\n");
    let (_wt, worktree) = worktree_with_rc(".envrc");
    let (_t, target) = outside_target("env.sh");

    let outcome = run(
        &shim,
        &worktree,
        &worktree.join(".envrc"),
        "change",
        &target,
    );

    assert_eq!(outcome.response, Response::Silent);
    assert!(
        detail(&outcome).contains("does not report"),
        "expected a refusal, got: {}",
        detail(&outcome)
    );
    assert!(
        !Path::new(&target).exists(),
        "the env file was created despite the refusal"
    );
    shim.assert_never_exported();
    shim.assert_nothing_executed(&worktree);
}

/// An rc file whose trust was explicitly revoked (direnv's code 2) is refused
/// exactly like one that was never allowed. The gate tests for the single
/// permitted code rather than excluding the known-bad ones, so an unrecognized
/// future code refuses too.
#[test]
fn an_explicitly_revoked_envrc_is_refused() {
    for code in [1, 2, 7] {
        let shim = Shim::new(Some(code), "export SECRET=hunter2\n");
        let (_wt, worktree) = worktree_with_rc(".envrc");
        let (_t, target) = outside_target("env.sh");

        let outcome = run(
            &shim,
            &worktree,
            &worktree.join(".envrc"),
            "change",
            &target,
        );

        assert!(
            detail(&outcome).contains("does not report"),
            "code {code} should refuse, got: {}",
            detail(&outcome)
        );
        assert!(!Path::new(&target).exists(), "code {code} wrote the file");
        shim.assert_never_exported();
        shim.assert_nothing_executed(&worktree);
    }
}

/// direnv finding no rc file at all answers `foundRC: null`, which carries no
/// `allowed` code. That unknown answer must refuse like any other.
#[test]
fn an_unknown_allow_state_refuses() {
    let shim = Shim::new(None, "export SECRET=hunter2\n");
    let (_wt, worktree) = worktree_with_rc(".envrc");
    let (_t, target) = outside_target("env.sh");

    let outcome = run(
        &shim,
        &worktree,
        &worktree.join(".envrc"),
        "change",
        &target,
    );

    assert!(detail(&outcome).contains("unknown"), "{}", detail(&outcome));
    shim.assert_never_exported();
    shim.assert_nothing_executed(&worktree);
}

// ── The allowed path ─────────────────────────────────────────────────────────

/// The happy path: direnv already reports the rc file as allowed, so the
/// export runs and lands in the harness's file — and even here, no `.envrc`
/// is executed by these tests, because the shim answers from a canned file.
#[test]
fn an_allowed_envrc_exports_into_the_harness_file() {
    let shim = Shim::new(Some(0), "export SECRET=hunter2\n");
    let (_wt, worktree) = worktree_with_rc(".envrc");
    let (_t, target) = outside_target("env.sh");

    let outcome = run(
        &shim,
        &worktree,
        &worktree.join(".envrc"),
        "change",
        &target,
    );

    assert_eq!(outcome.response, Response::Silent);
    assert!(
        detail(&outcome).contains("appended"),
        "{}",
        detail(&outcome)
    );
    let written = fs::read_to_string(&target).unwrap();
    assert!(written.contains("export SECRET=hunter2"), "{written}");
    shim.assert_nothing_executed(&worktree);
}

/// R92's "never copies exported values into ss-magic's state, heartbeat or
/// ledger". The heartbeat detail is the handler's only channel into any of
/// those, so the exported value must not appear anywhere in it.
#[test]
fn the_exported_values_never_reach_the_heartbeat_detail() {
    let shim = Shim::new(Some(0), "export SECRET=hunter2\nexport TOKEN=abc123\n");
    let (_wt, worktree) = worktree_with_rc(".envrc");
    let (_t, target) = outside_target("env.sh");

    let outcome = run(
        &shim,
        &worktree,
        &worktree.join(".envrc"),
        "change",
        &target,
    );

    let detail = detail(&outcome);
    for leaked in ["hunter2", "abc123", "SECRET", "TOKEN"] {
        assert!(
            !detail.contains(leaked),
            "heartbeat detail leaked {leaked}: {detail}"
        );
    }
}

/// A `.env` change goes through the identical allow gate, because direnv
/// treats a bare `.env` as an rc file in its own right and will not load one
/// the user has not allowed either.
#[test]
fn a_dot_env_change_takes_the_same_allow_gate() {
    let (_wt, worktree) = worktree_with_rc(".env");
    let (_t, allowed_target) = outside_target("env.sh");
    let shim = Shim::new(Some(0), "export FROM_DOTENV=1\n");
    let outcome = run(
        &shim,
        &worktree,
        &worktree.join(".env"),
        "change",
        &allowed_target,
    );
    assert!(
        detail(&outcome).contains("appended"),
        "{}",
        detail(&outcome)
    );
    assert!(fs::read_to_string(&allowed_target)
        .unwrap()
        .contains("FROM_DOTENV"));

    let (_wt2, worktree2) = worktree_with_rc(".env");
    let (_t2, refused_target) = outside_target("env.sh");
    let refusing = Shim::new(Some(1), "export FROM_DOTENV=1\n");
    let outcome = run(
        &refusing,
        &worktree2,
        &worktree2.join(".env"),
        "change",
        &refused_target,
    );
    assert!(
        detail(&outcome).contains("does not report"),
        "{}",
        detail(&outcome)
    );
    refusing.assert_never_exported();
}

/// Allow state is per rc file, so one worktree of a repository can be allowed
/// while its sibling is not. The handler asks about the directory the changed
/// file is in and follows that answer, rather than caching a repository-wide
/// verdict — the sibling must still be refused.
#[test]
fn an_envrc_allowed_in_one_worktree_is_not_allowed_in_a_sibling() {
    let (_wt_a, worktree_a) = worktree_with_rc(".envrc");
    let (_t_a, target_a) = outside_target("a.sh");
    let allowed = Shim::new(Some(0), "export WHICH=a\n");
    let outcome = run(
        &allowed,
        &worktree_a,
        &worktree_a.join(".envrc"),
        "change",
        &target_a,
    );
    assert!(
        detail(&outcome).contains("appended"),
        "{}",
        detail(&outcome)
    );

    let (_wt_b, worktree_b) = worktree_with_rc(".envrc");
    let (_t_b, target_b) = outside_target("b.sh");
    let not_allowed = Shim::new(Some(1), "export WHICH=b\n");
    let outcome = run(
        &not_allowed,
        &worktree_b,
        &worktree_b.join(".envrc"),
        "change",
        &target_b,
    );
    assert!(
        detail(&outcome).contains("does not report"),
        "the sibling worktree exported without its own allow: {}",
        detail(&outcome)
    );
    assert!(!Path::new(&target_b).exists());
    not_allowed.assert_never_exported();
    not_allowed.assert_nothing_executed(&worktree_b);
}

// ── The target: append, outside the worktree, owner-only ─────────────────────

/// The env file is a script the harness runs before every Bash command and may
/// already carry another hook's exports. Truncating it would drop them
/// silently, so the write has to be an append.
#[test]
fn the_export_is_appended_never_truncated() {
    let shim = Shim::new(Some(0), "export ADDED=2\n");
    let (_wt, worktree) = worktree_with_rc(".envrc");
    let (_t, target) = outside_target("env.sh");
    fs::write(&target, "export PRIOR=1\n").unwrap();

    run(
        &shim,
        &worktree,
        &worktree.join(".envrc"),
        "change",
        &target,
    );

    let written = fs::read_to_string(&target).unwrap();
    assert!(
        written.contains("export PRIOR=1"),
        "prior content lost: {written}"
    );
    assert!(written.contains("export ADDED=2"), "{written}");
    assert!(
        written.find("PRIOR").unwrap() < written.find("ADDED").unwrap(),
        "the append landed before existing content: {written}"
    );
}

/// Repeated events keep appending rather than replacing, which is what makes
/// two hooks sharing one file safe.
#[test]
fn repeated_events_keep_appending() {
    let shim = Shim::new(Some(0), "export N=1\n");
    let (_wt, worktree) = worktree_with_rc(".envrc");
    let (_t, target) = outside_target("env.sh");

    for _ in 0..3 {
        run(
            &shim,
            &worktree,
            &worktree.join(".envrc"),
            "change",
            &target,
        );
    }

    let written = fs::read_to_string(&target).unwrap();
    assert_eq!(written.matches("export N=1").count(), 3, "{written}");
}

/// The refusal R92 is most emphatic about. A target inside the worktree is
/// where `git add .` finds it, so exported secrets must never be written
/// there — no matter that the harness is the one that named the path.
#[test]
fn a_target_inside_the_worktree_is_refused() {
    let shim = Shim::new(Some(0), "export SECRET=hunter2\n");
    let (_wt, worktree) = worktree_with_rc(".envrc");
    let inside = worktree.join("env.sh").display().to_string();

    let outcome = run(
        &shim,
        &worktree,
        &worktree.join(".envrc"),
        "change",
        &inside,
    );

    assert!(
        detail(&outcome).contains("resolves inside"),
        "{}",
        detail(&outcome)
    );
    assert!(!Path::new(&inside).exists(), "wrote inside the worktree");
    shim.assert_never_exported();
    shim.assert_nothing_executed(&worktree);
}

/// The same refusal must survive a symlinked parent directory pointing back
/// into the worktree — which is why the resolver canonicalizes the parent
/// before the boundary is checked rather than comparing the raw string.
#[test]
fn a_target_reaching_the_worktree_through_a_symlink_is_refused() {
    let shim = Shim::new(Some(0), "export SECRET=hunter2\n");
    let (_wt, worktree) = worktree_with_rc(".envrc");
    let link_home = tempfile::tempdir().unwrap();
    let link = link_home.path().join("into-worktree");
    std::os::unix::fs::symlink(&worktree, &link).unwrap();
    let sneaky = link.join("env.sh").display().to_string();

    let outcome = run(
        &shim,
        &worktree,
        &worktree.join(".envrc"),
        "change",
        &sneaky,
    );

    assert!(
        detail(&outcome).contains("resolves inside"),
        "a symlinked parent walked past the boundary check: {}",
        detail(&outcome)
    );
    assert!(!worktree.join("env.sh").exists());
    shim.assert_never_exported();
}

/// These are secrets; the file that holds them is created owner-only.
#[test]
fn the_created_env_file_is_owner_only() {
    let shim = Shim::new(Some(0), "export SECRET=hunter2\n");
    let (_wt, worktree) = worktree_with_rc(".envrc");
    let (_t, target) = outside_target("env.sh");

    run(
        &shim,
        &worktree,
        &worktree.join(".envrc"),
        "change",
        &target,
    );

    let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "env file mode was {mode:o}");
}

/// R92: with no harness-supplied target the handler does nothing. It must not
/// invent a path, and it must not consult direnv either — there is nowhere to
/// put an answer.
#[test]
fn no_harness_supplied_env_file_is_a_no_op() {
    for target in ["", "   "] {
        let shim = Shim::new(Some(0), "export SECRET=hunter2\n");
        let (_wt, worktree) = worktree_with_rc(".envrc");

        let outcome = run(&shim, &worktree, &worktree.join(".envrc"), "change", target);

        assert!(
            detail(&outcome).contains("supplied no"),
            "{}",
            detail(&outcome)
        );
        assert_eq!(
            shim.calls(),
            "",
            "direnv was consulted with nowhere to write"
        );
        shim.assert_nothing_executed(&worktree);
    }
}

/// A relative target would have to be resolved against a guess, and every
/// guess here is a guess about where to put secrets.
#[test]
fn a_relative_target_is_refused() {
    let shim = Shim::new(Some(0), "export SECRET=hunter2\n");
    let (_wt, worktree) = worktree_with_rc(".envrc");

    let outcome = run(
        &shim,
        &worktree,
        &worktree.join(".envrc"),
        "change",
        "relative/env.sh",
    );

    assert!(
        detail(&outcome).contains("not absolute"),
        "{}",
        detail(&outcome)
    );
    shim.assert_never_exported();
}

/// A target whose directory does not exist is refused rather than created:
/// creating a tree to hold secrets somewhere nobody asked for is exactly the
/// "write them somewhere reasonable" failure R92 names.
#[test]
fn a_target_in_a_missing_directory_is_refused() {
    let shim = Shim::new(Some(0), "export SECRET=hunter2\n");
    let (_wt, worktree) = worktree_with_rc(".envrc");
    let (_t, base) = outside_target("nowhere");
    let missing = format!("{base}/deeper/env.sh");

    let outcome = run(
        &shim,
        &worktree,
        &worktree.join(".envrc"),
        "change",
        &missing,
    );

    assert!(
        detail(&outcome).contains("could not be resolved"),
        "{}",
        detail(&outcome)
    );
    assert!(!Path::new(&missing).exists());
    shim.assert_never_exported();
}

// ── Which changes the handler reacts to ──────────────────────────────────────

/// The event fires for every watched path, so a change to anything that is not
/// an rc file must cost one string comparison and stop there — without
/// consulting direnv at all.
#[test]
fn a_change_to_something_else_is_ignored() {
    for name in ["README.md", ".envrc.bak", "env", "notes.env.txt"] {
        let shim = Shim::new(Some(0), "export SECRET=hunter2\n");
        let (_wt, worktree) = worktree_with_rc(".envrc");
        let (_t, target) = outside_target("env.sh");

        let outcome = run(&shim, &worktree, &worktree.join(name), "change", &target);

        assert!(
            detail(&outcome).contains("no .env or .envrc"),
            "{name}: {}",
            detail(&outcome)
        );
        assert_eq!(shim.calls(), "", "{name} consulted direnv");
        assert!(!Path::new(&target).exists(), "{name} wrote the env file");
    }
}

/// A deleted rc file has nothing to export. Re-exporting whatever ancestor
/// direnv would fall back to is not what a deletion asked for.
#[test]
fn an_unlinked_rc_file_exports_nothing() {
    let shim = Shim::new(Some(0), "export SECRET=hunter2\n");
    let (_wt, worktree) = worktree_with_rc(".envrc");
    let (_t, target) = outside_target("env.sh");

    let outcome = run(
        &shim,
        &worktree,
        &worktree.join(".envrc"),
        "unlink",
        &target,
    );

    assert!(detail(&outcome).contains("removed"), "{}", detail(&outcome));
    assert_eq!(shim.calls(), "");
    assert!(!Path::new(&target).exists());
}

/// `add` is the verb for a newly created rc file, and is treated as a change.
#[test]
fn an_added_rc_file_is_treated_as_a_change() {
    let shim = Shim::new(Some(0), "export SECRET=hunter2\n");
    let (_wt, worktree) = worktree_with_rc(".envrc");
    let (_t, target) = outside_target("env.sh");

    let outcome = run(&shim, &worktree, &worktree.join(".envrc"), "add", &target);

    assert!(
        detail(&outcome).contains("appended"),
        "{}",
        detail(&outcome)
    );
}

/// A relative changed path is not resolvable against anything trustworthy, and
/// a path whose directory has gone is not worth asking direnv about.
#[test]
fn an_unusable_changed_path_is_ignored() {
    let shim = Shim::new(Some(0), "export SECRET=hunter2\n");
    let (_wt, worktree) = worktree_with_rc(".envrc");
    let (_t, target) = outside_target("env.sh");

    for changed in [".envrc", "/definitely/not/here/.envrc"] {
        let outcome = run(&shim, &worktree, Path::new(changed), "change", &target);
        assert!(
            detail(&outcome).contains("not an absolute path in an existing directory"),
            "{changed}: {}",
            detail(&outcome)
        );
    }
    assert_eq!(shim.calls(), "");
}

// ── direnv absent or failing ─────────────────────────────────────────────────

/// R92: absent direnv is a silent no-op with a heartbeat row. Nothing is
/// written and the invocation still succeeds.
#[test]
fn direnv_absent_is_a_silent_no_op() {
    let (_wt, worktree) = worktree_with_rc(".envrc");
    let (_t, target) = outside_target("env.sh");
    let envelope = envelope_for(
        &worktree,
        Some(&worktree.join(".envrc").display().to_string()),
        "change",
    );
    let config = PluginConfig::default();
    let ctx = ctx_for(&envelope, Some(worktree.clone()), &config);

    let outcome = handle_with(&ctx, "/definitely/not/a/real/direnv/binary", &target).unwrap();

    assert_eq!(outcome.response, Response::Silent);
    assert!(
        detail(&outcome).contains("not installed"),
        "{}",
        detail(&outcome)
    );
    assert!(!Path::new(&target).exists());
    assert!(!worktree.join(WITNESS).exists());
}

/// direnv installed but failing must not be read as permission. A non-zero
/// `status` leaves the allow state unknown, and unknown refuses.
#[test]
fn direnv_failing_the_status_query_refuses() {
    let shim = Shim::failing();
    let (_wt, worktree) = worktree_with_rc(".envrc");
    let (_t, target) = outside_target("env.sh");

    let outcome = run(
        &shim,
        &worktree,
        &worktree.join(".envrc"),
        "change",
        &target,
    );

    assert!(detail(&outcome).contains("unknown"), "{}", detail(&outcome));
    assert!(!Path::new(&target).exists());
    shim.assert_never_exported();
    shim.assert_nothing_executed(&worktree);
}

/// An export that fails after an allowed status leaves the env file untouched
/// rather than appending a half-written block.
#[test]
fn a_failing_export_appends_nothing() {
    // `status` succeeds and reports allowed; `export` is answered by a script
    // that exits non-zero.
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("status.json"),
        r#"{"state":{"foundRC":{"allowed":0,"path":"/rc/.envrc"},"loadedRC":null}}"#,
    )
    .unwrap();
    let base = dir.path().display();
    let program = dir.path().join("direnv");
    fs::write(
        &program,
        format!(
            "#!/bin/sh\n\
             case \"$1\" in\n\
             status) cat \"{base}/status.json\" ;;\n\
             export) exit 3 ;;\n\
             *) exit 9 ;;\n\
             esac\n"
        ),
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();

    let (_wt, worktree) = worktree_with_rc(".envrc");
    let (_t, target) = outside_target("env.sh");
    let envelope = envelope_for(
        &worktree,
        Some(&worktree.join(".envrc").display().to_string()),
        "change",
    );
    let config = PluginConfig::default();
    let ctx = ctx_for(&envelope, Some(worktree.clone()), &config);

    let outcome = handle_with(&ctx, &program.display().to_string(), &target).unwrap();

    assert!(
        detail(&outcome).contains("export failed"),
        "{}",
        detail(&outcome)
    );
    assert!(!Path::new(&target).exists());
}

/// An allowed rc file that exports nothing at all writes nothing, rather than
/// appending an empty block to the harness's file on every save.
#[test]
fn an_empty_export_writes_nothing() {
    let shim = Shim::new(Some(0), "\n  \n");
    let (_wt, worktree) = worktree_with_rc(".envrc");
    let (_t, target) = outside_target("env.sh");

    let outcome = run(
        &shim,
        &worktree,
        &worktree.join(".envrc"),
        "change",
        &target,
    );

    assert!(detail(&outcome).contains("empty"), "{}", detail(&outcome));
    assert!(!Path::new(&target).exists());
}

// ── The pure helpers ─────────────────────────────────────────────────────────

/// The allow codes, straight from direnv's own reporting: `0` allowed, `1`
/// never allowed, `2` revoked. Parsed from the exact shape direnv 2.37 emits.
#[test]
fn parse_status_reads_direnvs_shape() {
    let allowed = parse_status(
        r#"{"config":{},"state":{"foundRC":{"allowed":0,"path":"/w/.envrc"},"loadedRC":null}}"#,
    );
    assert_eq!(allowed.allowed, Some(0));
    assert_eq!(allowed.rc_path.as_deref(), Some("/w/.envrc"));

    let none = parse_status(r#"{"state":{"foundRC":null,"loadedRC":null}}"#);
    assert_eq!(none.allowed, None);
}

/// A reshaped, truncated or non-JSON answer degrades to "unknown", which the
/// caller refuses — it must never fail the invocation or read as permission.
#[test]
fn a_malformed_status_answer_is_unknown_not_allowed() {
    for body in [
        "",
        "not json",
        "{}",
        r#"{"state":{}}"#,
        r#"{"state":{"foundRC":{}}}"#,
    ] {
        assert_eq!(parse_status(body).allowed, None, "{body:?}");
    }
}

#[test]
fn only_the_two_rc_names_match() {
    for yes in ["/w/.envrc", "/w/.env", ".envrc"] {
        assert!(is_rc_path(yes), "{yes}");
    }
    for no in ["/w/.envrc.bak", "/w/env", "/w/.environment", "/w/a.env", ""] {
        assert!(!is_rc_path(no), "{no}");
    }
}

/// The singular field the harness actually sends wins, but the batch spelling
/// U11 also typed is still honoured.
#[test]
fn the_first_rc_path_comes_from_either_spelling() {
    let singular = FileChangedPayload {
        file_path: Some("/w/.envrc".into()),
        file_paths: Vec::new(),
        event: None,
    };
    assert_eq!(first_rc_path(&singular).as_deref(), Some("/w/.envrc"));

    let batch = FileChangedPayload {
        file_path: None,
        file_paths: vec!["/w/README.md".into(), "/w/.env".into()],
        event: None,
    };
    assert_eq!(first_rc_path(&batch).as_deref(), Some("/w/.env"));

    let neither = FileChangedPayload {
        file_path: Some("/w/README.md".into()),
        file_paths: vec!["/w/src/main.rs".into()],
        event: None,
    };
    assert_eq!(first_rc_path(&neither), None);
}

/// An unresolvable boundary counts as containing the target — the fail-closed
/// direction, so a boundary nobody could resolve never lets a write through.
#[test]
fn an_unresolvable_boundary_is_treated_as_containing_the_target() {
    assert!(within(
        Path::new("/tmp/anywhere/env.sh"),
        Path::new("/definitely/not/here")
    ));
}
