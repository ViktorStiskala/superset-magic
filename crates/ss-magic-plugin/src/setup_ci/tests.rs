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
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use tempfile::TempDir;

use super::*;
use ss_magic_core::testutil::git_run;

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
        render_job.contains("releases/download/ss-magic-plugin-v$SS_MAGIC_PLUGIN_VERSION"),
        "the download must be pinned to the version in the env block, on the \
         PLUGIN's tag line — a bare `v<version>` tag names a `ss-magic` CLI \
         release, whose versions are deliberately never equal to the plugin's"
    );
    assert!(
        render_job.contains("ss-magic-plugin-$target/ss-magic-plugin"),
        "the archive nests the plugin binary under its own target directory"
    );
    // `-f` is what turns a 404 into a failure instead of a saved error page.
    assert!(render_job.contains("curl -fsSL"));
}

/// The workflow selects the checklists a pull request changed with a `git diff`
/// pathspec on the same naming convention the binary writes and validates
/// explicit paths against. Tying the two together here means a rename of the
/// convention cannot leave the workflow selecting a path nothing writes.
#[test]
fn the_workflow_globs_the_checklist_convention() {
    let glob = format!("{ACTIONS_REL}/*{CHECKLIST_SUFFIX}");
    assert!(
        TEMPLATE.contains(&glob),
        "the workflow should look for {glob}"
    );
}

// ── The asset: which checklists a pull request renders ────────────────────────

/// One step of the `render` job, from its `- name: <name>` line up to the next
/// step at the same indent (or the end of the job).
fn render_step(name: &str) -> String {
    let job = job_body("render");
    let header = format!("      - name: {name}\n");
    let start = job
        .find(&header)
        .unwrap_or_else(|| panic!("the render job has no `{name}` step"));
    let rest = &job[start + header.len()..];
    let end = rest.find("\n      - ").map_or(rest.len(), |i| i + 1);
    format!("{header}{}", &rest[..end])
}

/// The merge ref the checkout lands on is a merge commit whose first parent is
/// the base branch tip. Depth 2 is what brings that parent into the shallow
/// clone, so the selection can diff against it; depth 1, the default, would
/// leave `HEAD^1` unresolvable.
#[test]
fn the_render_job_checks_out_the_merge_commit_and_its_parents() {
    let checkout = job_body("render");
    let checkout = &checkout[checkout.find("actions/checkout").unwrap()..];
    let checkout = &checkout[..checkout.find("\n      - ").unwrap()];
    assert!(checkout.contains("fetch-depth: 2"), "{checkout}");
    assert!(
        checkout.contains("persist-credentials: false"),
        "{checkout}"
    );
}

/// The selection is what decides whether the verify gate runs at all, so a
/// selection that fails must fail the job: a failure read as "zero checklists"
/// would quietly turn the gate off for every pull request.
#[test]
fn the_selection_diffs_the_merge_commit_and_fails_closed() {
    let probe = code(&render_step("Select the changed checklists"));
    assert!(
        probe.contains("git rev-parse --verify HEAD^2"),
        "a non-merge HEAD must fail the job rather than diff against the wrong commit:\n{probe}"
    );
    assert!(
        probe.contains(
            "git diff --name-only -z --no-renames --diff-filter=AMT HEAD^1 HEAD \\\n            \
             -- ':(glob)docs/actions/*.checklist.json' >\"$RUNNER_TEMP/checklist-names\""
        ),
        "the diff runs as its own command into a file, so `set -e` sees its status:\n{probe}"
    );
    // `T` is a modification too: a checklist that was a symlink on the base
    // branch and is a regular file in the pull request (or the reverse) is a
    // changed document, and leaving it out would let it skip verification.
    // Only `D` (and `R`/`C`, which `--no-renames` never produces) stays out.
    // A process substitution's exit status is invisible to `set -e`, so a
    // failed `git diff` read that way would look exactly like an empty list.
    for block in run_blocks() {
        let block = code(&block);
        assert!(
            !block.contains("< <("),
            "no step may read through `< <(`:\n{block}"
        );
    }
}

