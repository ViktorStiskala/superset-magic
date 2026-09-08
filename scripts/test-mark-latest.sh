#!/usr/bin/env bash
#
# Tests for scripts/mark-latest.sh, driven against a fake `gh` on PATH.
#
# The script runs unattended after every release, and the one thing it must
# never do is hand the repository's "latest" mark to a plugin tag – or leave it
# there. So the fixture is AE1's release list, in GitHub's creation order, with
# every trap the plan names: a plugin release, a draft of the next CLI minor, a
# wrongly-prefixed CLI tag, a pre-release, and two CLI releases whose numeric
# and lexical orders disagree (`v0.11.10` vs `v0.11.3`). Every case asserts on
# what the fake `gh` was asked to do, recorded verbatim, so "the plugin tag was
# never marked" is an assertion rather than a hope.
#
# Nothing here touches the network or a real repository: the fake `gh` answers
# `release list` from a fixture file, records `release edit`, and answers
# `api .../releases/latest` from whatever the case says GitHub would say.
#
# Written for bash 3.2, like the script it tests.
#
# Usage: bash scripts/test-mark-latest.sh [-v]

set -u

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
SCRIPT="$REPO_ROOT/scripts/mark-latest.sh"
VERBOSE=${1:-}

# pass/fail, the counters, and every assert_* helper.
. "$REPO_ROOT/scripts/lib/test-harness.sh"

SANDBOX=$(mktemp -d "${TMPDIR:-/tmp}/ss-magic-mark-latest-tests.XXXXXX") || exit 1
trap 'rm -rf "$SANDBOX"' EXIT

# --------------------------------------------------------------------------
# The fake gh. It reads:
#   FAKE_GH_LISTING   the file whose contents answer `release list`
#   FAKE_GH_LATEST    what `api repos/.../releases/latest --jq .tag_name` prints
#   FAKE_GH_LOG       where every invocation's argv is appended, one per line
# --------------------------------------------------------------------------
mkdir -p "$SANDBOX/bin"
cat >"$SANDBOX/bin/gh" <<'FAKE'
#!/bin/sh
printf '%s\n' "$*" >>"$FAKE_GH_LOG"
case "$1 $2" in
    "release list") cat "$FAKE_GH_LISTING" ;;
    "release edit") : ;;
    "api repos"*)
        # After an edit, GitHub answers whatever the case configured.
        printf '%s\n' "$FAKE_GH_LATEST" ;;
    *) printf 'fake gh: unexpected invocation: %s\n' "$*" >&2; exit 99 ;;
esac
exit 0
FAKE
chmod +x "$SANDBOX/bin/gh"

# AE1's list in GitHub's creation order, compact as gh emits it when piped.
AE1_COMPACT="$SANDBOX/ae1-compact.json"
cat >"$AE1_COMPACT" <<'JSON'
[{"isDraft":false,"isPrerelease":false,"tagName":"ss-magic-plugin-v1.2.0"},{"isDraft":false,"isPrerelease":false,"tagName":"v0.11.3"},{"isDraft":true,"isPrerelease":false,"tagName":"v0.12.0"},{"isDraft":false,"isPrerelease":false,"tagName":"ss-magic-v0.13.0"},{"isDraft":false,"isPrerelease":false,"tagName":"v0.11.10"},{"isDraft":false,"isPrerelease":true,"tagName":"v0.9.0-rc1"}]
JSON

# The same list pretty-printed, as gh emits it on a terminal – the parser must
# not care.
AE1_PRETTY="$SANDBOX/ae1-pretty.json"
cat >"$AE1_PRETTY" <<'JSON'
[
  {
    "isDraft": false,
    "isPrerelease": false,
    "tagName": "ss-magic-plugin-v1.2.0"
  },
  {
    "isDraft": false,
    "isPrerelease": false,
    "tagName": "v0.11.3"
  },
  {
    "isDraft": true,
    "isPrerelease": false,
    "tagName": "v0.12.0"
  },
  {
    "isDraft": false,
    "isPrerelease": false,
    "tagName": "ss-magic-v0.13.0"
  },
  {
    "isDraft": false,
    "isPrerelease": false,
    "tagName": "v0.11.10"
  },
  {
    "isDraft": false,
    "isPrerelease": true,
    "tagName": "v0.9.0-rc1"
  }
]
JSON

