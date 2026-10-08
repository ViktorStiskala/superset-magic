#!/usr/bin/env bash
#
# Documentation guards: the repository's current-state documents, checked for
# the mistakes that have already shipped once or would cost a release to undo.
#
# Seven checks, each printing exactly one `ok   <name>` or `FAIL <name>` line
# (the shape `python3 scripts/build-plugin-zip.py --check` prints), followed by
# `error: <name>: <detail>` lines on stderr for every problem found. The exit
# status is 1 when any check fails, 0 otherwise.
#
# A check also FAILS when anything it runs writes to stderr (an unreadable file,
# a `find` symlink loop, an awk error): every check decides "ok" from finding
# nothing, so a tool that died before it could look must not read as a clean
# result. Directory scans use `find -L`, so a symlinked file or subdirectory is
# checked (and counted) like any other: the reader of these files follows links.
#
# For the same reason a check FAILS when a path it requires is absent under
# ROOT, naming the path on stderr: a check whose scope is gone has looked at
# nothing, and a wrong ROOT, or a later move of the rules directory or the
# skills, would otherwise print seven `ok` lines and turn every guard off. The
# required paths are CLAUDE.md, README.md and .cursor/BUGBOT.md (regular files)
# and .claude/rules/ and plugin/skills/ (directories); each check requires the
# ones it reads, as its section below lists. The other scope paths
# (CONTRIBUTING.md, CONCEPTS.md, docs/solutions/, docs/runbooks/) are checked
# when present and skipped when absent.
#
#   retired subcommand spelling    no current-state document spells the old
#                                  `ss-magic` + `plugin` + `<verb>` form
#   skills name no CLAUDE_PLUGIN_DATA
#                                  no skill body names the data-dir variable
#   README pins the installer      README names no releases/latest/download/ URL
#   BUGBOT is self-contained       .cursor/BUGBOT.md has no Markdown link and
#                                  names no individual rule-file path
#   relative links resolve         every relative Markdown link in scope points
#                                  at a file or directory that exists
#   rule frontmatter               every .claude/rules/ file either has no
#                                  frontmatter or a well-formed `paths:` list
#   always-loaded budget           CLAUDE.md plus every rule file WITHOUT a
#                                  `paths:` list totals at most 50,000 bytes
#
# They live in a script rather than as inline workflow steps because a guard
# nobody can run locally is one nobody runs before pushing.
#
# docs/plans/ and docs/brainstorms/ are deliberately OUT of every guard's
# scope: a plan is a historical record and quotes the spellings, links and file
# names that were current when it was written.
#
# Written for bash 3.2, which is what macOS ships, and for both BSD and GNU
# grep/awk/find: no associative arrays, no `mapfile`, no `${var^^}`, no
# `grep -P`, no `case` inside `$( … )`, and every `read` loop handles an
# unterminated last line.
#
# Usage:
#   bash scripts/check-docs.sh [ROOT]          run the guards (ROOT defaults to
#                                              the repository holding this script)
#   bash scripts/check-docs.sh --selftest [-v] run the guards against fixture
#                                              trees in a temporary directory

# SC2329: the check_* functions are invoked indirectly, by name, from the
# runner's table (SC2317 is the same finding under shellcheck before 0.10).
# SC2016: the fixtures write literal backticks and `${...}`
# into Markdown on purpose.
# shellcheck disable=SC2329,SC2317,SC2016

set -u

SELF="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
DEFAULT_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

# The always-loaded budget, in bytes. The comparison is `total <= BUDGET`: a set
# of exactly 50,000 bytes passes and 50,001 fails. Every byte of the
# always-loaded set enters the context of every session in this repository
# before the first prompt, so it is bounded; the path-scoped rule files load
# only when a matching file is read or edited and do not count against it.
BUDGET_BYTES=50000

TAB=$(printf '\t')

# Prints one finding per required path that is absent under root, so the check
# calling it FAILS instead of reading clean over a scope it never saw. A path
# ending in `/` must be a directory; any other must be a regular file. Both
# tests follow symlinks, as the scans themselves do.
require_scope() { # root path...
    local root=$1 p
    shift
    for p in "$@"; do
        case $p in
            */)
                [ -d "$root/$p" ] ||
                    printf '%s: required directory is missing (the check cannot look at it)\n' "$p"
                ;;
            *)
                [ -f "$root/$p" ] ||
                    printf '%s: required file is missing (the check cannot look at it)\n' "$p"
                ;;
        esac
    done
    return 0
}

# --------------------------------------------------------------------------
# Check 1: the retired subcommand spelling.
#
# `ss-magic plugin <verb>` was the spelling while the plugin lived inside the
# sync CLI as a subcommand. It is now its own binary, `ss-magic-plugin <verb>`,
# and the old spelling is worse than merely stale: a user who has the sync CLI
# installed gets "unknown subcommand" from a real binary, and a skill body
# running it through the Bash tool reaches whichever `ss-magic` happens to be on
# PATH -- the update gate and the TUI included -- instead of the pinned plugin
# binary the wrapper resolves.
#
# The trailing space is load-bearing: it matches the verb form only, so prose
# about "the ss-magic plugin" as a thing still reads naturally.
#
# docs/solutions/, CLAUDE.md and the .claude/rules/ files it indexes are in
# scope too: all are read by agents working in this repo, so a retired command
# there is acted on, not merely misread. docs/plans/ is deliberately NOT in
# scope - a plan is a historical record and quotes the spelling that was current
# when it was written.
#
# Requires CLAUDE.md, README.md, .cursor/BUGBOT.md, .claude/rules/ and
# plugin/skills/; docs/solutions/, CONTRIBUTING.md and CONCEPTS.md are scanned
# when present.
# --------------------------------------------------------------------------
SPELLING_SCOPE="plugin/skills docs/solutions .claude/rules README.md CONTRIBUTING.md CONCEPTS.md CLAUDE.md .cursor/BUGBOT.md"

check_spelling() { # root
    local root=$1 p existing=""
    require_scope "$root" CLAUDE.md README.md .cursor/BUGBOT.md .claude/rules/ plugin/skills/
    for p in $SPELLING_SCOPE; do
        [ -e "$root/$p" ] && existing="$existing $p"
    done
    [ -n "$existing" ] || return 0
    # Word splitting of $existing is intended: the scope names contain no spaces.
    # shellcheck disable=SC2086
    (cd "$root" && find -L $existing -type f -exec grep -nHF -- 'ss-magic plugin ' {} +) |
        sed 's/$/ (spell it ss-magic-plugin <verb>)/'
    return 0
}

