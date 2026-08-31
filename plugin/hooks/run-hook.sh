#!/usr/bin/env bash
#
# The hook entry point: run `ss-magic plugin hook <event>` if the pinned binary
# is there, and do nothing at all if it is not.
#
# Every hook in hooks.json is spawned through this script rather than naming the
# binary directly, and that indirection is the whole point of the file.
# ${CLAUDE_PLUGIN_DATA}/bin/ss-magic does not exist until the SessionStart
# bootstrap fetches it, and hooks on one event fire CONCURRENTLY - so on a first
# install the harness would posix_spawn a path that is not there yet and the
# session would surface ENOENT. A hook that cannot do its job must be
# indistinguishable from one that decided to do nothing (R26, R72), and R77
# spells out the case: "no binary exists at the invocation path, so every
# ss-magic hook is inert for that session". Inert, not an error.
#
# The binary implements that fail-open itself - hook::run has no non-zero exit
# path - but that code is unreachable when the binary is the missing thing. This
# script is the layer where the guarantee has to live instead, because it ships
# inside ${CLAUDE_PLUGIN_ROOT} and therefore exists from the moment the plugin is
# installed.
#
# It is silent on BOTH streams, which is what separates it from
# bin/ss-magic-plugin. That wrapper prints one explanatory stderr line when the
# binary is missing, and that is right for its consumer: a person who ran a skill
# through the Bash tool and needs to know why nothing happened. This runs on
# PreToolUse, which fires on essentially every tool call, so the same line would
# be emitted dozens of times per session for the whole first session. Stdout
# silence is separately required: a SessionStart hook's stdout is fed into the
# model's context at every session start.
#
# The binary resolution itself is NOT reimplemented here - lib/tmproot.sh is
# sourced, exactly as the bootstrap and the wrapper do, so the three cannot drift
# about where the handoff lives.

set -u

# No `set -e`, and every path below ends in `exit 0`. A non-zero exit from a hook
# is a broken session for the user, and nothing this script does is worth that.
give_up() {
    exit 0
}

event=${1-}
[ -n "$event" ] || give_up

self_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd) || give_up

# ${CLAUDE_PLUGIN_DATA} IS exported to hook processes, so the common path never
# needs the handoff at all. The fallback exists because this script is also the
# one place a hook can run before the variable is meaningful, and because reusing
# the wrapper's resolution keeps a single definition of the R80 root.
data=${CLAUDE_PLUGIN_DATA:-}
if [ -z "$data" ]; then
    lib="$self_dir/../lib/tmproot.sh"
    [ -r "$lib" ] || give_up
    # shellcheck source=../lib/tmproot.sh
    . "$lib" || give_up

    # Read-only: a root this script had to create would by definition hold no
    # handoff to read.
    root=$(ss_magic_resolve_root) || give_up
    handoff="$root/$SS_MAGIC_DATA_ROOT_FILE"
    [ -r "$handoff" ] || give_up
    # The braces matter, and bootstrap.sh's `write_line` documents the same
    # trap: `cmd <FILE 2>/dev/null` redirects the stderr of the COMMAND, not of
    # the shell reporting that the input redirection itself failed. Redirections
    # apply left to right, so a `<"$handoff"` that fails prints before
    # `2>/dev/null` is in effect. Reachable as a TOCTOU - the check above passes
    # and a /tmp sweeper removes the file before it is opened - and this hook has
    # no stderr budget at all.
    { IFS= read -r data <"$handoff"; } 2>/dev/null
    [ -n "${data:-}" ] || give_up
fi

bin="$data/bin/ss-magic"
# `-f` as well as `-x`: `-x` alone is TRUE for a directory carrying the search
# bit, and `exec` on a directory does not fail quietly - bash prints a diagnostic
# and exits 126, which on PreToolUse would mean an error line per tool call from
# the one script whose whole job is to be silent.
[ -f "$bin" ] && [ -x "$bin" ] || give_up

exec "$bin" plugin hook "$event"
