#!/usr/bin/env bash
#
# SessionStart bootstrap: put the pinned `ss-magic` binary at
# ${CLAUDE_PLUGIN_DATA}/bin/ss-magic, or leave the machine exactly as it was.
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

bin_path="$data/bin/ss-magic"
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

# Whoever can write ss-magic.version decides what every installed machine
# downloads and executes at session start, so it is a supply-chain boundary and
# is treated as untrusted input. It is validated as a bare MAJOR.MINOR.PATCH
# literal BEFORE it is compared, interpolated, or allowed anywhere near a URL:
# a pin of `1.2.3; rm -rf ~` or `../../../etc` never reaches a command line.
pin_file="$plugin_root/ss-magic.version"
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
# version. `--version` short-circuits before the update gate and before the TUI,
# so it is safe to call with no TTY and costs one fast process spawn.
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

already_installed && exit 0

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
# the R80 root's install.lock - the exact file src/plugin/tmproot.rs locks from
# Rust - by re-executing this script under a lock holder.
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
# exactly what we were about to download.
already_installed && exit 0

# ---------------------------------------------------------------------------
# Fetch, verify, extract (R71, KTD17)
# ---------------------------------------------------------------------------

# The archive is fetched directly rather than by piping ss-magic-installer.sh
# into a shell. The release publishes a .sha256 sibling for every .tar.gz but
# none for the installer script, so the installer is the one executed artifact
# no published digest covers. Fetching the archive makes the verified thing and
# the executed thing the same thing.
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

archive_name="ss-magic-$triple.tar.gz"
archive_url="$RELEASE_DOWNLOAD_BASE/v$pin/$archive_name"
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
staged_bin="$stage/x/ss-magic-$triple/ss-magic"
[ -f "$staged_bin" ] || staged_bin="$stage/x/ss-magic"
[ -f "$staged_bin" ] ||
    give_up "$archive_name did not contain an ss-magic binary; installed nothing."
chmod 0755 "$staged_bin" 2>/dev/null

# The checksum already proves this is the published artifact for this triple;
# this confirms it is the artifact for this MACHINE. A wrong-architecture binary
# passes every check above and then fails to execute, which without this check
# would install successfully and quietly break every hook.
staged_version=$("$staged_bin" --version 2>/dev/null | head -1 | awk '{print $NF}')
[ "$staged_version" = "$pin" ] ||
    give_up "the downloaded ss-magic did not run as $pin here; installed nothing."

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
        printf 'ss-magic plugin: installed ss-magic %s into %s\n' "$pin" "$bin_path"
        printf '  from %s/v%s (archive verified against its published SHA-256).\n' \
            "$RELEASE_PAGE_BASE" "$pin"
        printf '  It registers hooks that run for every session while the plugin is enabled:\n'
        printf '  SessionStart, PreToolUse, PreCompact, SubagentStop, SessionEnd.\n'
        printf '  This notice appears once per machine; later sessions are silent.\n'
    } >&2
    write_line "$disclosed_marker" "$pin"
fi

exit 0
