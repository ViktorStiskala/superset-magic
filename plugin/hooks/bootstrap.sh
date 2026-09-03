#!/usr/bin/env bash
#
# SessionStart bootstrap: put the pinned `ss-magic-plugin` binary at
# ${CLAUDE_PLUGIN_DATA}/bin/ss-magic-plugin, or leave the machine exactly as it
# was.
#
# This script runs on every fresh session on every machine that has the plugin
# enabled, so its failure behaviour matters more than its success behaviour:
#
#   * There is deliberately NO `set -e`. Every path here ends in `exit 0`. A
#     non-zero exit, or a hang, from a SessionStart hook is a broken session for
#     the user, and no binary is worth that (R72). Offline, DNS failure, proxy,
#     404, checksum mismatch, unwritable data directory, unsupported platform:
#     all of them are "do nothing, say one line, exit 0".
#   * The success path prints NOTHING on stdout. A hook's stdout is fed into the
#     model's context at every session start, so silence is a token-budget rule
#     rather than a style preference (R72). Diagnostics go to stderr, and a
#     failure reports exactly one line there.
#   * An existing binary is never touched by a failing install. The download is
#     verified and staged first, and only a verified binary is moved into place.
#
# The install target is ${CLAUDE_PLUGIN_DATA}, never ${CLAUDE_PLUGIN_ROOT}: the
# plugin root is version-scoped and is replaced wholesale on every plugin
# update, so a binary installed there would be discarded on each bump (R70).
# The pin lives beside plugin.json inside that version-scoped root, which is
# what makes a plugin update the thing that triggers a binary update - the two
# cannot drift.
#
# There is deliberately NO cleanup of a stale ${CLAUDE_PLUGIN_DATA}/bin/ss-magic
# left behind by a pre-split install, when the plugin still shipped inside the
# `ss-magic` CLI binary and was reached as `ss-magic plugin <verb>`. Nothing
# spawns that path any more - hooks.json goes through run-hook.sh, and both
# run-hook.sh and bin/ss-magic-plugin now name bin/ss-magic-plugin - so the old
# file is inert wherever it survives. Deleting it would mean carrying migration
# code that runs on every machine forever to tidy one dead file on a handful of
# dogfooding machines, which is a worse trade: a bug in that code runs at every
# session start, while the file it removes costs a few megabytes and nothing
# else. Anyone who wants it gone deletes it by hand.

set -u

RELEASE_DOWNLOAD_BASE="https://github.com/ViktorStiskala/superset-magic/releases/download"
RELEASE_PAGE_BASE="https://github.com/ViktorStiskala/superset-magic/releases/tag"

# Bounded so the whole run fits comfortably inside the 90 s timeout hooks.json
# declares for this entry. Worst case is lock wait + both fetches, and there is
# no --retry on purpose: a transient failure costs nothing, because the next
# fresh session simply tries again.
CONNECT_TIMEOUT=8
ARCHIVE_MAX_TIME=40
DIGEST_MAX_TIME=15
LOCK_WAIT_SECONDS=20

stage=""
state_file=""

cleanup() {
    [ -n "$stage" ] && rm -rf "$stage" 2>/dev/null
    return 0
}
trap cleanup EXIT
# A hook killed at its timeout would otherwise leave a staging directory
# behind; EXIT alone does not fire on a signal.
trap 'cleanup; exit 0' INT TERM HUP

# The only failure exit in the file: drop the success marker so the next session
# retries instead of trusting whatever is on disk (R73), report one line, and
# still exit 0 (R72).
give_up() {
    [ -n "$state_file" ] && rm -f "$state_file" 2>/dev/null
    printf 'ss-magic: %s\n' "$1" >&2
    exit 0
}

# Write one line to a file, silently. The braces matter: `cmd > path 2>/dev/null`
# suppresses the COMMAND's stderr, not the SHELL's own "cannot create" message
# when the redirect itself fails - and every marker written here sits in a
# directory that may legitimately be unwritable, which must cost zero output.
write_line() {
    { printf '%s\n' "$2" >"$1"; } 2>/dev/null
}