/// Each step is a fresh shell, so the names have to be read back from the file
/// in every step that uses them; an array carried over from an earlier step
/// would expand to nothing and silently fall back to the no-argument route,
/// which refuses as soon as `docs/actions/` holds more than one document.
#[test]
fn verify_and_render_each_read_the_names_back_and_refuse_an_empty_list() {
    for (step, call) in [
        ("Verify the checklists", "checklist verify \"${names[@]}\""),
        (
            "Render the comment body",
            "checklist render-md --max-bytes 60000 \"${names[@]}\"",
        ),
    ] {
        let body = code(&render_step(step));
        assert!(
            body.contains("if: steps.probe.outputs.present == 'true'"),
            "{step} must run only when the selection found something:\n{body}"
        );
        assert!(
            body.contains("while IFS= read -r -d '' name; do")
                && body.contains("done <\"$RUNNER_TEMP/checklist-names\""),
            "{step} must re-read the NUL-separated names file:\n{body}"
        );
        assert!(
            body.contains("if [ ${#names[@]} -eq 0 ]; then") && body.contains("exit 1"),
            "{step} must fail on an empty list rather than call the verb with no paths:\n{body}"
        );
        assert!(body.contains(call), "{step} must pass every name:\n{body}");
    }
}

// ── Rendering and reading the pin back ────────────────────────────────────────

#[test]
fn rendering_substitutes_every_placeholder() {
    let out = render(V);
    assert!(
        !out.contains(VERSION_PLACEHOLDER),
        "an unsubstituted placeholder would download a release that cannot exist"
    );
    assert!(out.contains(&format!("SS_MAGIC_PLUGIN_VERSION: \"{V}\"")));
}

/// Every assertion elsewhere in this file is a `.contains(...)` substring
/// check, which cannot notice a template edit that corrupts the YAML *around*
/// the substrings it looks for — a stray tab, or an indent that drifts off
/// the file's own two-space grid. This is not a YAML parser (the crate
/// carries no YAML dependency, and a handful of tests do not justify adding
/// one); it is the cheapest possible tripwire, run against what actually gets
/// written to a user's `.github/workflows/`.
#[test]
fn the_rendered_workflow_has_sane_indentation() {
    let out = render(V);

    assert!(
        !out.contains('\t'),
        "the workflow must not contain tab characters"
    );

    for (n, line) in out.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - trimmed.len();
        assert!(
            indent % 2 == 0,
            "line {} has a {}-space indent, not a multiple of 2: {line:?}",
            n + 1,
            indent
        );
    }
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
        pinned_version("env:\n  SS_MAGIC_PLUGIN_VERSION: 0.4.2\n").as_deref(),
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
            found: "0.1.0".to_string(),
            generation: Generation::Current,
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

// ── Workflows an earlier release wrote ────────────────────────────────────────

/// The legacy generation recorded under `label`.
fn legacy(label: &str) -> &'static LegacyTemplate {
    LEGACY_TEMPLATES
        .iter()
        .find(|generation| generation.label == label)
        .unwrap_or_else(|| panic!("no legacy generation labelled {label}"))
}

/// Every workflow an earlier release could have written, untouched, is one
/// that can be advanced without `--force`. Looping over the table means a
/// generation added to it later is covered without a new test.
#[test]
fn every_legacy_generation_is_a_stale_pin() {
    assert!(!LEGACY_TEMPLATES.is_empty());
    for generation in LEGACY_TEMPLATES {
        let old = generation.render("0.1.0");
        assert_ne!(
            old, generation.body,
            "{} has no placeholder",
            generation.label
        );
        assert_eq!(
            classify(Some(&old), V),
            State::PinStale {
                found: "0.1.0".to_string(),
                generation: Generation::Legacy(generation.label),
            },
            "an untouched {} workflow must not read as a local edit",
            generation.label
        );
    }
}

/// The 0.11.0 generation pinned the sync CLI under its own key, so reading the
/// pin back has to know that key as well as today's.
#[test]
fn the_oldest_generation_pins_through_its_own_key() {
    let generation = legacy("0.11.0");
    assert_eq!(generation.pin_key, "SS_MAGIC_VERSION:");
    let old = generation.render("0.11.0");
    assert_eq!(pinned_version(&old).as_deref(), Some("0.11.0"));
    assert_eq!(
        classify(Some(&old), env!("CARGO_PKG_VERSION")).token(),
        "pin-stale"
    );
}

/// The version-equality rule belongs to the current template only: a legacy
/// workflow pinning exactly this build's version still selects checklists the
/// old way, so it is stale, not identical.
#[test]
fn a_legacy_generation_at_the_current_version_is_still_a_stale_pin() {
    for generation in LEGACY_TEMPLATES {
        assert_eq!(
            classify(Some(&generation.render(V)), V),
            State::PinStale {
                found: V.to_string(),
                generation: Generation::Legacy(generation.label),
            }
        );
    }
}

