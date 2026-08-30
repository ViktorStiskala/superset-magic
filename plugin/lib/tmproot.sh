# shellcheck shell=bash
#
# The R80 per-machine temporary root, in shell.
#
# This file is SOURCED by both `hooks/bootstrap.sh` and `bin/ss-magic-plugin`.
# It is deliberately one file rather than two copies: the two scripts must
# derive byte-identically the same directory as `src/plugin/tmproot.rs` does in
# Rust, and a lock or a handoff that lands in two different directories is a bug
# that only shows up under the concurrency the lock exists for. One copy cannot
# drift from itself.
#
# The contract, mirrored from `src/plugin/tmproot.rs`:
#
#   <base>/ss-magic-plugin/<identifier>/
#
#   base        /tmp first, then $TMPDIR (or /tmp again when $TMPDIR is unset,
#               matching Rust's std::env::temp_dir fallback).
#   identifier  the first 16 hex characters of sha256($HOME). $HOME is hashed
#               exactly as the environment hands it over: an unset $HOME hashes
#               as the empty string on both sides, so neither implementation has
#               to agree with the other about how to detect "unset".
#               printf %s, never echo - echo's trailing newline would change the
#               digest and silently split the two implementations apart.
#   validation  both the `ss-magic-plugin` level and the `<identifier>` level
#               (never /tmp or $TMPDIR themselves, which are system-owned) must
#               be a real directory, not a symlink, owned by this euid, at mode
#               exactly 0700. A guessable path is not evidence of ownership: on
#               a shared machine someone else may have created it first. Any
#               failure at either level makes the whole base unusable, so we
#               fall through to the other base and, if that also fails, refuse.
#
# Every function returns non-zero rather than printing a diagnostic. The callers
# own their own messaging, because both of them are bound by a one-line stderr
# budget (R72 for the bootstrap, R75's fail-open for the wrapper).

# The namespace directory under a temp base. Fixed cross-language contract text;
# `tmproot.rs::NAMESPACE_DIR` must say the same thing.
SS_MAGIC_NAMESPACE_DIR="ss-magic-plugin"

# The lock file the bootstrap serialises concurrent installs on, directly inside
# the resolved root. `tmproot.rs::INSTALL_LOCK_NAME`.
SS_MAGIC_INSTALL_LOCK_NAME="install.lock"

# The Bash-visible handoff the bootstrap writes and the wrapper reads: one line
# holding the resolved ${CLAUDE_PLUGIN_DATA}. It lives here because this root is
# the one location both processes can compute from $HOME alone, and $HOME is the
# only input a hook process and a Bash-tool process are guaranteed to share.
SS_MAGIC_DATA_ROOT_FILE="data-root"

# Read stdin, print its SHA-256 as lowercase hex. Non-zero when the machine has
# neither digest tool, in which case the caller must degrade rather than invent
# a different identifier (a wrong identifier is worse than no coordination: it
# looks like it works and guards nothing).
ss_magic_sha256_hex() {
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 | cut -d' ' -f1
    elif command -v sha256sum >/dev/null 2>&1; then
        sha256sum | cut -d' ' -f1
    else
        return 1
    fi
}

# The 16-hex-character per-machine identifier, or non-zero if it cannot be
# computed. Output is validated as hex before it is returned so a digest tool
# that prints something unexpected cannot produce a path segment nobody meant.
ss_magic_identifier() {
    local digest short
    digest=$(printf %s "${HOME-}" | ss_magic_sha256_hex) || return 1
    short=$(printf %s "$digest" | cut -c1-16)
    case "$short" in
        [0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]) ;;
        *) return 1 ;;
    esac
    printf %s "$short"
}

# True when $1 is a real directory (not a symlink), owned by uid $2, at mode
# exactly 0700.
#
# The symlink test comes first and is what makes the rest meaningful: `test -d`
# follows symlinks, so a symlink pointing at a directory someone else owns would
# otherwise pass. With the symlink rejected up front, `test -d` is exactly an
# lstat-and-is-a-directory check.
#
# GNU `stat -c` is tried before BSD `stat -f` on purpose. On macOS `-c` is an
# illegal option and fails cleanly, so the fallback is taken; the reverse order
# is not safe, because on GNU coreutils `-f` means --file-system and would
# SUCCEED while printing filesystem fields that have nothing to do with the mode
# and owner we asked for.
ss_magic_valid_component() {
    local path=$1 euid=$2 fields mode owner
    [ -L "$path" ] && return 1
    [ -d "$path" ] || return 1
    fields=$(stat -c '%a %u' "$path" 2>/dev/null) ||
        fields=$(stat -f '%Lp %u' "$path" 2>/dev/null) ||
        return 1
    mode=${fields%% *}
    owner=${fields##* }
    [ "$mode" = "700" ] || return 1
    [ "$owner" = "$euid" ] || return 1
    return 0
}

# Print the validated root path, or return non-zero when no base yields one.
#
# $1 is "create" (the bootstrap: make the directories if they are missing) or
# anything else, including omitted (the wrapper: read-only, never create). The
# wrapper never creates because it is a pure consumer of a handoff the bootstrap
# wrote; if the root is gone it has nothing to read anyway and fails open.
#
# `mkdir -m 0700` is used rather than a plain mkdir so the mode is set
# explicitly instead of being masked by whatever umask the session inherited -
# a umask that cleared an owner bit would otherwise create a directory that this
# file's own validation immediately rejects.
ss_magic_resolve_root() {
    local mode=${1-readonly} id euid base namespace root
    id=$(ss_magic_identifier) || return 1
    euid=$(id -u 2>/dev/null) || return 1
    case "$euid" in
        ''|*[!0-9]*) return 1 ;;
    esac

    for base in /tmp "${TMPDIR:-/tmp}"; do
        namespace="${base%/}/$SS_MAGIC_NAMESPACE_DIR"
        [ "$mode" = create ] && mkdir -m 0700 "$namespace" 2>/dev/null
        ss_magic_valid_component "$namespace" "$euid" || continue

        root="$namespace/$id"
        [ "$mode" = create ] && mkdir -m 0700 "$root" 2>/dev/null
        ss_magic_valid_component "$root" "$euid" || continue

        printf %s "$root"
        return 0
    done
    return 1
}