# ---------------------------------------------------------------------------
# Where things live
# ---------------------------------------------------------------------------

plugin_root=${CLAUDE_PLUGIN_ROOT:-}
if [ -z "$plugin_root" ]; then
    # Not invoked as a hook. Fall back to this script's own directory's parent
    # so a manual run still works; the harness always sets the variable.
    plugin_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." 2>/dev/null && pwd) || exit 0
fi

# ${CLAUDE_PLUGIN_DATA} is set for hook processes. The fallback reproduces the
# documented layout, <config dir>/plugins/data/<plugin>-<marketplace>/, where
# every character outside [a-zA-Z0-9_-] is replaced - so "ss-magic@ss-magic"
# becomes "ss-magic-ss-magic".
data=${CLAUDE_PLUGIN_DATA:-}
if [ -z "$data" ]; then
    data="${CLAUDE_CONFIG_DIR:-${HOME:-}/.claude}/plugins/data/ss-magic-ss-magic"
fi

# The installed binary is `ss-magic-plugin`; the marker file names below keep
# their historical `.ss-magic-` spelling on purpose. They are per-machine state
# keyed to the marketplace plugin name, which is still `ss-magic`, so renaming
# them would strand every existing install's markers and re-run the one-time
# disclosure on machines that have already seen it.
bin_path="$data/bin/ss-magic-plugin"
state_file="$data/.ss-magic-installed"
disclosed_marker="$data/.ss-magic-disclosed"
unsupported_marker="$data/.ss-magic-unsupported"

# shellcheck source=../lib/tmproot.sh
lib="$plugin_root/lib/tmproot.sh"
if [ -r "$lib" ]; then
    . "$lib" 2>/dev/null || true
fi

# ---------------------------------------------------------------------------
# The Bash-visible handoff (R75)
# ---------------------------------------------------------------------------

# ${CLAUDE_PLUGIN_DATA} reaches hook and MCP/LSP processes but NOT the Bash tool,
# so `bin/ss-magic-plugin` - which a skill body invokes through the Bash tool -
# cannot read it. Publish the resolved value into the one directory both
# processes can compute from $HOME alone. Written on every run, including the
# silent no-op path, because /tmp does not survive a reboot while the installed
# binary does.
#
# Written through a temporary file and renamed, so a wrapper reading it
# concurrently sees either the old complete line or the new complete line and
# never a half-written path.
publish_data_root() {
    local root tmp
    command -v ss_magic_resolve_root >/dev/null 2>&1 || return 0
    root=$(ss_magic_resolve_root create) || return 0
    tmp="$root/.$SS_MAGIC_DATA_ROOT_FILE.$$"
    if write_line "$tmp" "$data"; then
        mv -f "$tmp" "$root/$SS_MAGIC_DATA_ROOT_FILE" 2>/dev/null || rm -f "$tmp" 2>/dev/null
    else
        rm -f "$tmp" 2>/dev/null
    fi
    return 0
}
publish_data_root

# ---------------------------------------------------------------------------
# The pin, validated before it is used for anything at all (R71)
# ---------------------------------------------------------------------------

# Whoever can write ss-magic-plugin.version decides what every installed machine
# downloads and executes at session start, so it is a supply-chain boundary and
# is treated as untrusted input. It is validated as a bare MAJOR.MINOR.PATCH
# literal BEFORE it is compared, interpolated, or allowed anywhere near a URL:
# a pin of `1.2.3; rm -rf ~` or `../../../etc` never reaches a command line.
pin_file="$plugin_root/ss-magic-plugin.version"
[ -r "$pin_file" ] || give_up "no version pin at $pin_file; installed nothing."

