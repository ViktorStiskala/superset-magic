# shellcheck shell=bash
#
# The one answer to "will the kernel actually run this file", shared by every
# script that execs the pinned binary.
#
# It is one file rather than a copy per caller for the reason the copies would
# fail: `hooks/run-hook.sh` and `bin/ss-magic-plugin` resolve the same path and
# exec the same binary, and when this check lived in only one of them the other
# kept a weaker test for months. A guard duplicated across siblings drifts; a
# guard defined once cannot.
#
# `hooks/bootstrap.sh` deliberately does NOT use this. It has something stronger
# available at install time: it runs the staged binary and refuses the install
# unless it reports the pinned version. That is a real execve plus a behavioural
# check, so a magic-number test would add nothing.
#
# What the check has to cover, and why the obvious `[ -x ]` does not:
#
#   * `-x` is TRUE for a directory carrying the search bit. `exec` on a directory
#     does not fail quietly - bash prints a diagnostic and exits 126.
#   * `-f` and `-x` both ask about the FILE. Neither can see execve failing on
#     the way into it, and the ENOEXEC case is the dangerous one: when the file
#     is readable but is not a loadable executable - a truncated download, a
#     foreign architecture, on-disk corruption - bash falls back to POSIX
#     behaviour and REINTERPRETS it as a shell script. That is not an exec
#     failure, so `shopt -s execfail` never fires, and the process exits with
#     whatever those bytes happen to parse to. Measured over 30 corrupted
#     binaries on bash 3.2: exit 2 roughly half the time. Exit 2 from a
#     PreToolUse hook means BLOCK the tool call, so a binary damaged after
#     install would silently block nearly every tool call in a session.
#
# Callers still want `shopt -s execfail` after this: it catches the genuine exec
# failures this cannot see from the file alone - a shebang naming an interpreter
# that is missing, a noexec mount, EACCES.

# True when $1 is a regular file, executable, and starts with a magic number the
# kernel (or bash, for `#!`) will actually load.
#
# The list is the formats this binary ships as, plus `#!` so an interpreted
# wrapper still works - the test suite's own stand-in binary is one, so a guard
# that rejected `#!` would make every test that depends on it vacuous. An
# unrecognised format returns non-zero, which every caller turns into "do
# nothing": for the FORMAT question, failing closed is right, since the
# alternative is running something we cannot identify. The TOOL question is the
# opposite and is answered inside the function - see the note on `od` below.
ss_magic_is_loadable_executable() {
    local bin=$1
    [ -f "$bin" ] || return 1
    [ -x "$bin" ] || return 1
    # If the machine has no `od`, the magic check cannot run - and it must then
    # fail OPEN, not closed. This is a robustness guard, not a security boundary:
    # refusing here would silently disable the whole plugin on a machine that is
    # merely missing a utility, which is a far worse outcome than the rare
    # damaged-binary case the check exists for. `-f`, `-x` and the caller's
    # `shopt -s execfail` all still apply.
    command -v od >/dev/null 2>&1 || return 0
    command -v tr >/dev/null 2>&1 || return 0
    case "$(od -An -N4 -tx1 <"$bin" 2>/dev/null | tr -d ' ')" in
        7f454c46) ;;                              # ELF
        cffaedfe|cefaedfe|feedface|feedfacf) ;;   # Mach-O 32/64, either endianness
        cafebabe|bebafeca) ;;                     # Mach-O universal ("fat")
        2321*) ;;                                 # "#!" - an interpreted wrapper
        *) return 1 ;;
    esac
    return 0
}
