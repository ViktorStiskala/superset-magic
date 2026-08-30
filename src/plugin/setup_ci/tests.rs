//! Two halves, and they check different things.
//!
//! The first half is about the **asset**: the workflow that ships is the
//! security artifact here, so its shape is asserted directly — the trigger, the
//! permissions on each job, which job checks out code, and the fact that no
//! repository-controlled text is ever interpolated into a shell. These read the
//! embedded template rather than a second copy of it. A checked-in golden file
//! would be a byte-for-byte duplicate of `assets/workflow/checklist.yml`, since
//! rendering is one string substitution on that file: editing the workflow
//! would then mean editing two identical files, and the copy that drifts is the
//! one nobody notices. The template *is* the golden file, and these assertions
//! are what a grep over it would be, kept honest by the compiler.
//!
//! The second half is about the **verb**: the four states, and what a run does
//! in each of them.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use tempfile::TempDir;

use super::*;
use crate::tests::support::git_run;

/// A version that is obviously not a real one, so a test failure cannot be
/// confused with the crate's actual version leaking into an assertion.
const V: &str = "9.9.9";

// ── Reading the template as a structure, without a YAML parser ────────────────

/// The body of one top-level job, from its `  <name>:` line up to the next
/// two-space-indented key at the same level (or the end of the file).
///
/// Line-based on purpose: the crate has no YAML reader, and pulling one in for
/// a handful of assertions would add a dependency to the shipped binary that
/// only the tests use.
fn job_body(name: &str) -> String {
    let header = format!("\n  {name}:\n");
    let start = TEMPLATE
        .find(&header)
        .unwrap_or_else(|| panic!("the workflow has no `{name}` job"))
        + 1;
    let rest = &TEMPLATE[start..];
    let mut out = String::new();
    for (i, line) in rest.lines().enumerate() {
        // The job's own header line, then everything indented under it.
        let ends =
            i > 0 && !line.trim().is_empty() && line.starts_with("  ") && !line.starts_with("   ");
        if ends {
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// `text` with its full-line comments removed.
///
/// The workflow explains its own security properties in prose, so the words
/// this file scans for - `write`, `pull_request_target` - appear in comments
/// describing why they are absent from the YAML. Scanning the comments would
/// mean the file could not document itself.
fn code(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every `run:` script in the workflow, as one string per step.
///
/// Handles both the block form (`run: |`) and an inline `run: cmd`, so a step
/// added in the inline form cannot slip past the interpolation check below.
fn run_blocks() -> Vec<String> {
    let mut blocks = Vec::new();
    let mut lines = TEMPLATE.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("run:") else {
            continue;
        };
        let indent = line.len() - trimmed.len();
        if rest.trim() != "|" && !rest.trim().is_empty() {
            blocks.push(rest.trim().to_string());
            continue;
        }
        let mut body = String::new();
        while let Some(next) = lines.peek() {
            let deeper = next.trim().is_empty() || next.len() - next.trim_start().len() > indent;
            if !deeper {
                break;
            }
            body.push_str(next);
            body.push('\n');
            lines.next();
        }
        blocks.push(body);
    }
    blocks
}

// ── The asset: the trigger and the two-job split ──────────────────────────────

/// The whole reason the workflow is shipped rather than described:
/// `pull_request_target` runs fork code against the base branch with a
/// write-capable token, which is the failure the split below exists to prevent.
#[test]
fn the_workflow_never_uses_the_dangerous_trigger() {
    assert!(
        !code(TEMPLATE).contains("pull_request_target"),
        "the workflow must never trigger on pull_request_target"
    );
    assert!(
        TEMPLATE.contains("\non:\n  pull_request:\n"),
        "the workflow must trigger on pull_request"
    );
}

/// Least privilege has to start from nothing: a job with no `permissions:` of
/// its own inherits the repository default, which is write on many
/// repositories.
#[test]
fn the_workflow_denies_every_permission_by_default() {
    assert!(
        TEMPLATE.contains("\npermissions: {}\n"),
        "the workflow needs a top-level deny-all permissions block"
    );
}

/// The security core: the job that runs pull-request code holds no write
/// scope, and the job that holds one runs no pull-request code.
#[test]
fn no_job_both_reads_pull_request_code_and_can_write() {
    let render_job = code(&job_body("render"));
    let comment_job = code(&job_body("comment"));

    assert!(
        render_job.contains("permissions:\n      contents: read\n"),
        "the render job must be read-only"
    );
    assert!(
        !render_job.contains("write"),
        "the render job checks out pull-request code, so it must hold no write scope:\n\
         {render_job}"
    );

    assert!(
        comment_job.contains("permissions:\n      pull-requests: write\n"),
        "the comment job needs pull-requests: write to post"
    );
    assert!(
        !comment_job.contains("actions/checkout"),
        "the comment job holds a write token and must check out nothing:\n{comment_job}"
    );
    // Nothing else may grant itself a scope either: exactly one `write` in the
    // whole file, and it is the comment job's.
    assert_eq!(
        code(TEMPLATE).matches("pull-requests: write").count(),
        1,
        "only the comment job may hold a write scope"
    );

    assert!(
        render_job.contains("actions/checkout"),
        "the render job is the one that checks the pull request out"
    );
    assert_eq!(
        code(TEMPLATE).matches("actions/checkout").count(),
        1,
        "only one job may check out code"
    );
}

/// The checked-out repository must not be able to read the job's token back
/// out of `.git/config` — a build script in pull-request code would otherwise
/// have it, read-only though it is.
#[test]
fn the_checkout_persists_no_credentials() {
    assert!(job_body("render").contains("persist-credentials: false"));
}

// ── The asset: no repository text reaches a shell ─────────────────────────────

/// A checklist's prose is written by whoever opened the pull request. Every
/// value derived from it travels as a file, so `${{ }}` — which pastes its
/// result into the script before bash ever sees it — must not appear inside a
/// `run:` block at all.
#[test]
fn no_run_step_interpolates_an_expression() {
    let blocks = run_blocks();
    assert!(!blocks.is_empty(), "the scanner found no run: steps");
    for block in &blocks {
        let block = code(block);
        assert!(
            !block.contains("${{"),
            "a run: step interpolates an expression, which is a shell-injection seam:\n{block}"
        );
    }
}

/// The rendered Markdown reaches `gh` as a file it opens itself, never as an
/// argument.
#[test]
fn the_comment_is_posted_from_a_file() {
    let comment_job = code(&job_body("comment"));
    assert!(comment_job.contains("--body-file"));
    assert!(
        !comment_job.contains("--body "),
        "the comment body must never be an argument"
    );
    // One comment per pull request, rewritten on each push.
    assert!(comment_job.contains("--edit-last"));
    assert!(comment_job.contains("--create-if-none"));
}

// ── The asset: the pinned, verified install ───────────────────────────────────

/// The downloaded binary is checked against the release's own published digest
/// before it is installed, and the pin is the only thing that names which
/// release.
#[test]
fn the_installed_binary_is_pinned_and_checksum_verified() {
    let render_job = job_body("render");
    assert!(
        render_job.contains("sha256sum --check"),
        "the downloaded archive must be checksum-verified before it is installed"
    );
    assert!(
        render_job.contains("releases/download/v$SS_MAGIC_VERSION"),
        "the download must be pinned to the version in the env block"
    );
    // `-f` is what turns a 404 into a failure instead of a saved error page.
    assert!(render_job.contains("curl -fsSL"));
}

/// The workflow decides whether there is anything to do by globbing the same
/// naming convention the binary falls back to when the (gitignored, so absent
/// in CI) pointer does not answer. Tying the two together here means a rename
/// of the convention cannot leave the workflow globbing a path nothing writes.
#[test]
fn the_workflow_globs_the_checklist_convention() {
    let glob = format!("{ACTIONS_REL}/*{CHECKLIST_SUFFIX}");
    assert!(
        TEMPLATE.contains(&glob),
        "the workflow should look for {glob}"
    );
}

// ── Rendering and reading the pin back ────────────────────────────────────────

#[test]
fn rendering_substitutes_every_placeholder() {
    let out = render(V);
    assert!(
        !out.contains(VERSION_PLACEHOLDER),
        "an unsubstituted placeholder would download a release that cannot exist"
    );
    assert!(out.contains(&format!("SS_MAGIC_VERSION: \"{V}\"")));
}

/// The template must carry the placeholder and no literal version of its own:
/// a hard-coded version in the asset would be a second copy of `Cargo.toml`'s
/// value with nothing keeping the two in step.
#[test]
fn the_template_pins_nothing_of_its_own() {
    assert_eq!(
        TEMPLATE.matches(VERSION_PLACEHOLDER).count(),
        1,
        "the pin belongs in exactly one place in the template"
    );
    assert_eq!(
        pinned_version(TEMPLATE).as_deref(),
        Some(VERSION_PLACEHOLDER)
    );
}

#[test]
fn the_pin_round_trips_through_a_rendered_file() {
    assert_eq!(pinned_version(&render(V)).as_deref(), Some(V));
}

/// A file with nothing recognisable in it yields no pin, which lands it in
/// `Differs` — the state that refuses to overwrite.
#[test]
fn an_unrecognisable_file_pins_nothing() {
    assert_eq!(pinned_version("name: something else\n"), None);
}

/// An unquoted pin is still a pin. Someone editing the file by hand will drop
/// the quotes, and reporting that as an unrelated local edit would be unhelpful.
#[test]
fn an_unquoted_pin_is_still_read() {
    assert_eq!(
        pinned_version("env:\n  SS_MAGIC_VERSION: 0.4.2\n").as_deref(),
        Some("0.4.2")
    );
}

// ── The four states ───────────────────────────────────────────────────────────

#[test]
fn no_file_is_absent() {
    assert_eq!(classify(None, V), State::Absent);
}

#[test]
fn the_current_workflow_is_identical() {
    assert_eq!(classify(Some(&render(V)), V), State::Identical);
}

#[test]
fn the_same_workflow_at_another_version_is_a_stale_pin() {
    let old = render("0.1.0");
    assert_eq!(
        classify(Some(&old), V),
        State::PinStale {
            found: "0.1.0".to_string()
        }
    );
}

/// A hand edit is `Differs` even when the pin is current — the pin being right
/// is not evidence that the rest of the file is.
#[test]
fn a_local_edit_at_the_current_pin_differs() {
    let edited = render(V).replace("runs-on: ubuntu-latest", "runs-on: self-hosted");
    assert_eq!(classify(Some(&edited), V), State::Differs);
}

/// Whitespace is not cosmetic in YAML — indentation is structure and a block
/// scalar carries its trailing newlines into the script — so a difference in
/// it is a real difference, not something to write through.
#[test]
fn a_whitespace_only_difference_still_differs() {
    let padded = format!("{}\n", render(V));
    assert_eq!(classify(Some(&padded), V), State::Differs);
}

/// A stale pin inside a file that was *also* edited is not a stale pin: the
/// re-render at the found version does not reproduce it, so it stays in the
/// state that asks first.
#[test]
fn an_edited_file_with_an_old_pin_differs() {
    let edited = render("0.1.0").replace("timeout-minutes: 10", "timeout-minutes: 30");
    assert_eq!(classify(Some(&edited), V), State::Differs);
}

// ── The verb, against a real repository ───────────────────────────────────────

/// An empty git repository, canonicalized so the path matches what
/// `git rev-parse --show-toplevel` reports on macOS.
fn repo() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    git_run(&["init", "-q", "-b", "main"], dir.path());
    let root = dir.path().canonicalize().unwrap();
    (dir, root)
}

fn workflow_path(root: &Path) -> PathBuf {
    root.join(WORKFLOW_REL)
}

/// Give the repository a checklist, so the advisory about the missing one does
/// not fire in tests that are not about it.
fn add_checklist(root: &Path) {
    let dir = root.join(ACTIONS_REL);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("2026-08-thing{CHECKLIST_SUFFIX}")), "{}\n").unwrap();
}