# A list with no CLI release at all.
PLUGIN_ONLY="$SANDBOX/plugin-only.json"
cat >"$PLUGIN_ONLY" <<'JSON'
[{"isDraft":false,"isPrerelease":false,"tagName":"ss-magic-plugin-v1.0.0"},{"isDraft":true,"isPrerelease":false,"tagName":"v0.12.0"}]
JSON

# Numeric versus lexical order on every component.
NUMERIC="$SANDBOX/numeric.json"
cat >"$NUMERIC" <<'JSON'
[{"isDraft":false,"isPrerelease":false,"tagName":"v0.9.0"},{"isDraft":false,"isPrerelease":false,"tagName":"v0.10.0"},{"isDraft":false,"isPrerelease":false,"tagName":"v0.10.2"},{"isDraft":false,"isPrerelease":false,"tagName":"v1.0.0"},{"isDraft":false,"isPrerelease":false,"tagName":"v0.11.0"},{"isDraft":false,"isPrerelease":false,"tagName":"V2.0.0"},{"isDraft":false,"isPrerelease":false,"tagName":"v10.0.0"}]
JSON

# run <tag> <listing-file> <latest-answer> <dry-run:0|1>
# Leaves stdout in $out, stderr in $err, the gh log in $log, exit code in $rc.
run() {
    log="$SANDBOX/gh.log"; : >"$log"
    out="$SANDBOX/out"; err="$SANDBOX/err"
    env -i PATH="$SANDBOX/bin:/usr/bin:/bin" HOME="$SANDBOX" \
        MARK_LATEST_TAG="$1" MARK_LATEST_REPO="example/repo" MARK_LATEST_DRY_RUN="$4" \
        FAKE_GH_LISTING="$2" FAKE_GH_LATEST="$3" FAKE_GH_LOG="$log" \
        bash "$SCRIPT" >"$out" 2>"$err"
    rc=$?
}

# ==========================================================================
current_case="dry run over AE1 picks v0.11.10"
run "ss-magic-plugin-v1.2.0" "$AE1_COMPACT" "unused" 1
assert_eq 0 "$rc" "exits 0"
assert_eq "v0.11.10" "$(cat "$out")" "prints the chosen tag alone on stdout"
assert_contains_fixed "$log" "release list --repo example/repo --json tagName,isDraft,isPrerelease --limit 200" "lists with --limit 200 (the default 30 would truncate)"
assert_lacks_fixed "$log" "release edit" "a dry run edits nothing"
assert_lacks_fixed "$out" "ss-magic-plugin" "the plugin tag is never chosen"
assert_lacks_fixed "$out" "v0.12.0" "the draft is never chosen"
assert_lacks_fixed "$out" "v0.9.0-rc1" "the pre-release is never chosen"
assert_lacks_fixed "$out" "ss-magic-v0.13.0" "the wrongly-prefixed tag is never chosen"

# ==========================================================================
current_case="pretty-printed listing parses the same"
run "ss-magic-plugin-v1.2.0" "$AE1_PRETTY" "unused" 1
assert_eq 0 "$rc" "exits 0"
assert_eq "v0.11.10" "$(cat "$out")" "same choice from pretty JSON"

# ==========================================================================
current_case="numeric order on every component"
run "ss-magic-plugin-v1.2.0" "$NUMERIC" "unused" 1
assert_eq "v10.0.0" "$(cat "$out")" "v10.0.0 beats v1.0.0 and v0.11.0 (numeric, not lexical)"
assert_lacks_fixed "$out" "V2.0.0" "a case variant is not a CLI tag"

# ==========================================================================
current_case="a plugin release re-marks the newest v* release"
run "ss-magic-plugin-v1.2.0" "$AE1_COMPACT" "v0.11.10" 0
assert_eq 0 "$rc" "exits 0 once the mark is confirmed"
assert_contains_fixed "$log" "release edit v0.11.10 --repo example/repo --latest" "edits exactly the newest v* release with --latest"
assert_contains_fixed "$log" "api repos/example/repo/releases/latest --jq .tag_name" "reads the mark back"
assert_lacks_fixed "$log" "release edit ss-magic-plugin" "never edits a plugin release"
assert_empty_file "$out" "nothing on stdout outside a dry run"
assert_contains_fixed "$err" "releases/latest now names v0.11.10" "reports the confirmed mark"
edits=$(grep -c "release edit" "$log")
assert_eq 1 "$edits" "exactly one edit"