# --------------------------------------------------------------------------
# Check 2: no skill body names CLAUDE_PLUGIN_DATA.
#
# A skill body runs through the Bash tool, where ${CLAUDE_PLUGIN_DATA} is NOT
# exported - it reaches hook and MCP/LSP processes only. Naming it in a skill
# would expand to nothing and produce a path at the filesystem root, with no
# error. Skills reach the pinned binary through the bin/ wrapper instead, which
# is on the Bash tool's PATH while the plugin is enabled.
#
# Requires plugin/skills/.
# --------------------------------------------------------------------------
check_plugin_data() { # root
    local root=$1
    require_scope "$root" plugin/skills/
    [ -d "$root/plugin/skills" ] || return 0
    (cd "$root" && find -L plugin/skills -type f -exec grep -nHF -- 'CLAUDE_PLUGIN_DATA' {} +) |
        sed 's/$/ (expands to nothing in the Bash tool; use ss-magic-plugin)/'
    return 0
}

# --------------------------------------------------------------------------
# Check 3: README pins the installer to a release, never `latest`.
#
# README documents `releases/download/v<V>/ss-magic-installer.sh`, pinned to a
# published release, not `releases/latest/download/...`. Two reasons, and the
# second is the one that bites: the command a reader copies must be
# reproducible, and `latest` is repository-wide -- it resolves to whichever
# release was published most recently, which since the split can be a
# `ss-magic-plugin-vX.Y.Z` release that carries no installer script at all. The
# documented one-liner would then 404 for everyone.
#
# Requires README.md.
# --------------------------------------------------------------------------
check_installer_pin() { # root
    local root=$1
    require_scope "$root" README.md
    [ -f "$root/README.md" ] || return 0
    (cd "$root" && grep -nHF -- 'releases/latest/download/' README.md) |
        sed 's/$/ (latest can resolve to a plugin release with no installer; pin a vX.Y.Z release)/'
    return 0
}

# --------------------------------------------------------------------------
# Check 4: BUGBOT is self-contained.
#
# Cursor Bugbot reads .cursor/BUGBOT.md on its own: it cannot follow a link,
# load CLAUDE.md, or read a rule file. So the file restates every convention
# inline, contains no Markdown link at all (no inline `](` anywhere and no
# `[label]: target` reference definition, which is what turns a `[text][label]`
# into a link), and names no individual rule file (`.claude/rules/<name>.md`): a reference it cannot follow
# reads as a rule while carrying none of the rule's content. Naming the
# `.claude/rules/` directory itself is allowed, as the subject of a review rule
# ("flag a change under `.claude/rules/` that ..."); so is a glob such as
# `.claude/rules/*.md`, which names no individual file.
#
# Requires .cursor/BUGBOT.md.
# --------------------------------------------------------------------------
check_bugbot() { # root
    local root=$1 f=".cursor/BUGBOT.md"
    require_scope "$root" "$f"
    [ -f "$root/$f" ] || return 0
    (cd "$root" && grep -nHF -- '](' "$f") |
        sed 's/$/ (Markdown link; BUGBOT cannot follow links, restate the rule inline)/'
    (cd "$root" && grep -nHE -- '^ *\[[^]]+\]:' "$f") |
        sed 's/$/ (Markdown reference definition; BUGBOT cannot follow links, restate the rule inline)/'
    (cd "$root" && grep -nHE -- '\.claude/rules/[A-Za-z0-9._/-]+\.md' "$f") |
        sed 's/$/ (names an individual rule file; restate the rule inline)/'
    return 0
}

# --------------------------------------------------------------------------
# Check 5: relative links resolve.
#
# Every relative Markdown link in the current-state documents must point at a
# file or directory that exists, resolved against the linking file's own
# directory (a leading `/` resolves against the repository root, as GitHub
# renders it). That covers inline links (`[text](target)`, `![alt](target)`) and
# reference definitions (`[label]: target`; a `[^1]:` footnote is not one). A
# `#anchor` suffix and a `?query` are stripped first; a pure `#anchor`, and any
# target with a URL scheme (`https:`, `mailto:`, ...), are not checked.
#
# Links inside code are skipped, because those show syntax rather than link
# anything, and "inside code" follows the CommonMark rules closely enough that
# a quirk of the document cannot quietly switch the check off for the rest of a
# file:
#   - a fence is a run of at least three backticks or tildes (indentation
#     allowed, for fences inside list items); a backtick fence's info string
#     holds no backtick, so a line opening with an inline span such as
#     ```` ```foo``` ```` is prose, not a fence;
#   - a fence closes only on a run of the SAME character at least as LONG as
#     the opener with nothing else on the line, so a ```` ```rust ```` inside a
#     four-backtick fence is content;
#   - a fence still open at the end of the file is itself reported, since
#     every link after its opener would otherwise go unchecked without a word;
#   - a code span is a backtick run closed by the next run of the SAME length
#     (so ``` `` a ` b `` ``` is one span), and may continue onto later lines of
#     the same paragraph; an unmatched run is literal text.
#
# The scope is the documents an agent or a reader acts on: CLAUDE.md and every
# .claude/rules/ file, README.md, CONTRIBUTING.md, CONCEPTS.md, docs/runbooks/,
# docs/solutions/ and plugin/skills/. docs/plans/ and docs/brainstorms/ are out
# of scope (historical records; their links point at the tree as it was).
# Requires CLAUDE.md, README.md, .claude/rules/ and plugin/skills/; the rest of
# the scope is checked when present.
# --------------------------------------------------------------------------
link_scope_files() { # root -> repo-relative paths, one per line
    local root=$1 p d
    for p in CLAUDE.md README.md CONTRIBUTING.md CONCEPTS.md; do
        [ -f "$root/$p" ] && printf '%s\n' "$p"
    done
    rule_files "$root"
    for d in docs/runbooks docs/solutions plugin/skills; do
        [ -d "$root/$d" ] || continue
        (cd "$root" && find -L "$d" -type f -name '*.md' | LC_ALL=C sort)
    done
    return 0
}