/// The bare case: no `.github/` at all, so the verb has to create the whole
/// directory chain.
#[test]
fn a_repository_with_no_workflows_directory_gets_one() {
    let (_d, root) = repo();
    add_checklist(&root);
    assert!(!root.join(".github").exists());

    assert_eq!(run_core(&root, V, false, false).unwrap(), ExitCode::SUCCESS);
    assert_eq!(fs::read_to_string(workflow_path(&root)).unwrap(), render(V));
}

/// Committed repository content, so world-readable — not the owner-only mode
/// the plugin's state tree uses.
#[test]
fn the_written_workflow_is_world_readable() {
    let (_d, root) = repo();
    run_core(&root, V, false, false).unwrap();
    let mode = fs::metadata(workflow_path(&root))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o644, "the workflow is committed content");
}

/// A second run against a workflow it already wrote changes nothing at all —
/// not the bytes, and not the file's mtime, so it does not show up as a
/// modification in a working copy.
#[test]
fn a_second_run_against_an_identical_workflow_writes_nothing() {
    let (_d, root) = repo();
    add_checklist(&root);
    run_core(&root, V, false, false).unwrap();

    let path = workflow_path(&root);
    let before = fs::metadata(&path).unwrap().modified().unwrap();

    assert_eq!(run_core(&root, V, false, false).unwrap(), ExitCode::SUCCESS);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
    assert_eq!(fs::read_to_string(&path).unwrap(), render(V));
}