pin=$(tr -d '[:space:]' <"$pin_file" 2>/dev/null)
# Two passes, because glob patterns cannot count. The first rejects every
# character outside [0-9.] and every empty field (a leading dot, a trailing dot,
# a doubled dot); the second then only has to require exactly two dots, which is
# what makes the survivors exactly MAJOR.MINOR.PATCH. A leading `v`, a
# pre-release suffix, a path, a URL and a shell metacharacter all die in the
# first pass.
case "$pin" in
    *[!0-9.]*|.*|*.|*..*)
        give_up "version pin is not a MAJOR.MINOR.PATCH literal; installed nothing." ;;
esac
case "$pin" in
    *.*.*.*) give_up "version pin is not a MAJOR.MINOR.PATCH literal; installed nothing." ;;
    *.*.*) ;;
    *) give_up "version pin is not a MAJOR.MINOR.PATCH literal; installed nothing." ;;
esac

# ---------------------------------------------------------------------------
# Already installed? (R70)
# ---------------------------------------------------------------------------

# Both halves must agree before this is a no-op: the marker says the last
# install ran to completion (R73) and the binary itself answers with the pinned
# version. `ss-magic-plugin --version` prints `ss-magic-plugin <version>` on one
# line and exits 0 ahead of any verb parsing, so it is safe to call with no TTY,
# never prompts, and costs one fast process spawn. `awk '{print $NF}'` takes the
# last field of that line, which is the bare version.
installed_version() {
    [ -x "$bin_path" ] || return 1
    "$bin_path" --version 2>/dev/null | head -1 | awk '{print $NF}'
}

already_installed() {
    local marked current
    # `cat ... | tr` rather than a `< "$state_file"` redirect: a redirect from a
    # missing file makes the SHELL print "No such file or directory", which
    # 2>/dev/null on the redirected command does not suppress - and the marker
    # being absent is the normal first-run state, not an error worth a line.
    marked=$(cat "$state_file" 2>/dev/null | tr -d '[:space:]')
    [ "$marked" = "$pin" ] || return 1
    current=$(installed_version) || return 1
    [ "$current" = "$pin" ]
}

# ---------------------------------------------------------------------------
# Pre-seed the configuration block (R3a)
# ---------------------------------------------------------------------------

# The plugin used to be reachable from a terminal as `ss-magic plugin config
# set ...`. It is its own binary now and the CLI has dropped the `plugin`
# subcommand entirely, so there is no longer any terminal path a person would
# discover for turning the plugin on or adjusting its gate. Nothing installs
# `ss-magic-plugin` onto a user's PATH either - it lives under
# ${CLAUDE_PLUGIN_DATA}, a directory a person is not expected to know about.
#
# So the settings are made discoverable in the file people already edit: this
# folds a `plugin` block of GATE DEFAULTS into an EXISTING
# .superset/magic.json, where the knobs can be read and changed by hand.
#
# WHERE THIS IS CALLED FROM, AND WHY IT IS NOT ONE PLACE. The binary is
# installed once per MACHINE, but the block has to be seeded once per
# REPOSITORY - and a person opens many repositories on one machine. Calling
# this only after a fresh install would therefore seed the first repository
# and silently skip every one after it, because `already_installed` returns
# above that point on every later session. That is the steady state, so the
# bug would be the common case. It is called instead from both places where a
# usable pinned binary is known to exist: the already-installed fast path just
# below, and the end of a successful install.
#
# Calling it on every session is safe because the ONCE-ness lives in the
# binary, not in the caller: it writes only when `.superset/magic.json` has no
# `plugin` key at all. So a repeat call is a read and an exit, measured at
# about 6 ms, and there is no marker file to keep in sync.
#
# What the call never does - all of it enforced inside the binary, none of it
# here:
#   * it never writes the `enabled` key, so installing the plugin cannot turn
#     the plugin on. Enablement stays a deliberate human act, which is the
#     property that stops a repository from arranging its own enablement;
#   * it never creates .superset/magic.json, so a repository that does not use
#     ss-magic is left completely untouched;
#   * it never writes through a symlink that leaves the repository;
#   * it never stages anything with git, so the change shows up as an ordinary
#     unstaged edit the user reviews like any other;
#   * it does nothing at all when a `plugin` key already exists, so a
#     hand-tuned block is never rewritten.
#
# Both streams are discarded and the exit status is ignored: the bootstrap's
# contract is unchanged (no `set -e`, silent on stdout, every path exits 0),
# and nothing about seeding a config file is worth failing a session start
# for. It runs in this script's own working directory, which the harness sets
# to the user's repository - that is how the binary finds the file, and why
# there is deliberately no `cd` anywhere in this script.
seed_config() {
    "$bin_path" seed-config >/dev/null 2>&1
    return 0
}