# Prints `L<TAB><line><TAB><raw target>` for every link outside code, and
# `U<TAB><line><TAB>` for a fence opened on <line> that never closes.
#
# Lines are gathered into blocks (a paragraph or list item; a heading or a table
# row is a block of its own) so a code span can be matched across the lines of
# one block but never across two.
extract_links() { # file
    awk '
        function runlen(s, i,   n) {
            n = 0
            while (substr(s, i + n, 1) == "`") n++
            return n
        }
        # The start of the next backtick run of exactly n, at or after j; 0 if none.
        function span_end(s, j, n,   m, L) {
            L = length(s)
            while (j <= L) {
                if (substr(s, j, 1) == "`") {
                    m = runlen(s, j)
                    if (m == n) return j
                    j += m
                } else j++
            }
            return 0
        }
        # Removes every code span; a newline inside a span is kept, so the
        # result still splits into the block'"'"'s original lines.
        function strip_spans(s,   out, i, L, c, n, e, k) {
            out = ""; i = 1; L = length(s)
            while (i <= L) {
                c = substr(s, i, 1)
                if (c == "\\" && substr(s, i + 1, 1) == "`") { out = out "\\`"; i += 2; continue }
                if (c == "`") {
                    n = runlen(s, i)
                    e = span_end(s, i + n, n)
                    if (e == 0) { out = out substr(s, i, n); i += n; continue }
                    for (k = i; k < e + n; k++) if (substr(s, k, 1) == "\n") out = out "\n"
                    i = e + n
                    continue
                }
                out = out c; i++
            }
            return out
        }
        function flush(   s, k, n, parts, line, t) {
            if (nb == 0) return
            s = buf[1]
            for (k = 2; k <= nb; k++) s = s "\n" buf[k]
            n = split(strip_spans(s), parts, "\n")
            for (k = 1; k <= n; k++) {
                line = parts[k]
                if (line ~ /^ *\[[^]^][^]]*\]:/) {
                    t = line
                    sub(/^ *\[[^]]*\]:[ \t]*/, "", t)
                    print "L\t" bl[k] "\t" t
                    continue
                }
                while (match(line, /\]\([^)]*\)/)) {
                    print "L\t" bl[k] "\t" substr(line, RSTART + 2, RLENGTH - 3)
                    line = substr(line, RSTART + RLENGTH)
                }
            }
            nb = 0
        }
        function is_closer(s,   t, n) {
            t = s
            sub(/^[ \t]*/, "", t)
            n = 0
            while (substr(t, n + 1, 1) == fchar) n++
            if (n < flen) return 0
            return substr(t, n + 1) ~ /^[ \t]*$/
        }
        { sub(/\r$/, "") }
        infence {
            if (is_closer($0)) infence = 0
            next
        }
        {
            t = $0
            sub(/^[ \t]*/, "", t)
            c = substr(t, 1, 1)
            if (c == "`" || c == "~") {
                n = 0
                while (substr(t, n + 1, 1) == c) n++
                if (n >= 3 && !(c == "`" && index(substr(t, n + 1), "`") > 0)) {
                    flush()
                    infence = 1; fchar = c; flen = n; fline = NR
                    next
                }
            }
        }
        /^[ \t]*$/ { flush(); next }
        /^ *#+([ \t]|$)/ || /^[ \t]*\|/ {
            flush(); nb = 1; buf[1] = $0; bl[1] = NR; flush()
            next
        }
        /^[ \t]*([-*+]|[0-9]+[.)])([ \t]|$)/ { flush() }
        { nb++; buf[nb] = $0; bl[nb] = NR }
        END {
            flush()
            if (infence) print "U\t" fline "\t"
        }
    ' "$1"
}

