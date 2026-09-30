#!/usr/bin/env bash
#
# Give the repository-wide "latest" mark back to the newest CLI release after a
# plugin release took it (R12; U7 of the workspace-split plan).
#
# Why this exists. One repository publishes two release lines: `ss-magic` on
# bare `vX.Y.Z` tags and `ss-magic-plugin` on `ss-magic-plugin-vX.Y.Z` tags.
# cargo-dist's generated workflow publishes each with a plain `gh release
# create`, which marks whatever it just created as the repository's latest
# release. Every `ss-magic` binary released before the per-line update check
# (0.11.0) polls `releases/latest` and parses only a bare `vX.Y.Z`, so a plugin
# release left holding the mark makes those installs report "already up to
# date" until the next CLI release. This script runs as a cargo-dist
# post-announce job (.github/workflows/mark-latest.yml) and can be run by hand.
#
# What it does, in order:
#   1. Reads the tag just announced from MARK_LATEST_TAG, else GITHUB_REF_NAME.
#      No tag at all is a usage error (exit 2).
#   2. A bare CLI tag (exactly `v` + MAJOR.MINOR.PATCH) exits 0 at once: the CLI
#      release holds the mark itself and must not be touched.
#   3. Otherwise lists the repository's releases (`--limit 200`: the default of
#      30 would, after thirty releases, silently truncate the very tag this
#      script exists to find), DROPS every draft and prerelease (a `v*` draft
#      would make `gh release edit --latest` fail outright, and R16 excludes the
#      same population), keeps only tags of the exact `v` + triple shape via an
#      anchored regex, and picks the numerically greatest (`v0.11.10` beats
#      `v0.11.3`; a lexical sort would get that wrong).
#   4. Runs `gh release edit <tag> --latest`, then reads `releases/latest` back
#      and FAILS (exit 1) if it does not name that tag – a token that cannot
#      edit releases must show up as a red job, not as a silently kept plugin
#      mark. With MARK_LATEST_DRY_RUN=1 it prints the chosen tag on stdout
#      instead and touches nothing.
#
# No CLI release at all (a repository whose first release is a plugin one) is
# a note and exit 0: there is nothing the mark could be given to.
#
# Environment:
#   MARK_LATEST_TAG      the announced tag (falls back to GITHUB_REF_NAME)
#   MARK_LATEST_REPO     OWNER/REPO (default: ViktorStiskala/superset-magic)
#   MARK_LATEST_DRY_RUN  when set to 1, print the chosen tag and do not edit
#   GH_TOKEN             what `gh` authenticates with in Actions
#
# Written for bash 3.2 (macOS): no associative arrays, no `mapfile`, no
# `${var^^}`. JSON is parsed with tr/sed over the three fields this script
# requests, so it needs neither jq nor python – `gh` itself is the only tool.

set -eu

tag="${MARK_LATEST_TAG:-${GITHUB_REF_NAME:-}}"
repo="${MARK_LATEST_REPO:-ViktorStiskala/superset-magic}"
dry_run="${MARK_LATEST_DRY_RUN:-0}"

note() { printf 'mark-latest: %s\n' "$*" >&2; }

# Exactly `v` + MAJOR.MINOR.PATCH: the shape the updater's anchored filter
# accepts and nothing else – no prefix before the `v`, no suffix after the
# patch, ASCII digits only.
is_bare_cli_tag() {
    printf '%s\n' "$1" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$'
}

if [ -z "$tag" ]; then
    note "no announced tag: set MARK_LATEST_TAG (or GITHUB_REF_NAME)"
    exit 2
fi

if is_bare_cli_tag "$tag"; then
    note "$tag is a CLI release and holds the latest mark itself; nothing to do"
    exit 0
fi

note "$tag is not a CLI release; finding the newest v* release to re-mark as latest"

# `gh` filters nothing here on purpose: the draft/prerelease exclusion and the
# tag-shape filter below are what the test drives, against a fixture listing.
listing=$(gh release list --repo "$repo" --json tagName,isDraft,isPrerelease --limit 200)

# One JSON object per line, whitespace removed (a tag can contain none, and gh
# pretty-prints when it feels like it). Then: drop drafts and prereleases, keep
# bare CLI tags, sort numerically on the three components, take the greatest.
# A named function rather than an inline pipeline: bash 3.2 cannot parse a
# `case` arm's closing paren inside `$( … )`.
newest_cli_tag_in() {
    tr -d ' \t\n\r' \
        | sed 's/},{/}\
{/g' \
        | while IFS= read -r obj || [ -n "$obj" ]; do
            # `|| [ -n "$obj" ]`: the last object has no trailing newline
            # after the `tr`, and a bare `read` returns non-zero on it and
            # would skip it – which silently dropped the NEWEST release when
            # it happened to be listed last.
            case "$obj" in
                *'"isDraft":true'* | *'"isPrerelease":true'*) continue ;;
            esac
            candidate=$(printf '%s' "$obj" | sed -n 's/.*"tagName":"\([^"]*\)".*/\1/p')
            [ -n "$candidate" ] || continue
            if is_bare_cli_tag "$candidate"; then
                # Sort on the bare triple: a key that starts mid-field
                # (`-k 1.2,1n`) is not numeric on every sort, and BSD sort
                # ranked `v1.0.0` above `v10.0.0` with it.
                printf '%s\n' "${candidate#v}"
            fi
        done \
        | sort -t . -k 1,1n -k 2,2n -k 3,3n \
        | tail -n 1 \
        | sed 's/^/v/'
}

newest=$(printf '%s' "$listing" | newest_cli_tag_in)

if [ -z "$newest" ]; then
    note "no published v* release exists yet; there is nothing to mark as latest"
    exit 0
fi

if [ "$dry_run" = "1" ]; then
    note "dry run: would mark $newest as the latest release"
    printf '%s\n' "$newest"
    exit 0
fi

note "marking $newest as the latest release"
gh release edit "$newest" --repo "$repo" --latest >/dev/null

# Read the mark back. A token that could not edit the release – or a release
# that GitHub refused to re-mark – must fail the job loudly here, because the
# alternative is every pre-0.11.0 binary silently reporting "up to date".
marked=$(gh api "repos/$repo/releases/latest" --jq .tag_name)
if [ "$marked" != "$newest" ]; then
    note "FAILED: releases/latest names $marked, not $newest; run by hand:"
    note "  gh release edit $newest --repo $repo --latest"
    exit 1
fi
note "releases/latest now names $newest"
exit 0