if already_installed; then
    seed_config
    exit 0
fi

# ---------------------------------------------------------------------------
# Platform (R78)
# ---------------------------------------------------------------------------

# cargo-dist publishes {aarch64,x86_64} x {apple-darwin,unknown-linux-gnu} and
# nothing else - there is no Windows target. Anywhere else this script installs
# nothing and says so ONCE, because repeating it on every session start would
# make the plugin permanently noisy on a machine it can never serve.
uname_s=$(uname -s 2>/dev/null)
uname_m=$(uname -m 2>/dev/null)

os_part=""
case "$uname_s" in
    Darwin) os_part="apple-darwin" ;;
    Linux) os_part="unknown-linux-gnu" ;;
esac
arch_part=""
case "$uname_m" in
    arm64|aarch64) arch_part="aarch64" ;;
    x86_64|amd64) arch_part="x86_64" ;;
esac

if [ -z "$os_part" ] || [ -z "$arch_part" ]; then
    signature="${uname_s:-unknown} ${uname_m:-unknown}"
    if [ "$(cat "$unsupported_marker" 2>/dev/null)" = "$signature" ]; then
        exit 0
    fi
    mkdir -p "$data" 2>/dev/null
    write_line "$unsupported_marker" "$signature"
    printf 'ss-magic: no published release binary for %s; the plugin is inactive here.\n' \
        "$signature" >&2
    exit 0
fi
triple="$arch_part-$os_part"

# ---------------------------------------------------------------------------
# Serialise concurrent sessions (R73, AE61)
# ---------------------------------------------------------------------------

# Two sessions can start at the same second on a machine where neither finds a
# binary, and both would then download the same archive. The lock is taken on
# the R80 root's install.lock - the exact file that
# crates/ss-magic-plugin/src/tmproot.rs locks from Rust - by re-executing this
# script under a lock holder.
#
# `flock(1)` is the holder where it exists (Linux). macOS does not ship it, so
# perl's flock() - the same flock(2) underneath, on the same file - stands in.
# Where neither exists the install proceeds UNLOCKED, and is still correct: the
# staging directory is per-process and the final step is a rename(2) onto the
# destination, so the loser of a race overwrites the winner with byte-identical,
# checksum-verified content. The lock saves a duplicate download; it is not what
# makes concurrent installs safe.
#
# Note the placement: locking happens AFTER the "already installed" check, so
# the overwhelmingly common no-op session never opens the lock file at all and
# never waits behind an install.
if [ -z "${SS_MAGIC_BOOTSTRAP_LOCKED:-}" ] && command -v ss_magic_resolve_root >/dev/null 2>&1; then
    lock_root=$(ss_magic_resolve_root create) && {
        lock_file="$lock_root/$SS_MAGIC_INSTALL_LOCK_NAME"
        export SS_MAGIC_BOOTSTRAP_LOCKED=1
        # Deliberately NOT `exec`. Exec would replace this process, making the
        # lock holder's exit status the hook's own - and flock(1) exits 1 on a
        # timed-out wait, which would turn "another session is installing" into
        # a failed SessionStart hook. Calling it and then exiting 0
        # unconditionally is what keeps R72 true no matter what the holder does.
        if command -v flock >/dev/null 2>&1; then
            flock -w "$LOCK_WAIT_SECONDS" "$lock_file" bash "${BASH_SOURCE[0]}"
            exit 0
        elif command -v perl >/dev/null 2>&1; then
            perl -e '
                use Fcntl qw(:flock);
                my ($path, $wait) = (shift, shift);
                if (open(my $fh, ">>", $path)) {
                    local $SIG{ALRM} = sub { exit 0 };
                    alarm($wait);
                    flock($fh, LOCK_EX) or exit 0;
                    alarm(0);
                }
                my $rc = system(@ARGV);
                exit($rc == -1 ? 0 : $rc >> 8);
            ' "$lock_file" "$LOCK_WAIT_SECONDS" bash "${BASH_SOURCE[0]}"
            exit 0
        fi
    }