/// AE78, the reporting half: `--check` against an older pin reports it and
/// leaves the file exactly as it was.
#[test]
fn check_against_a_stale_pin_writes_nothing() {
    let (_d, root) = repo();
    add_checklist(&root);
    let path = workflow_path(&root);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let old = render("0.1.0");
    fs::write(&path, &old).unwrap();

    assert_eq!(classify(Some(&old), V).token(), "pin-stale");
    assert_eq!(run_core(&root, V, true, false).unwrap(), ExitCode::SUCCESS);
    assert_eq!(fs::read_to_string(&path).unwrap(), old);
}

/// AE78, the writing half: the run that follows the confirmation advances the
/// pin, and the result is the current workflow in full.
#[test]
fn a_stale_pin_is_advanced_on_a_write_run() {
    let (_d, root) = repo();
    add_checklist(&root);
    let path = workflow_path(&root);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, render("0.1.0")).unwrap();

    assert_eq!(run_core(&root, V, false, false).unwrap(), ExitCode::SUCCESS);
    let written = fs::read_to_string(&path).unwrap();
    assert_eq!(written, render(V));
    assert_eq!(pinned_version(&written).as_deref(), Some(V));
    assert!(!code(&written).contains("pull_request_target"));
}

/// The one destructive case needs a flag. Without it the local file survives
/// untouched and the run fails, so a skill cannot overwrite a deliberate edit
/// by running the same command it runs everywhere else.
#[test]
fn a_locally_changed_workflow_is_not_overwritten_without_force() {
    let (_d, root) = repo();
    let path = workflow_path(&root);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mine = render(V).replace("runs-on: ubuntu-latest", "runs-on: self-hosted");
    fs::write(&path, &mine).unwrap();

    assert_eq!(run_core(&root, V, false, false).unwrap(), ExitCode::from(1));
    assert_eq!(fs::read_to_string(&path).unwrap(), mine);

    assert_eq!(run_core(&root, V, false, true).unwrap(), ExitCode::SUCCESS);
    assert_eq!(fs::read_to_string(&path).unwrap(), render(V));
}