/// AE5: a legacy workflow somebody changed is still a local change, whatever
/// generation it started from.
#[test]
fn a_locally_edited_legacy_workflow_differs() {
    for generation in LEGACY_TEMPLATES {
        let edited = generation
            .render(V)
            .replace("runs-on: ubuntu-latest", "runs-on: self-hosted");
        assert_ne!(edited, generation.render(V));
        assert_eq!(
            classify(Some(&edited), V),
            State::Differs,
            "{}",
            generation.label
        );
    }
}

/// A fixture that drifted from what its release shipped would make every
/// workflow of that generation read as a local edit, so each fixture is held to
/// the shape its release wrote: the pin key and placeholder it names, and the
/// placeholder exactly once.
#[test]
fn every_legacy_fixture_carries_its_own_pin() {
    for generation in LEGACY_TEMPLATES {
        assert_eq!(generation.body.matches(generation.placeholder).count(), 1);
        assert!(
            generation.body.contains(&format!(
                "{} \"{}\"",
                generation.pin_key, generation.placeholder
            )),
            "{}",
            generation.label
        );
    }
}

/// The current generation keeps reporting a moved pin as nothing more than
/// that, and a legacy one as a whole new workflow, because that is what each
/// write does.
#[test]
fn the_check_line_says_what_the_write_would_change() {
    let current = State::PinStale {
        found: "0.1.0".to_string(),
        generation: Generation::Current,
    };
    assert!(would(&current, false).contains("advance the pin"));
    let old = State::PinStale {
        found: "0.1.0".to_string(),
        generation: Generation::Legacy(legacy("1.0.1").label),
    };
    assert!(
        would(&old, false).contains("replace it with the current workflow"),
        "{}",
        would(&old, false)
    );
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

/// [`run_core_with`] against `root`, returning the exit code and what it
/// printed to each stream.
fn run_captured(root: &Path, check: bool, force: bool) -> (ExitCode, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run_core_with(root, V, check, force, &mut out, &mut err).unwrap();
    (
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

/// AE4, the reporting half: a workflow the 1.0.1 generation wrote at its own
/// version reads as a stale pin, names that generation, shows the whole
/// difference (more than the pin moves), and stays exactly as it was.
#[test]
fn check_against_a_legacy_workflow_names_the_generation_and_shows_the_diff() {
    let (_d, root) = repo();
    add_checklist(&root);
    let path = workflow_path(&root);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let generation = legacy("1.0.1");
    let old = generation.render(generation.label);
    fs::write(&path, &old).unwrap();

    let (code, out, _) = run_captured(&root, true, false);
    assert_eq!(code, ExitCode::SUCCESS);
    assert_eq!(out.lines().next(), Some("state: pin-stale"), "{out}");
    assert!(
        out.contains(&format!("the {} generation", generation.label)),
        "the report must name the generation it found:\n{out}"
    );
    assert!(
        out.contains("--- on disk") && out.contains("+++ would write"),
        "a legacy upgrade changes more than the pin, so the diff is shown:\n{out}"
    );
    assert!(
        out.contains("replace it with the current workflow"),
        "{out}"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), old);
}

/// AE4, the writing half: no `--force` is needed to bring an untouched legacy
/// workflow up to the current one.
#[test]
fn a_legacy_workflow_is_replaced_without_force() {
    for generation in LEGACY_TEMPLATES {
        let (_d, root) = repo();
        add_checklist(&root);
        let path = workflow_path(&root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, generation.render(generation.label)).unwrap();

        let (code, _, err) = run_captured(&root, false, false);
        assert_eq!(code, ExitCode::SUCCESS, "{}: {err}", generation.label);
        assert_eq!(fs::read_to_string(&path).unwrap(), render(V));
    }
}

/// AE5, end to end: an edited legacy workflow is refused without `--force`
/// and left as it was.
#[test]
fn an_edited_legacy_workflow_is_not_overwritten_without_force() {
    let (_d, root) = repo();
    add_checklist(&root);
    let path = workflow_path(&root);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let generation = legacy("1.0.1");
    let mine = generation
        .render(generation.label)
        .replace("runs-on: ubuntu-latest", "runs-on: self-hosted");
    fs::write(&path, &mine).unwrap();

    let (code, out, _) = run_captured(&root, true, false);
    assert_eq!(code, ExitCode::SUCCESS);
    assert_eq!(out.lines().next(), Some("state: differs"), "{out}");

    let (code, _, err) = run_captured(&root, false, false);
    assert_eq!(code, ExitCode::from(1));
    assert!(err.contains("refused"), "{err}");
    assert_eq!(fs::read_to_string(&path).unwrap(), mine);
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
