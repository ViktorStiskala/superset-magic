#!/usr/bin/env bash
# =============================================================================
# Superset workspace setup
# =============================================================================
#
# Copies selected files and directories from $SUPERSET_ROOT_PATH into
# $SUPERSET_WORKSPACE_PATH, preserving their relative paths.
#
# Configuration
# -------------
# Edit .superset/setup_config.json next to this script:
#
#   {
#     "files": [
#       ".dev.vars",           # literal file at the repo root
#       "**/.dev.vars",        # .dev.vars at ANY depth (root included)
#       ".env.*",              # .env.local, .env.production, ...
#       "apps/*/config",       # every apps/<name>/config directory
#       "packages/**/fixtures" # fixtures dirs at any depth under packages/
#     ]
#   }
#
# All paths are relative to $SUPERSET_ROOT_PATH.
# Absolute paths and entries containing ".." are rejected.
#
# Glob syntax (bash, with globstar + nullglob + dotglob)
# ------------------------------------------------------
#   *         matches any characters except "/"
#   **        matches zero or more path segments (requires globstar)
#   **/foo    matches foo at any depth, INCLUDING the root
#   ?         matches exactly one character
#   [abc]     character class
#
# Notes:
#   - Dotfiles (like .dev.vars) ARE matched by * / **, thanks to dotglob.
#   - A glob that matches nothing is reported as "Skipped (no matches)" in the
#     default color and does NOT count as a problem.
#   - A literal path that doesn't exist is reported as "Skipped (missing)" in
#     bold red and DOES count toward the skipped tally.
#   - Matches that resolve to a directory are copied recursively.
#
# Default excludes
# ----------------
# Matches whose path contains any of the directory names listed in
# DEFAULT_EXCLUDES (defined below) are dropped, at any depth. This keeps
# patterns like "**/.dev.vars" from leaking files out of node_modules,
# virtualenvs, etc. Each excluded path is reported as
# "Skipped (excluded): <path>" in gray; excludes do NOT count toward the
# skipped tally that flips the final summary color. Edit the array to
# customize.
#
# Requirements
# ------------
#   - bash >= 4    (macOS default is 3.2; install via `brew install bash`)
#   - jq           (`brew install jq` / `apt-get install jq`)
#
# Environment
# -----------
#   SUPERSET_ROOT_PATH        source directory (required)
#   SUPERSET_WORKSPACE_PATH   destination directory (required)
#   NO_COLOR                  if set, disables ANSI color output
# =============================================================================

set -euo pipefail

# ---------------------------------------------------------------------------
# In-script config
# ---------------------------------------------------------------------------
# Directory names to exclude at any depth when collecting matches. A path is
# dropped if any of its segments equals one of these names.
DEFAULT_EXCLUDES=(
    node_modules
    .venv
)
# ---------------------------------------------------------------------------

if (( BASH_VERSINFO[0] < 4 )); then
    echo "Error: bash >= 4 required (found ${BASH_VERSION}). On macOS: 'brew install bash' and re-run with the Homebrew bash." >&2
    exit 1
fi

command -v jq >/dev/null 2>&1 || {
    echo "Error: jq is required (brew install jq / apt-get install jq)" >&2
    exit 1
}

: "${SUPERSET_ROOT_PATH:?SUPERSET_ROOT_PATH must be set}"
: "${SUPERSET_WORKSPACE_PATH:?SUPERSET_WORKSPACE_PATH must be set}"

SUPERSET_ROOT_PATH="${SUPERSET_ROOT_PATH%/}"
SUPERSET_WORKSPACE_PATH="${SUPERSET_WORKSPACE_PATH%/}"

if [[ ! -d "$SUPERSET_ROOT_PATH" ]]; then
    echo "Error: source does not exist: $SUPERSET_ROOT_PATH" >&2
    exit 1
fi

if [[ ! -d "$SUPERSET_WORKSPACE_PATH" ]]; then
    echo "Error: destination does not exist: $SUPERSET_WORKSPACE_PATH" >&2
    exit 1
fi

if [[ -t 1 && -z "${NO_COLOR:-}" ]]; then
    C_RESET=$'\033[0m'
    C_GRAY=$'\033[90m'
    C_RED_BOLD=$'\033[1;31m'
    C_GREEN_BOLD=$'\033[1;32m'
    C_ORANGE_BOLD=$'\033[1;38;5;208m'
else
    C_RESET=
    C_GRAY=
    C_RED_BOLD=
    C_GREEN_BOLD=
    C_ORANGE_BOLD=