/// `--force` does not make `--check` write; the two are independent, and
/// `--check` is the one that never touches the filesystem.
#[test]
fn check_writes_nothing_even_with_force() {
    let (_d, root) = repo();
    let path = workflow_path(&root);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "name: mine\n").unwrap();

    assert_eq!(run_core(&root, V, true, true).unwrap(), ExitCode::SUCCESS);
    assert_eq!(fs::read_to_string(&path).unwrap(), "name: mine\n");
}

/// `--check` on an empty directory reports and creates nothing, so a dry run
/// leaves no trace whatsoever.
#[test]
fn check_on_an_absent_workflow_creates_nothing() {
    let (_d, root) = repo();
    assert_eq!(run_core(&root, V, true, false).unwrap(), ExitCode::SUCCESS);
    assert!(!root.join(".github").exists());
}

/// Outside a repository there is no `.github/` to write into, and guessing one
/// would scatter a workflow into whatever directory the user happened to be in.
#[test]
fn outside_a_git_repository_the_verb_refuses() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();

    assert_eq!(run_core(&root, V, false, false).unwrap(), ExitCode::from(2));
    assert!(!root.join(".github").exists());
}

/// A repository with no checklist still gets the workflow: setting CI up first
/// and writing the checklist afterwards is a reasonable order, and the workflow
/// is built to stay quiet until one appears.
#[test]
fn a_repository_without_a_checklist_still_gets_the_workflow() {
    let (_d, root) = repo();
    assert_eq!(run_core(&root, V, false, false).unwrap(), ExitCode::SUCCESS);
    assert_eq!(fs::read_to_string(workflow_path(&root)).unwrap(), render(V));
}

// ── Flag parsing ──────────────────────────────────────────────────────────────

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn help_and_bad_flags_never_reach_the_filesystem() {
    assert_eq!(run(&args(&["--help"])).unwrap(), ExitCode::SUCCESS);
    assert_eq!(run(&args(&["-h"])).unwrap(), ExitCode::SUCCESS);
    assert_eq!(run(&args(&["--nope"])).unwrap(), ExitCode::from(2));
    // A positional argument is a typo, not a path to write to: the destination
    // is fixed, so accepting one would silently ignore what the caller meant.
    assert_eq!(run(&args(&["somewhere.yml"])).unwrap(), ExitCode::from(2));
}