fi

# Re-check under the lock: the session we just waited behind may have installed
# exactly what we were about to download. Seed here too - this session reached
# a usable binary, it just did not install it itself.
if already_installed; then
    seed_config
    exit 0
fi

# ---------------------------------------------------------------------------
# Fetch, verify, extract (R71, KTD17)
# ---------------------------------------------------------------------------

# The archive is fetched directly rather than by piping a cargo-dist installer
# script into a shell. The release publishes a .sha256 sibling for every .tar.gz
# but none for an installer script, so such a script would be the one executed
# artifact no published digest covers. Fetching the archive makes the verified
# thing and the executed thing the same thing.
downloader=""
if command -v curl >/dev/null 2>&1; then
    downloader=curl
elif command -v wget >/dev/null 2>&1; then
    downloader=wget
else
    give_up "neither curl nor wget is available; installed nothing."
fi

# $1 url, $2 destination, $3 max seconds. Silent on both streams; the caller
# turns a non-zero return into the one line this script is allowed to print.
fetch() {
    case "$downloader" in
        curl)
            curl --proto '=https' --tlsv1.2 -fsSL \
                --connect-timeout "$CONNECT_TIMEOUT" --max-time "$3" \
                -o "$2" "$1" >/dev/null 2>&1
            ;;
        wget)
            wget -q --https-only --timeout="$CONNECT_TIMEOUT" --tries=1 \
                -O "$2" "$1" >/dev/null 2>&1
            ;;
    esac
}

digest_of() {
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" 2>/dev/null | cut -d' ' -f1
    elif command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" 2>/dev/null | cut -d' ' -f1
    else
        return 1
    fi
}

mkdir -p "$data" 2>/dev/null
# The staging directory sits under the data directory rather than in /tmp, and
# that is load-bearing: it puts the staged binary on the same filesystem as its
# destination, so the final install step is a single rename(2) - atomic, with no
# window in which a half-copied binary is visible at the invocation path.
stage=$(mktemp -d "$data/.ss-magic-stage.XXXXXX" 2>/dev/null) ||
    give_up "cannot write to $data; installed nothing."

# The plugin binary rides its OWN release line. One repository publishes two:
# the `ss-magic` CLI on bare `vX.Y.Z` tags, and this binary on
# `ss-magic-plugin-vX.Y.Z`. The tag and the asset name both carry the
# `ss-magic-plugin` prefix, so a pin of 1.0.0 resolves to
# .../download/ss-magic-plugin-v1.0.0/ss-magic-plugin-<triple>.tar.gz and can
# never collide with the CLI's asset for the same version number.
archive_name="ss-magic-plugin-$triple.tar.gz"
archive_url="$RELEASE_DOWNLOAD_BASE/ss-magic-plugin-v$pin/$archive_name"
archive_path="$stage/$archive_name"

fetch "$archive_url" "$archive_path" "$ARCHIVE_MAX_TIME" ||
    give_up "could not download $archive_url; installed nothing."