# ==========================================================================
current_case="a CLI release is a no-op that never calls gh"
run "v0.11.3" "$AE1_COMPACT" "v0.11.3" 0
assert_eq 0 "$rc" "exits 0"
assert_empty_file "$log" "gh is never invoked for a v* tag"
assert_contains_fixed "$err" "holds the latest mark itself" "says why"

# ==========================================================================
current_case="a prefixed CLI-looking tag is NOT a CLI release"
# `ss-magic-v0.13.0` fails the anchored filter, so it is treated like any other
# non-CLI tag: the newest bare v* release gets the mark.
run "ss-magic-v0.13.0" "$AE1_COMPACT" "v0.11.10" 0
assert_eq 0 "$rc" "exits 0"
assert_contains_fixed "$log" "release edit v0.11.10 --repo example/repo --latest" "re-marks v0.11.10"

# ==========================================================================
current_case="the mark not taking is a loud failure"
run "ss-magic-plugin-v1.2.0" "$AE1_COMPACT" "ss-magic-plugin-v1.2.0" 0
assert_eq 1 "$rc" "exits 1 when releases/latest still names the plugin tag"
assert_contains_fixed "$err" "FAILED" "says so"
assert_contains_fixed "$err" "gh release edit v0.11.10 --repo example/repo --latest" "prints the manual command"

# ==========================================================================
current_case="no CLI release at all"
run "ss-magic-plugin-v1.0.0" "$PLUGIN_ONLY" "unused" 0
assert_eq 0 "$rc" "exits 0"
assert_lacks_fixed "$log" "release edit" "nothing to edit"
assert_contains_fixed "$err" "no published v* release exists yet" "says so"

# ==========================================================================
current_case="no announced tag"
log="$SANDBOX/gh.log"; : >"$log"
env -i PATH="$SANDBOX/bin:/usr/bin:/bin" HOME="$SANDBOX" \
    FAKE_GH_LISTING="$AE1_COMPACT" FAKE_GH_LATEST="unused" FAKE_GH_LOG="$log" \
    bash "$SCRIPT" >"$SANDBOX/out" 2>"$SANDBOX/err"
assert_eq 2 "$?" "usage error"
assert_empty_file "$log" "gh is never invoked"

# ==========================================================================
current_case="GITHUB_REF_NAME is the fallback"
log="$SANDBOX/gh.log"; : >"$log"
env -i PATH="$SANDBOX/bin:/usr/bin:/bin" HOME="$SANDBOX" GITHUB_REF_NAME="v0.11.3" \
    FAKE_GH_LISTING="$AE1_COMPACT" FAKE_GH_LATEST="unused" FAKE_GH_LOG="$log" \
    bash "$SCRIPT" >"$SANDBOX/out" 2>"$SANDBOX/err"
assert_eq 0 "$?" "a v* ref name is the no-op"
assert_empty_file "$log" "gh is never invoked"

# ==========================================================================
current_case="the workflow wires the script and the manifest registers the job"
WF="$REPO_ROOT/.github/workflows/mark-latest.yml"
if [ -f "$WF" ]; then
    pass "mark-latest.yml exists"
    assert_contains_fixed "$WF" "scripts/mark-latest.sh" "the workflow runs the script"
    assert_contains_fixed "$WF" "workflow_call:" "callable by cargo-dist's post-announce step"
    assert_contains_fixed "$WF" "workflow_dispatch:" "runnable by hand as the documented fallback"
    assert_contains_fixed "$WF" "contents: write" "asks for the permission an edit needs"
else
    fail "mark-latest.yml is missing"
fi
assert_contains_fixed "$REPO_ROOT/dist-workspace.toml" 'post-announce-jobs = ["./mark-latest"]' "dist-workspace.toml registers the post-announce job"
assert_contains_fixed "$REPO_ROOT/.github/workflows/release.yml" "uses: ./.github/workflows/mark-latest.yml" "the generated release workflow calls it"

# ==========================================================================
printf '\n%s: %d passed, %d failed\n' "$(basename "$0")" "$passed" "$failed"
[ "$failed" -eq 0 ] || exit 1
exit 0
