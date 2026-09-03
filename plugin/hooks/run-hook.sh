#!/usr/bin/env bash
#
# The hook entry point: run `ss-magic-plugin hook <event>` if the pinned binary
# is there, and do nothing at all if it is not.
#
# Every EVENT hook in hooks.json is spawned through this script rather than
# naming the binary directly, and that indirection is the whole point of the
# file. The one entry that is NOT spawned through it is the SessionStart
# bootstrap, which must keep naming bootstrap.sh directly: this script does
# nothing when the binary is absent, and the bootstrap is what installs it, so
# routing the bootstrap through here would leave the plugin inert forever
# instead of for one session.
# ${CLAUDE_PLUGIN_DATA}/bin/ss-magic-plugin does not exist until the SessionStart
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

bin="$data/bin/ss-magic-plugin"
# `-f` as well as `-x`: `-x` alone is TRUE for a directory carrying the search
# bit, and `exec` on a directory does not fail quietly - bash prints a diagnostic
# and exits 126, which on PreToolUse would mean an error line per tool call from
# the one script whose whole job is to be silent.
# `lib/execguard.sh` owns the "will this actually run" test, shared with
# bin/ss-magic-plugin so the two cannot drift - which is exactly how the weaker
# `[ -x ]` survived in the wrapper after this script was hardened. It is sourced
# unconditionally, unlike lib/tmproot.sh below, because the check is needed on
# the common path where CLAUDE_PLUGIN_DATA is already set.
guard="$self_dir/../lib/execguard.sh"
[ -r "$guard" ] || give_up
# shellcheck source=../lib/execguard.sh
. "$guard" || give_up

ss_magic_is_loadable_executable "$bin" || give_up

# `execfail` catches the genuine exec failures the check above cannot see from
# the file alone - a shebang naming a missing interpreter, a noexec mount,
# EACCES. Without it bash exits 126/127 on those; with it control returns here
# and falls through to `exit 0`, so this file's invariant holds: every path
# exits 0.
#
# Bash still prints its own one-line diagnostic before returning, and that is
# accepted rather than suppressed. `exec ... 2>/dev/null` would apply to the
# SUCCESSFUL path too and swallow the binary's own HookContext::diagnostic
# output, which is a real channel this hook is supposed to carry.
#
# `exec` itself is deliberate and must stay: it makes the harness's child pid BE
# the binary, so a hook timeout kills the binary rather than killing this shell
# and orphaning it. That matters most on SessionEnd, which the CLI blocks on
# while a session exits.
#
# The argv is `hook <event>` and nothing else. There is no `plugin` token any
# more: the verb tree is its OWN binary now, so its argv IS the verb, where the
# old shared `ss-magic` binary needed a `plugin` prefix to reach the same code.
shopt -s execfail
exec "$bin" hook "$event"
exit 0