fetch "$archive_url.sha256" "$archive_path.sha256" "$DIGEST_MAX_TIME" ||
    give_up "could not download $archive_url.sha256; installed nothing."

expected=$(cut -d' ' -f1 <"$archive_path.sha256" 2>/dev/null | head -1 | tr 'A-Z' 'a-z')
case "$expected" in
    [0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]) ;;
    *) give_up "the published checksum for $archive_name is unreadable; installed nothing." ;;
esac

actual=$(digest_of "$archive_path") ||
    give_up "no SHA-256 tool available to verify $archive_name; installed nothing."
if [ "$actual" != "$expected" ]; then
    give_up "checksum mismatch for $archive_name; installed nothing."
fi

mkdir -p "$stage/x" 2>/dev/null
tar -xzf "$archive_path" -C "$stage/x" >/dev/null 2>&1 ||
    give_up "could not extract $archive_name; installed nothing."

# cargo-dist's tarball layout is <bin>-<target>/<bin>. The flat form is accepted
# as a fallback so a layout change degrades into a working install rather than a
# silent no-op.
staged_bin="$stage/x/ss-magic-plugin-$triple/ss-magic-plugin"
[ -f "$staged_bin" ] || staged_bin="$stage/x/ss-magic-plugin"
[ -f "$staged_bin" ] ||
    give_up "$archive_name did not contain an ss-magic-plugin binary; installed nothing."
chmod 0755 "$staged_bin" 2>/dev/null

# The checksum already proves this is the published artifact for this triple;
# this confirms it is the artifact for this MACHINE. A wrong-architecture binary
# passes every check above and then fails to execute, which without this check
# would install successfully and quietly break every hook.
staged_version=$("$staged_bin" --version 2>/dev/null | head -1 | awk '{print $NF}')
[ "$staged_version" = "$pin" ] ||
    give_up "the downloaded ss-magic-plugin did not run as $pin here; installed nothing."

# ---------------------------------------------------------------------------
# Install (R73)
# ---------------------------------------------------------------------------

mkdir -p "$data/bin" 2>/dev/null ||
    give_up "cannot create $data/bin; installed nothing."

# One rename, same filesystem, replacing whatever was there. A process already
# executing the old binary keeps running it; the next invocation gets the new
# one. Nothing before this line has touched the installed binary, which is what
# makes every failure above leave a working older install alone.
mv -f "$staged_bin" "$bin_path" 2>/dev/null ||
    give_up "could not install into $bin_path; installed nothing."

# The marker goes in only now, after the move landed. Its absence is what tells
# the next session to retry rather than trust a tree that may be half-written.
write_line "$state_file" "$pin"

# Seed the configuration block now that a usable binary is in place. See
# `seed_config` above for what it does and why it is called from more than one
# site. It sits before the one-time disclosure so that message stays the last
# thing a first install prints.
seed_config


# ---------------------------------------------------------------------------
# One-time disclosure (R79)
# ---------------------------------------------------------------------------

# Emitted after the first install that actually succeeds on this machine, and
# never again. It names what was installed, where it came from, and the fact
# that the plugin registers machine-global hooks - all three are things a user
# is entitled to be told once, and none of them is worth repeating at every
# session start. On stderr, like every other message here, because stdout goes
# into the model's context.
if [ ! -f "$disclosed_marker" ]; then
    {
        printf 'ss-magic plugin: installed ss-magic-plugin %s into %s\n' "$pin" "$bin_path"
        printf '  from %s/ss-magic-plugin-v%s (archive verified against its published SHA-256).\n' \
            "$RELEASE_PAGE_BASE" "$pin"
        printf '  It registers hooks that run for every session while the plugin is enabled:\n'
        printf '  SessionStart, PreToolUse, PreCompact, SubagentStop, SessionEnd.\n'
        printf '  This notice appears once per machine; later sessions are silent.\n'
    } >&2
    write_line "$disclosed_marker" "$pin"
fi

exit 0