check_links() { # root
    local root=$1 rel dir kind ln raw target path scheme_re='^[A-Za-z][A-Za-z0-9+.-]*:'
    require_scope "$root" CLAUDE.md README.md .claude/rules/ plugin/skills/
    link_scope_files "$root" | while IFS= read -r rel || [ -n "$rel" ]; do
        dir=$(dirname "$rel")
        extract_links "$root/$rel" | while IFS="$TAB" read -r kind ln raw || [ -n "$kind" ]; do
            if [ "$kind" = U ]; then
                printf '%s:%s: code fence never closes (every link after it would go unchecked)\n' "$rel" "$ln"
                continue
            fi
            # Trim surrounding whitespace, then drop an optional title.
            target=$(printf '%s' "$raw" | sed 's/^[[:space:]]*//; s/[[:space:]]*$//')
            if [ "${target#<}" != "$target" ]; then
                target=${target#<}
                target=${target%%>*}
            else
                target=${target%% *}
            fi
            [ -n "$target" ] || continue
            [ "${target#\#}" = "$target" ] || continue
            [[ $target =~ $scheme_re ]] && continue
            path=${target%%#*}
            path=${path%%\?*}
            path=${path//%20/ }
            [ -n "$path" ] || continue
            if [ "${path#/}" != "$path" ]; then
                path="$root$path"
            else
                path="$root/$dir/$path"
            fi
            [ -e "$path" ] || printf '%s:%s: %s (no such file)\n' "$rel" "$ln" "$target"
        done
    done
    return 0
}

# --------------------------------------------------------------------------
# Check 6: rule frontmatter is absent or a well-formed `paths:` list.
#
# Claude Code loads a .claude/rules/ file with no frontmatter in every session,
# and one whose frontmatter carries a `paths:` list only when a file matching
# one of those globs is read or edited. Frontmatter that opens with `---` but is
# anything else (no closing `---`, no `paths:` key, an empty list, an inline
# `paths: [...]` form, a stray key) is a file whose loading is a guess, so it is
# refused. The accepted shape, with blank and `#` comment lines allowed:
#
#   ---
#   paths:
#     - "crates/ss-magic/**"
#   ---
#
# Each list item must be something a YAML parser reads as a plain string:
# double- or single-quoted, or an unquoted path starting with a letter, digit,
# `_`, `.` or `/` and holding no `: `. An unquoted `**/*.rs` is NOT one - a
# leading `*` is a YAML alias, and `&`, `!`, `{`, `[`, `|`, `>`, `%`, `@` and
# a backtick are syntax too - so the parser rejects the whole frontmatter while
# a line-shape test would accept it. Indentation is spaces only: YAML forbids
# tabs there. Accepting either would also drop the file from the always-loaded
# budget below, although the loader cannot read its `paths:` at all.
#
# Requires .claude/rules/.
# --------------------------------------------------------------------------

# Prints `none`, `paths`, or `bad <reason>` for one file.
frontmatter_state() { # file
    awk -v q="'" '
        BEGIN {
            dq = "^\"([^\"\\\\]|\\\\.)*\"([ \t]+#.*)?$"
            sq = "^" q "([^" q "]|" q q ")*" q "([ \t]+#.*)?$"
        }
        { sub(/\r$/, "") }
        NR == 1 {
            if ($0 != "---") { none = 1; exit }
            next
        }
        $0 == "---" { closed = 1; exit }
        /^[ \t]*$/ || /^[ \t]*#/ { next }
        /^paths:[ \t]*$/ {
            if (seen && bad == "") bad = "line " NR ": a second paths: key"
            seen = 1
            next
        }
        seen && /^[ \t]*-/ {
            lead = $0
            sub(/-.*/, "", lead)
            v = $0
            sub(/^[ \t]*-/, "", v)
            sub(/[ \t]+$/, "", v)
            if (lead ~ /\t/) {
                if (bad == "") bad = "line " NR ": tab in the indentation of [" $0 "] (YAML allows spaces only)"
                next
            }
            if (v !~ /^ +[^ ]/) {
                if (bad == "") bad = "line " NR ": list item [" $0 "] has no value after a space"
                next
            }
            sub(/^ +/, "", v)
            if (v ~ dq || v ~ sq) { items++; next }
            sub(/[ \t]+#.*$/, "", v)
            if (v ~ /^[A-Za-z0-9_.\/]/ && v !~ /: / && v !~ /:$/) { items++; next }
            if (bad == "") bad = "line " NR ": list item [" $0 "] is not a quoted string or a plain path (quote it)"
            next
        }
        { if (bad == "") bad = "line " NR ": unexpected [" $0 "]" }
        END {
            if (NR == 0 || none) { print "none"; exit }
            if (!closed) { print "bad frontmatter opened with --- is never closed"; exit }
            if (!seen) { print "bad frontmatter has no paths: list"; exit }
            if (bad != "") { print "bad " bad; exit }
            if (items == 0) { print "bad paths: has no list items"; exit }
            print "paths"
        }
    ' "$1"
}

rule_files() { # root -> repo-relative paths of every .claude/rules/**/*.md
    local root=$1
    [ -d "$root/.claude/rules" ] || return 0
    (cd "$root" && find -L .claude/rules -type f -name '*.md' | LC_ALL=C sort)
    return 0
}

check_frontmatter() { # root
    local root=$1 rel state
    require_scope "$root" .claude/rules/
    rule_files "$root" | while IFS= read -r rel || [ -n "$rel" ]; do
        state=$(frontmatter_state "$root/$rel")
        [ "${state#bad }" = "$state" ] || printf '%s: %s\n' "$rel" "${state#bad }"
    done
    return 0
}

# --------------------------------------------------------------------------
# Check 7: the always-loaded budget.
#
# The always-loaded set is CLAUDE.md plus every .claude/rules/**/*.md that does
# NOT carry a well-formed `paths:` list (a malformed one is counted: whatever
# the loader makes of it, assuming it loads every session is the safe reading).
# The set is DERIVED by scanning the directory, never named, so a new rule file
# added without frontmatter, in a subdirectory or not, counts the moment it
# exists.
#
# Requires CLAUDE.md and .claude/rules/: without either, the total would count
# only what is left and pass however large the missing part has grown.
# --------------------------------------------------------------------------
always_loaded_files() { # root
    local root=$1 rel
    [ -f "$root/CLAUDE.md" ] && printf '%s\n' CLAUDE.md
    rule_files "$root" | while IFS= read -r rel || [ -n "$rel" ]; do
        [ "$(frontmatter_state "$root/$rel")" = "paths" ] || printf '%s\n' "$rel"
    done
    return 0
}

always_loaded_bytes() { # root
    local root=$1 rel total=0 size
    while IFS= read -r rel || [ -n "$rel" ]; do
        [ -n "$rel" ] || continue
        # BSD wc pads its count with spaces; arithmetic expansion ignores them.
        size=$(wc -c <"$root/$rel")
        total=$((total + size))
    done <<EOF
$(always_loaded_files "$root")
EOF
    printf '%s\n' "$total"
}

check_budget() { # root
    local root=$1 total rel size
    require_scope "$root" CLAUDE.md .claude/rules/
    total=$(always_loaded_bytes "$root")
    [ "$total" -le "$BUDGET_BYTES" ] && return 0
    printf '%s bytes > %s budget; the always-loaded set is:\n' "$total" "$BUDGET_BYTES"
    always_loaded_files "$root" | while IFS= read -r rel || [ -n "$rel" ]; do
        size=$(wc -c <"$root/$rel")
        printf '  %s (%s bytes)\n' "$rel" "$((size + 0))"
    done
    return 0
}

# --------------------------------------------------------------------------
# The runner.
# --------------------------------------------------------------------------
run_checks() { # root
    local root=$1 problems="" found name fn status=0
    # name|function pairs, one per line, in print order.
    while IFS='|' read -r name fn || [ -n "$name" ]; do
        [ -n "$name" ] || continue
        # stderr is folded into the findings: a check whose tool failed (an
        # unreadable file, a symlink loop, an awk error) has looked at nothing,
        # and an empty result from it must not print `ok`.
        found=$("$fn" "$root" 2>&1)
        if [ -n "$found" ]; then
            printf 'FAIL %s\n' "$name"
            problems="$problems$(printf '%s\n' "$found" | sed "s|^|error: $name: |")
"
            status=1
        else
            printf 'ok   %s\n' "$name"
        fi
    done <<'EOF'
retired subcommand spelling|check_spelling
skills name no CLAUDE_PLUGIN_DATA|check_plugin_data
README pins the installer|check_installer_pin
BUGBOT is self-contained|check_bugbot
relative links resolve|check_links
rule frontmatter|check_frontmatter
always-loaded budget|check_budget
EOF
    if [ -n "$problems" ]; then
        printf '\n%s' "$problems" >&2
    fi
    return $status
}

# --------------------------------------------------------------------------
# --selftest: every check driven against fixture trees, each case asserting on
# the one ok/FAIL line it is about (and on the detail line where two failure
# reasons share a check).
# --------------------------------------------------------------------------
selftest() {
    VERBOSE=${1:-}
    # pass/fail, the counters, and every assert_* helper.
    # shellcheck source=lib/test-harness.sh
    . "$DEFAULT_ROOT/scripts/lib/test-harness.sh"

    SANDBOX=$(mktemp -d "${TMPDIR:-/tmp}/ss-magic-check-docs-tests.XXXXXX") || exit 1
    trap 'rm -rf "$SANDBOX"' EXIT

    # A clean tree every check passes on: one always-loaded rule, one scoped
    # rule, a link of each kind the check must accept, and a BUGBOT that names
    # the rules directory as a subject.
    BASE="$SANDBOX/base"
    mkdir -p "$BASE/.claude/rules" "$BASE/.cursor" "$BASE/plugin/skills/demo" \
        "$BASE/docs/solutions/logic-errors" "$BASE/docs/runbooks" "$BASE/docs/plans"
    cat >"$BASE/CLAUDE.md" <<'MD'
# Index

See [hard rules](./.claude/rules/hard.md) and [README](./README.md#install).
Online: [site](https://example.com/x.md), [mail](mailto:a@b.c), [top](#index).

```plaintext
[not a link](./missing-in-fence.md)
```

Inline code: `[x](./missing-in-code.md)` is syntax.
MD
    printf '## Hard rules\n\nNever do the thing.\n' >"$BASE/.claude/rules/hard.md"
    printf -- '---\npaths:\n  - "crates/x/**"\n  - "assets/**"\n---\n\n## Map\n' >"$BASE/.claude/rules/scoped.md"
    printf '# Readme\n\n## Install\n\nInstall from releases/download/v1.0.0/installer.sh\n' >"$BASE/README.md"
    printf '# Contributing\n\nSee [solutions](./docs/solutions/logic-errors/one.md).\n' >"$BASE/CONTRIBUTING.md"
    printf '# Concepts\n' >"$BASE/CONCEPTS.md"
    printf '# One\n\nBack to [contributing](../../../CONTRIBUTING.md).\n' >"$BASE/docs/solutions/logic-errors/one.md"
    printf '# Runbook\n\nSee [README](../../README.md).\n' >"$BASE/docs/runbooks/r.md"
    printf '# Skill\n\nRun `ss-magic-plugin status`.\n' >"$BASE/plugin/skills/demo/SKILL.md"
    printf '# Plan\n\nSee [gone](./gone.md).\n' >"$BASE/docs/plans/old-plan.md"
    cat >"$BASE/.cursor/BUGBOT.md" <<'MD'
# Bugbot

- Flag a change under `.claude/rules/` that adds frontmatter other than a
  `paths:` list.
- The plugin is spelled ss-magic-plugin and reached through the wrapper.
MD

    fixture() { # name -> fresh copy of BASE at $SANDBOX/<name>
        rm -rf "${SANDBOX:?}/$1"
        cp -R "$BASE" "$SANDBOX/$1"
        F="$SANDBOX/$1"
    }
    run_guard() { # root -> OUT, ERR, CODE
        OUT="$SANDBOX/out"; ERR="$SANDBOX/err"
        "$BASH" "$SELF" "$1" >"$OUT" 2>"$ERR"
        CODE=$?
    }
    pad_to() { # file total-bytes-of-the-always-loaded-set
        local now
        now=$(always_loaded_bytes "$F")
        head -c $(($2 - now)) /dev/zero | tr '\0' 'x' >>"$1"
    }

    # ======================================================================
    current_case="the clean fixture"
    run_guard "$BASE"
    assert_eq 0 "$CODE" "exits 0"
    assert_eq 7 "$(grep -c '^ok   ' "$OUT")" "seven ok lines"
    assert_eq 7 "$(wc -l <"$OUT" | tr -d ' ')" "one line per check and nothing else"
    assert_empty_file "$ERR" "nothing on stderr"
    assert_contains_fixed "$OUT" "ok   always-loaded budget" "the budget line is printed"

    # ======================================================================
    current_case="retired spelling under .claude/rules/"
    fixture spelling-rules
    printf 'Run ss-magic plugin status to see why.\n' >>"$F/.claude/rules/hard.md"
    run_guard "$F"
    assert_eq 1 "$CODE" "exits 1"
    assert_contains_fixed "$OUT" "FAIL retired subcommand spelling" "the spelling guard fails"
    assert_contains_fixed "$ERR" ".claude/rules/hard.md:" "the detail names the file"

    current_case="retired spelling under docs/plans/"
    fixture spelling-plans
    printf 'Run ss-magic plugin status to see why.\n' >>"$F/docs/plans/old-plan.md"
    run_guard "$F"
    assert_eq 0 "$CODE" "exits 0"
    assert_contains_fixed "$OUT" "ok   retired subcommand spelling" "plans are out of scope"

    current_case="retired spelling in a nested rule file"
    fixture spelling-nested
    mkdir -p "$F/.claude/rules/sub"
    printf -- '---\npaths:\n  - "x/**"\n---\nss-magic plugin enable\n' >"$F/.claude/rules/sub/deep.md"
    run_guard "$F"
    assert_contains_fixed "$OUT" "FAIL retired subcommand spelling" "a scoped, nested rule is scanned too"

    # ======================================================================
    current_case="CLAUDE_PLUGIN_DATA in a skill body"
    fixture plugin-data
    printf 'Run ${CLAUDE_PLUGIN_DATA}/bin/ss-magic-plugin status\n' >>"$F/plugin/skills/demo/SKILL.md"
    run_guard "$F"
    assert_eq 1 "$CODE" "exits 1"
    assert_contains_fixed "$OUT" "FAIL skills name no CLAUDE_PLUGIN_DATA" "the guard fails"

    current_case="releases/latest/download/ in README"
    fixture installer
    printf 'curl https://github.com/o/r/releases/latest/download/installer.sh\n' >>"$F/README.md"
    run_guard "$F"
    assert_eq 1 "$CODE" "exits 1"
    assert_contains_fixed "$OUT" "FAIL README pins the installer" "the guard fails"

    # ======================================================================
    current_case="BUGBOT carries a Markdown link"
    fixture bugbot-link
    printf 'See [the rule](./x.md).\n' >>"$F/.cursor/BUGBOT.md"
    run_guard "$F"
    assert_eq 1 "$CODE" "exits 1"
    assert_contains_fixed "$OUT" "FAIL BUGBOT is self-contained" "the guard fails"
    assert_contains_fixed "$ERR" "(Markdown link;" "the link-free half fired"

    current_case="BUGBOT names an individual rule file"
    fixture bugbot-rule-path
    printf 'The rule lives in .claude/rules/foo.md now.\n' >>"$F/.cursor/BUGBOT.md"
    run_guard "$F"
    assert_eq 1 "$CODE" "exits 1"
    assert_contains_fixed "$OUT" "FAIL BUGBOT is self-contained" "the guard fails"
    assert_contains_fixed "$ERR" "(names an individual rule file;" "the rule-path half fired"
    assert_lacks_fixed "$ERR" "(Markdown link;" "the link-free half stayed quiet"

    current_case="BUGBOT names the rules directory and a glob"
    fixture bugbot-dir
    printf 'Flag a new file under `.claude/rules/` or `.claude/rules/*.md` without review.\n' >>"$F/.cursor/BUGBOT.md"
    run_guard "$F"
    assert_eq 0 "$CODE" "exits 0"
    assert_contains_fixed "$OUT" "ok   BUGBOT is self-contained" "the directory is an allowed subject"

    # ======================================================================
    current_case="a relative link to a missing file"
    fixture link-missing
    printf 'See [gone](./docs/nowhere.md).\n' >>"$F/CONTRIBUTING.md"
    run_guard "$F"
    assert_eq 1 "$CODE" "exits 1"
    assert_contains_fixed "$OUT" "FAIL relative links resolve" "the link guard fails"
    assert_contains_fixed "$ERR" "CONTRIBUTING.md:4: ./docs/nowhere.md" "the detail names file, line and target"

    current_case="a broken link in a nested rule file and in a skill"
    fixture link-nested
    mkdir -p "$F/.claude/rules/sub"
    printf 'See [x](../../../nope.md).\n' >"$F/.claude/rules/sub/deep.md"
    printf 'See [ref](./reference.md).\n' >>"$F/plugin/skills/demo/SKILL.md"
    run_guard "$F"
    assert_contains_fixed "$ERR" ".claude/rules/sub/deep.md:1:" "nested rule file is scanned"
    assert_contains_fixed "$ERR" "plugin/skills/demo/SKILL.md:" "skills are scanned"

    current_case="a link with an anchor to an existing file"
    fixture link-anchor
    printf 'See [the install](./README.md#install) and [one](docs/solutions/logic-errors/one.md#problem "title").\n' >>"$F/CONTRIBUTING.md"
    run_guard "$F"
    assert_eq 0 "$CODE" "exits 0"
    assert_contains_fixed "$OUT" "ok   relative links resolve" "the anchor is stripped before resolving"

    current_case="a broken link in a plan"
    # BASE's plan already links ./gone.md; this case documents that it is
    # deliberately out of scope rather than an accident of the fixture.
    run_guard "$BASE"
    assert_contains_fixed "$OUT" "ok   relative links resolve" "plans are out of scope"

    # ======================================================================
    current_case="frontmatter without a paths: list"
    fixture fm-no-paths
    printf -- '---\ndescription: loads somehow\n---\n\nBody.\n' >"$F/.claude/rules/odd.md"
    run_guard "$F"
    assert_eq 1 "$CODE" "exits 1"
    assert_contains_fixed "$OUT" "FAIL rule frontmatter" "the frontmatter guard fails"

    current_case="frontmatter never closed"
    fixture fm-unclosed
    printf -- '---\npaths:\n  - "x/**"\n\nBody.\n' >"$F/.claude/rules/odd.md"
    run_guard "$F"
    assert_contains_fixed "$OUT" "FAIL rule frontmatter" "the frontmatter guard fails"
    assert_contains_fixed "$ERR" "never closed" "and says why"

    current_case="frontmatter with an empty paths: list"
    fixture fm-empty
    printf -- '---\npaths:\n---\n\nBody.\n' >"$F/.claude/rules/odd.md"
    run_guard "$F"
    assert_contains_fixed "$OUT" "FAIL rule frontmatter" "the frontmatter guard fails"

    current_case="no frontmatter and a well-formed paths: list"
    fixture fm-good
    printf 'Body only.\n' >"$F/.claude/rules/plain.md"
    printf -- '---\npaths:\n  - "a/**"\n  - b.toml\n---\nBody.\n' >"$F/.claude/rules/list.md"
    run_guard "$F"
    assert_eq 0 "$CODE" "exits 0"
    assert_contains_fixed "$OUT" "ok   rule frontmatter" "both shapes pass"

    # ======================================================================
    current_case="a new unnamed rule file counts against the budget"
    fixture budget-new
    printf 'New rule.\n' >"$F/.claude/rules/brand-new.md"
    assert_eq "$(printf '%s\n' CLAUDE.md .claude/rules/brand-new.md .claude/rules/hard.md)" \
        "$(always_loaded_files "$F")" "the derived set holds the new file and not the scoped one"

    current_case="a nested rule file counts against the budget"
    fixture budget-nested
    mkdir -p "$F/.claude/rules/sub"
    printf 'Nested rule.\n' >"$F/.claude/rules/sub/deep.md"
    before=$(always_loaded_bytes "$BASE")
    assert_eq $((before + 13)) "$(always_loaded_bytes "$F")" "its 13 bytes are counted"

    current_case="a malformed frontmatter file counts against the budget"
    fixture budget-malformed
    printf -- '---\nglobs: x\n---\n' >"$F/.claude/rules/odd.md"
    assert_eq "$(printf '%s\n' CLAUDE.md .claude/rules/hard.md .claude/rules/odd.md)" \
        "$(always_loaded_files "$F")" "a file without a well-formed list is always-loaded"

    current_case="the always-loaded set exactly at the budget"
    fixture budget-at
    printf 'Padding: ' >"$F/.claude/rules/zz-pad.md"
    pad_to "$F/.claude/rules/zz-pad.md" "$BUDGET_BYTES"
    assert_eq "$BUDGET_BYTES" "$(always_loaded_bytes "$F")" "the fixture is exactly at the budget"
    run_guard "$F"
    assert_eq 0 "$CODE" "exits 0"
    assert_contains_fixed "$OUT" "ok   always-loaded budget" "at the budget passes"

    current_case="the always-loaded set one byte over the budget"
    fixture budget-over
    mkdir -p "$F/.claude/rules/sub"
    printf 'Padding: ' >"$F/.claude/rules/sub/zz-pad.md"
    pad_to "$F/.claude/rules/sub/zz-pad.md" $((BUDGET_BYTES + 1))
    run_guard "$F"
    assert_eq 1 "$CODE" "exits 1"
    assert_contains_fixed "$OUT" "FAIL always-loaded budget" "one byte over fails"
    assert_contains_fixed "$ERR" ".claude/rules/sub/zz-pad.md" "the detail lists the nested file"

    current_case="a large scoped rule file does not count"
    fixture budget-scoped
    head -c $((BUDGET_BYTES * 2)) /dev/zero | tr '\0' 'x' >>"$F/.claude/rules/scoped.md"
    run_guard "$F"
    assert_contains_fixed "$OUT" "ok   always-loaded budget" "a paths: file is outside the set"

    # ======================================================================
    # Code fences and spans: a quirk of the document must never switch the
    # link check off for the rest of the file.
    current_case="a nested fence inside a longer fence"
    fixture fence-nested
    printf '\n````md\n```rust\n[in](./in-fence.md)\n```\n````\n\n[x](./missing-after-fence.md)\n' >>"$F/CLAUDE.md"
    run_guard "$F"
    assert_eq 1 "$CODE" "exits 1"
    assert_contains_fixed "$ERR" "missing-after-fence.md" "the link after the outer fence is checked"
    assert_lacks_fixed "$ERR" "in-fence.md" "the link inside the outer fence is not"

    current_case="a line opening with an inline triple-backtick span"
    fixture fence-inline
    printf '\n```foo``` is inline code, not a fence.\n\n[y](./missing-after-span.md)\n' >>"$F/CLAUDE.md"
    run_guard "$F"
    assert_contains_fixed "$OUT" "FAIL relative links resolve" "the link guard fails"
    assert_contains_fixed "$ERR" "missing-after-span.md" "the link after the span is checked"

    current_case="a fence that never closes"
    fixture fence-unclosed
    printf '\n```plaintext\n[z](./missing-in-open-fence.md)\n' >>"$F/CONTRIBUTING.md"
    run_guard "$F"
    assert_contains_fixed "$OUT" "FAIL relative links resolve" "the link guard fails"
    assert_contains_fixed "$ERR" "CONTRIBUTING.md:5: code fence never closes" "the detail names the opening line"

    current_case="double-backtick and line-wrapping code spans"
    fixture span-shapes
    printf '\nA ``span with ` [a](./missing-a.md) inside`` here, and `one that\nwraps [b](./missing-b.md) onto a line` too.\n' >>"$F/CLAUDE.md"
    run_guard "$F"
    assert_contains_fixed "$OUT" "ok   relative links resolve" "links inside spans are syntax"

    current_case="a link after a span on the same line"
    fixture span-then-link
    printf '\nSee `code` then [v](./missing-v.md) and ``x``.\n' >>"$F/CLAUDE.md"
    run_guard "$F"
    assert_contains_fixed "$ERR" "missing-v.md" "the link outside the span is checked"

    current_case="reference definitions"
    fixture refdef
    printf '\nSee [the doc][d] and a note[^1].\n\n[d]: ./missing-ref.md "title"\n[^1]: a footnote, not a link\n[ok]: ./README.md#install\n' >>"$F/CONTRIBUTING.md"
    run_guard "$F"
    assert_contains_fixed "$OUT" "FAIL relative links resolve" "the link guard fails"
    assert_contains_fixed "$ERR" "CONTRIBUTING.md:7: ./missing-ref.md" "the broken definition is reported"
    assert_lacks_fixed "$ERR" "footnote" "a footnote is not a link"
    assert_lacks_fixed "$ERR" "README.md#install" "a resolving definition passes"

    current_case="BUGBOT carries a reference definition"
    fixture bugbot-refdef
    printf 'See [the rule][r].\n\n[r]: ./x.md\n' >>"$F/.cursor/BUGBOT.md"
    run_guard "$F"
    assert_contains_fixed "$OUT" "FAIL BUGBOT is self-contained" "the guard fails"
    assert_contains_fixed "$ERR" "(Markdown reference definition;" "the reference half fired"

    # ======================================================================
    current_case="an unquoted list item YAML reads as an alias"
    fixture fm-alias
    printf -- '---\npaths:\n  - **/*.rs\n---\nBody.\n' >"$F/.claude/rules/alias.md"
    run_guard "$F"
    assert_contains_fixed "$OUT" "FAIL rule frontmatter" "the frontmatter guard fails"
    assert_contains_fixed "$ERR" "not a quoted string or a plain path" "and says why"
    always_loaded_files "$F" >"$SANDBOX/al"
    assert_contains_fixed "$SANDBOX/al" ".claude/rules/alias.md" "and the file counts against the budget"

    current_case="a tab-indented list item"
    fixture fm-tab
    printf -- '---\npaths:\n\t- "x/**"\n---\nBody.\n' >"$F/.claude/rules/tab.md"
    run_guard "$F"
    assert_contains_fixed "$OUT" "FAIL rule frontmatter" "the frontmatter guard fails"
    assert_contains_fixed "$ERR" "tab in the indentation" "and says why"

    current_case="quoted, single-quoted and plain items"
    fixture fm-shapes
    printf -- "---\npaths:\n  - \"**/*.rs\"  # comment\n  - '*.toml'\n  - crates/x/**\n  - .github/**\n---\nBody.\n" >"$F/.claude/rules/shapes.md"
    run_guard "$F"
    assert_contains_fixed "$OUT" "ok   rule frontmatter" "all three spellings pass"

    # ======================================================================
    current_case="a symlinked rule file and a symlinked rule directory"
    fixture budget-symlink
    mkdir -p "$SANDBOX/ext/dir"
    head -c $((BUDGET_BYTES + 1)) /dev/zero | tr '\0' 'x' >"$SANDBOX/ext/big.md"
    printf 'Linked rule.\n' >"$SANDBOX/ext/dir/linked.md"
    ln -s "$SANDBOX/ext/big.md" "$F/.claude/rules/big.md"
    ln -s "$SANDBOX/ext/dir" "$F/.claude/rules/ext"
    run_guard "$F"
    assert_contains_fixed "$OUT" "FAIL always-loaded budget" "the linked file is counted"
    always_loaded_files "$F" >"$SANDBOX/al"
    assert_contains_fixed "$SANDBOX/al" ".claude/rules/ext/linked.md" "a linked directory is scanned"

    current_case="an unreadable file fails instead of passing"
    fixture unreadable
    printf 'See [gone](./nowhere.md).\n' >>"$F/README.md"
    chmod 000 "$F/README.md"
    if [ -r "$F/README.md" ]; then
        # Running as root: permissions do not stop the read, so there is no
        # failure to provoke. Count it rather than report a false pass.
        pass "skipped: running as root"
    else
        run_guard "$F"
        assert_eq 1 "$CODE" "exits 1"
        assert_contains_fixed "$OUT" "FAIL README pins the installer" "the grep error fails the installer guard"
        assert_contains_fixed "$OUT" "FAIL relative links resolve" "the awk error fails the link guard"
    fi
    chmod 644 "$F/README.md"

    # ======================================================================
    # A missing scope fails the checks that need it. Each case asserts the
    # WHOLE ok/FAIL output, so a check that does not require the path is
    # proved to stay ok, and asserts each failing check's own "required"
    # detail line, so a check that would fail anyway (a link to the removed
    # file) is proved to fail for the missing path too.
    expect_scope_failures() { # missing-path check-names (newline-separated)
        local missing=$1 failing=$2 name expected=""
        while IFS= read -r name || [ -n "$name" ]; do
            [ -n "$name" ] || continue
            if printf '%s\n' "$failing" | grep -qxF -- "$name"; then
                expected="${expected}FAIL $name
"
                assert_contains_fixed "$ERR" "error: $name: $missing: required" "$name names $missing"
            else
                expected="${expected}ok   $name
"
            fi
        done <<'NAMES'
retired subcommand spelling
skills name no CLAUDE_PLUGIN_DATA
README pins the installer
BUGBOT is self-contained
relative links resolve
rule frontmatter
always-loaded budget
NAMES
        assert_eq 1 "$CODE" "exits 1"
        assert_eq "$expected" "$(cat "$OUT")
" "exactly the checks that need $missing fail"
    }

    current_case="an empty ROOT"
    mkdir -p "$SANDBOX/empty"
    run_guard "$SANDBOX/empty"
    assert_eq 1 "$CODE" "exits 1"
    assert_eq 7 "$(grep -c '^FAIL ' "$OUT")" "every check fails"
    assert_eq 7 "$(wc -l <"$OUT" | tr -d ' ')" "still one line per check"
    for p in CLAUDE.md README.md .cursor/BUGBOT.md .claude/rules/ plugin/skills/; do
        assert_contains_fixed "$ERR" ": $p: required" "the detail names $p"
    done

    current_case="CLAUDE.md missing"
    fixture scope-claude
    rm -f "$F/CLAUDE.md"
    run_guard "$F"
    expect_scope_failures CLAUDE.md "retired subcommand spelling
relative links resolve
always-loaded budget"

    current_case=".claude/rules/ missing"
    fixture scope-rules
    rm -rf "$F/.claude/rules"
    run_guard "$F"
    expect_scope_failures .claude/rules/ "retired subcommand spelling
relative links resolve
rule frontmatter
always-loaded budget"

    current_case=".claude/rules is a file, not a directory"
    fixture scope-rules-file
    rm -rf "$F/.claude/rules"
    printf 'not a directory\n' >"$F/.claude/rules"
    run_guard "$F"
    assert_contains_fixed "$OUT" "FAIL rule frontmatter" "a file in its place is not the directory"
    assert_contains_fixed "$ERR" "error: rule frontmatter: .claude/rules/: required directory is missing" "and says why"

    current_case="README.md missing"
    fixture scope-readme
    rm -f "$F/README.md"
    run_guard "$F"
    expect_scope_failures README.md "retired subcommand spelling
README pins the installer
relative links resolve"

    current_case=".cursor/BUGBOT.md missing"
    fixture scope-bugbot
    rm -f "$F/.cursor/BUGBOT.md"
    run_guard "$F"
    expect_scope_failures .cursor/BUGBOT.md "retired subcommand spelling
BUGBOT is self-contained"

    current_case="plugin/skills/ missing"
    fixture scope-skills
    rm -rf "$F/plugin/skills"
    run_guard "$F"
    expect_scope_failures plugin/skills/ "retired subcommand spelling
skills name no CLAUDE_PLUGIN_DATA
relative links resolve"

    current_case="an optional scope path missing"
    fixture scope-optional
    rm -rf "$F/CONCEPTS.md" "$F/docs/runbooks"
    run_guard "$F"
    assert_eq 0 "$CODE" "exits 0"
    assert_eq 7 "$(grep -c '^ok   ' "$OUT")" "seven ok lines"

    # ======================================================================
    current_case="several failures at once"
    fixture many
    printf 'ss-magic plugin status\n' >>"$F/CLAUDE.md"
    printf 'See [x](./x.md)\n' >>"$F/.cursor/BUGBOT.md"
    run_guard "$F"
    assert_eq 1 "$CODE" "exits 1"
    assert_eq 7 "$(wc -l <"$OUT" | tr -d ' ')" "still one line per check"
    assert_eq 2 "$(grep -c '^FAIL ' "$OUT")" "exactly the two failing checks"

    printf '\n%s --selftest: %d passed, %d failed\n' "$(basename "$SELF")" "$passed" "$failed"
    [ "$failed" -eq 0 ] || exit 1
    exit 0
}

case "${1:-}" in
    --selftest)
        selftest "${2:-}"
        ;;
    -h|--help)
        # The header comment: every line after the shebang up to the first
        # line that is not a comment.
        awk 'NR == 1 { next } /^#/ { print; next } { exit }' "$SELF" | sed 's/^# \{0,1\}//'
        exit 0
        ;;
    -*)
        printf 'check-docs.sh: unknown option %s (try --help)\n' "$1" >&2
        exit 2
        ;;
    *)
        ROOT=${1:-$DEFAULT_ROOT}
        if [ ! -d "$ROOT" ]; then
            printf 'check-docs.sh: %s is not a directory\n' "$ROOT" >&2
            exit 2
        fi
        ROOT=$(cd "$ROOT" && pwd)
        run_checks "$ROOT"
        exit $?
        ;;
esac