fi

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
CONFIG_PATH="$SCRIPT_DIR/setup_config.json"

if [[ ! -f "$CONFIG_PATH" ]]; then
    echo "Error: config not found: $CONFIG_PATH" >&2
    exit 1
fi

PATTERNS=()
while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    PATTERNS+=( "$line" )
done < <(jq -r '(.files // []) | .[]' "$CONFIG_PATH")

shopt -s globstar nullglob dotglob

declare -A SEEN=()
MATCHES=()
copied=0
skipped=0

has_parent_segment() {
    # True if "$1" contains a ".." path segment (not just the literal substring).
    case "/$1/" in
        */../*) return 0 ;;
        *)      return 1 ;;
    esac
}

is_excluded() {
    # True if any path segment of "$1" matches an entry in DEFAULT_EXCLUDES.
    local path="$1" name
    for name in "${DEFAULT_EXCLUDES[@]}"; do
        case "/$path/" in
            */"$name"/*) return 0 ;;
        esac
    done
    return 1
}

pushd "$SUPERSET_ROOT_PATH" >/dev/null

for pat in "${PATTERNS[@]}"; do
    if [[ "$pat" == /* ]]; then
        printf '%sSkipped (absolute path rejected): %s%s\n' \
            "$C_RED_BOLD" "$pat" "$C_RESET" >&2
        skipped=$((skipped + 1))
        continue
    fi

    if has_parent_segment "$pat"; then
        printf '%sSkipped (".." not allowed): %s%s\n' \
            "$C_RED_BOLD" "$pat" "$C_RESET" >&2
        skipped=$((skipped + 1))
        continue
    fi

    if [[ "$pat" == *[\*\?\[]* ]]; then
        # Disable word splitting so patterns like "my folder/*" aren't split
        # on spaces before pathname expansion runs.
        _saved_IFS=$IFS
        IFS=
        expanded=( $pat )
        IFS=$_saved_IFS
        if (( ${#expanded[@]} == 0 )); then
            printf 'Skipped (no matches): %s\n' "$pat" >&2
            continue
        fi
        for rel in "${expanded[@]}"; do
            if is_excluded "$rel"; then
                # Intentional drop from DEFAULT_EXCLUDES; log but do not count.
                printf '%sSkipped (excluded): %s%s\n' "$C_GRAY" "$rel" "$C_RESET" >&2
                continue
            fi
            if [[ -n "${SEEN[$rel]:-}" ]]; then
                continue
            fi
            SEEN[$rel]=1
            MATCHES+=( "$rel" )
        done
    else
        if [[ ! -e "$pat" ]]; then
            printf '%sSkipped (missing): %s%s\n' "$C_RED_BOLD" "$pat" "$C_RESET" >&2
            skipped=$((skipped + 1))
            continue
        fi
        if is_excluded "$pat"; then
            # Literal entry inside an excluded directory: default excludes
            # take precedence over the explicit config entry. Logged but not
            # counted; edit DEFAULT_EXCLUDES to override.
            printf '%sSkipped (excluded): %s%s\n' "$C_GRAY" "$pat" "$C_RESET" >&2
            continue
        fi
        if [[ -z "${SEEN[$pat]:-}" ]]; then
            SEEN[$pat]=1
            MATCHES+=( "$pat" )
        fi
    fi
done

popd >/dev/null

for rel in "${MATCHES[@]}"; do
    src="$SUPERSET_ROOT_PATH/$rel"
    dst="$SUPERSET_WORKSPACE_PATH/$rel"

    if [[ -d "$src" ]]; then
        mkdir -p -- "$dst"
        cp -R -- "$src/." "$dst/"
    elif [[ -f "$src" ]]; then
        mkdir -p -- "$(dirname -- "$dst")"
        cp -- "$src" "$dst"
    else
        printf '%sSkipped (not a file or dir): %s%s\n' \
            "$C_RED_BOLD" "$rel" "$C_RESET" >&2
        skipped=$((skipped + 1))
        continue
    fi

    printf '%sCopied: %s%s\n' "$C_GRAY" "$rel" "$C_RESET"
    copied=$((copied + 1))
done

if (( skipped == 0 )); then
    summary_color="$C_GREEN_BOLD"
else
    summary_color="$C_ORANGE_BOLD"
fi

printf '%sFile setup done: copied: %d files, skipped %d files%s\n' \
    "$summary_color" "$copied" "$skipped" "$C_RESET"
