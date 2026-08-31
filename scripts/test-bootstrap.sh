#!/usr/bin/env bash
#
# Failure-path tests for plugin/hooks/bootstrap.sh and plugin/bin/ss-magic-plugin.
#
# This script runs on every fresh session on every machine that enables the
# plugin, which makes its failure behaviour, not its success behaviour, the part
# worth testing: a bug here is a broken session start for every user at once.
# So the scenarios below are mostly things going wrong - offline, a corrupted
# download, a hostile version pin, an unwritable data directory, a platform with
# no published build - and each one asserts the same three properties: exit 0,
# nothing on stdout, and any existing binary left exactly as it was.
#
# Nothing here touches the network. A `curl` shim earlier on PATH serves a
# locally built fake release (a tarball containing a shell script that answers
# `--version`), and records every URL it is asked for, so "we never composed a
# URL from a hostile pin" is an assertion rather than a hope.
#
# Written for bash 3.2, which is what macOS ships: no associative arrays, no
# `${var^^}`, no `mapfile`.
#
# Usage: bash scripts/test-bootstrap.sh [-v]

set -u

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
PLUGIN_SRC="$REPO_ROOT/plugin"
VERBOSE=${1:-}

passed=0
failed=0
current_case="(none)"

pass() { passed=$((passed + 1)); [ "$VERBOSE" = "-v" ] && printf '  ok   %s\n' "$1"; return 0; }
fail() { failed=$((failed + 1)); printf '  FAIL %s: %s\n' "$current_case" "$1" >&2; return 0; }

assert_eq() { # expected actual label
    if [ "$1" = "$2" ]; then pass "$3"; else fail "$3 (expected [$1], got [$2])"; fi
}
assert_file_absent() {
    if [ -e "$1" ]; then fail "$2 (still exists: $1)"; else pass "$2"; fi
}
assert_file_present() {
    if [ -e "$1" ]; then pass "$2"; else fail "$2 (missing: $1)"; fi
}
assert_contains() { # haystack-file needle label
    if grep -q -- "$2" "$1" 2>/dev/null; then pass "$3"; else fail "$3 (no [$2] in $1)"; fi
}

# --------------------------------------------------------------------------
# The platform triple the bootstrap will resolve here, derived the same way it
# derives it. The fake release is built under exactly this name, so the test
# exercises the real triple resolution rather than stubbing it out.
# --------------------------------------------------------------------------
case "$(uname -s)" in
    Darwin) HOST_OS=apple-darwin ;;
    Linux) HOST_OS=unknown-linux-gnu ;;
    *) printf 'test-bootstrap: unsupported host %s; nothing to test against.\n' "$(uname -s)" >&2
       exit 0 ;;
esac
case "$(uname -m)" in
    arm64|aarch64) HOST_ARCH=aarch64 ;;
    x86_64|amd64) HOST_ARCH=x86_64 ;;
    *) printf 'test-bootstrap: unsupported host arch %s.\n' "$(uname -m)" >&2; exit 0 ;;
esac
TRIPLE="$HOST_ARCH-$HOST_OS"

sha256_of() {
    if command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | cut -d' ' -f1
    else sha256sum "$1" | cut -d' ' -f1; fi
}

# --------------------------------------------------------------------------
# Sandbox construction
# --------------------------------------------------------------------------

SANDBOX_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/ss-magic-bootstrap-tests.XXXXXX") || exit 1
CREATED_TMPROOTS=""

cleanup_all() {
    # Remove the R80 roots the sandboxed $HOME values caused to be created,
    # under whichever base actually got used.
    for id in $CREATED_TMPROOTS; do
        rm -rf "/tmp/ss-magic-plugin/$id" "${TMPDIR:-/tmp}/ss-magic-plugin/$id" 2>/dev/null
    done
    chmod -R u+rwX "$SANDBOX_ROOT" 2>/dev/null
    rm -rf "$SANDBOX_ROOT" 2>/dev/null
}
trap cleanup_all EXIT

sb=""          # current sandbox
SB_ID=""       # its R80 identifier

# new_sandbox <name> <pin>
new_sandbox() {
    sb="$SANDBOX_ROOT/$1"
    mkdir -p "$sb/home" "$sb/data" "$sb/shim" "$sb/release" "$sb/tmp"
    cp -R "$PLUGIN_SRC" "$sb/plugin"
    printf '%s\n' "$2" >"$sb/plugin/ss-magic.version"
    : >"$sb/curl.log"

    SB_ID=$(printf %s "$sb/home" | sha256_stdin | cut -c1-16)
    CREATED_TMPROOTS="$CREATED_TMPROOTS $SB_ID"

    cat >"$sb/shim/curl" <<'SHIM'
#!/usr/bin/env bash
# Stand-in for curl. Serves $FAKE_RELEASE/<version-dir>/<basename> and logs the
# URL, so a test can assert that a URL was never composed at all.
url=""; out=""
while [ $# -gt 0 ]; do
  case "$1" in
    -o) out=$2; shift 2 ;;
    http://*|https://*) url=$1; shift ;;
    *) shift ;;
  esac
done
printf '%s\n' "$url" >>"$FAKE_CURL_LOG"
case "${FAKE_CURL_MODE:-serve}" in
  offline) exit 6 ;;
  slow) sleep "${FAKE_CURL_DELAY:-1}" ;;
esac
name=${url##*/}
rest=${url%/*}; ver=${rest##*/}
src="$FAKE_RELEASE/$ver/$name"
[ -f "$src" ] || exit 22
cp "$src" "$out" 2>/dev/null || exit 23
exit 0
SHIM
    chmod 755 "$sb/shim/curl"
    # wget must not become a silent second path when the curl shim reports a
    # failure, so shadow it too.
    cat >"$sb/shim/wget" <<'SHIM'
#!/usr/bin/env bash
exit 4
SHIM
    chmod 755 "$sb/shim/wget"
}

sha256_stdin() {
    if command -v shasum >/dev/null 2>&1; then shasum -a 256 | cut -d' ' -f1
    else sha256sum | cut -d' ' -f1; fi
}

# publish_release <version> [--corrupt-digest|--empty-archive]
publish_release() {
    local ver=$1 variant=${2:-} dir build
    dir="$sb/release/v$ver"
    build="$sb/build-$ver"
    rm -rf "$build"; mkdir -p "$dir" "$build/ss-magic-$TRIPLE"
    if [ "$variant" != "--empty-archive" ]; then
        cat >"$build/ss-magic-$TRIPLE/ss-magic" <<FAKEBIN
#!/usr/bin/env bash
case "\${1:-}" in
  --version|-V) echo "ss-magic $ver"; exit 0 ;;
esac
printf '%s\n' "\$*" >>"\${SS_MAGIC_FAKE_LOG:-/dev/null}"
exit 0
FAKEBIN
        chmod 755 "$build/ss-magic-$TRIPLE/ss-magic"
    else
        # A well-formed archive that simply does not contain the binary: the
        # shape a layout change or a truncated build would take.
        printf 'not the binary\n' >"$build/ss-magic-$TRIPLE/README"
    fi
    tar -czf "$dir/ss-magic-$TRIPLE.tar.gz" -C "$build" "ss-magic-$TRIPLE"
    if [ "$variant" = "--corrupt-digest" ]; then
        printf '%s  %s\n' \
            "0000000000000000000000000000000000000000000000000000000000000000" \
            "ss-magic-$TRIPLE.tar.gz" >"$dir/ss-magic-$TRIPLE.tar.gz.sha256"
    else
        printf '%s  %s\n' "$(sha256_of "$dir/ss-magic-$TRIPLE.tar.gz")" \
            "ss-magic-$TRIPLE.tar.gz" >"$dir/ss-magic-$TRIPLE.tar.gz.sha256"
    fi
}

# run_bootstrap -> writes $sb/out, $sb/err; sets RC
RC=0
run_bootstrap() {
    : >"$sb/out"; : >"$sb/err"
    env -i \
        PATH="$sb/shim:/usr/bin:/bin:/usr/sbin:/sbin" \
        HOME="$sb/home" \
        TMPDIR="$sb/tmp" \
        CLAUDE_PLUGIN_ROOT="$sb/plugin" \
        CLAUDE_PLUGIN_DATA="$sb/data" \
        FAKE_RELEASE="$sb/release" \
        FAKE_CURL_LOG="$sb/curl.log" \
        FAKE_CURL_MODE="${FAKE_CURL_MODE:-serve}" \
        FAKE_CURL_DELAY="${FAKE_CURL_DELAY:-0}" \
        bash "$sb/plugin/hooks/bootstrap.sh" >"$sb/out" 2>"$sb/err"
    RC=$?
}

# run_wrapper <args...> ; honours $WRAPPER_DATA ("" means unset, forcing the handoff)
run_wrapper() {
    : >"$sb/wout"; : >"$sb/werr"
    if [ -n "${WRAPPER_DATA:-}" ]; then
        env -i PATH="/usr/bin:/bin" HOME="$sb/home" TMPDIR="$sb/tmp" \
            CLAUDE_PLUGIN_DATA="$WRAPPER_DATA" \
            SS_MAGIC_FAKE_LOG="$sb/fakebin.log" \
            bash "$sb/plugin/bin/ss-magic-plugin" "$@" >"$sb/wout" 2>"$sb/werr"
    else
        env -i PATH="/usr/bin:/bin" HOME="$sb/home" TMPDIR="$sb/tmp" \
            SS_MAGIC_FAKE_LOG="$sb/fakebin.log" \
            bash "$sb/plugin/bin/ss-magic-plugin" "$@" >"$sb/wout" 2>"$sb/werr"
    fi
    RC=$?
}

stderr_lines() { wc -l <"$sb/err" | tr -d ' '; }
stdout_bytes() { wc -c <"$sb/out" | tr -d ' '; }
archive_fetches() { grep -c 'tar\.gz$' "$sb/curl.log" 2>/dev/null | tr -d ' '; }

# Assert the three invariants every path shares. $1 is a label prefix.
assert_never_fails_session() {
    assert_eq 0 "$RC" "$1: exit status is 0"
    assert_eq 0 "$(stdout_bytes)" "$1: stdout is empty"
}

# ==========================================================================
# AE57 - the pin already matches: silent, and nothing is downloaded
# ==========================================================================
current_case="AE57 silent no-op"
new_sandbox ae57 0.10.0
publish_release 0.10.0
run_bootstrap                                  # first run installs
assert_eq 0 "$RC" "AE57: install exits 0"
assert_file_present "$sb/data/bin/ss-magic" "AE57: binary installed"
before=$(sha256_of "$sb/data/bin/ss-magic")
fetches_before=$(archive_fetches)

run_bootstrap                                  # second run must be a no-op
assert_never_fails_session "AE57"
assert_eq 0 "$(stderr_lines)" "AE57: stderr is empty on the no-op path"
assert_eq "$fetches_before" "$(archive_fetches)" "AE57: no further download"
assert_eq "$before" "$(sha256_of "$sb/data/bin/ss-magic")" "AE57: binary untouched"

# ==========================================================================
# AE58 - no network at all
# ==========================================================================
current_case="AE58 offline"
new_sandbox ae58 0.10.0
publish_release 0.10.0
FAKE_CURL_MODE=offline run_bootstrap
assert_never_fails_session "AE58"
assert_eq 1 "$(stderr_lines)" "AE58: exactly one stderr line"
assert_file_absent "$sb/data/bin/ss-magic" "AE58: nothing installed"
assert_file_absent "$sb/data/.ss-magic-installed" "AE58: no success marker"
assert_eq "" "$(ls -d "$sb"/data/.ss-magic-stage.* 2>/dev/null)" "AE58: no staging left behind"

# An existing older install must survive an offline session untouched.
current_case="AE58 offline with an older install present"
new_sandbox ae58b 0.10.0
publish_release 0.10.0
run_bootstrap
old_digest=$(sha256_of "$sb/data/bin/ss-magic")
printf '%s\n' "0.11.0" >"$sb/plugin/ss-magic.version"
FAKE_CURL_MODE=offline run_bootstrap
assert_never_fails_session "AE58b"
assert_eq 1 "$(stderr_lines)" "AE58b: exactly one stderr line"
assert_eq "$old_digest" "$(sha256_of "$sb/data/bin/ss-magic")" "AE58b: old binary untouched"
assert_file_absent "$sb/data/.ss-magic-installed" "AE58b: marker cleared so the next session retries"

# ==========================================================================
# AE59 - the archive does not match its published digest
# ==========================================================================
current_case="AE59 checksum mismatch"
new_sandbox ae59 0.10.0
publish_release 0.10.0
run_bootstrap
good_digest=$(sha256_of "$sb/data/bin/ss-magic")
printf '%s\n' "0.11.0" >"$sb/plugin/ss-magic.version"
publish_release 0.11.0 --corrupt-digest
run_bootstrap
assert_never_fails_session "AE59"
assert_eq 1 "$(stderr_lines)" "AE59: exactly one stderr line"
assert_contains "$sb/err" "checksum mismatch" "AE59: says what went wrong"
assert_eq "$good_digest" "$(sha256_of "$sb/data/bin/ss-magic")" "AE59: existing binary untouched"
assert_eq "0.10.0" "$("$sb/data/bin/ss-magic" --version | awk '{print $NF}')" "AE59: still the old version"
assert_file_absent "$sb/data/.ss-magic-installed" "AE59: marker cleared"
assert_eq "" "$(ls -d "$sb"/data/.ss-magic-stage.* 2>/dev/null)" "AE59: no staging left behind"

# ==========================================================================
# AE60 - a hostile pin never reaches a URL, a shell, or the filesystem
# ==========================================================================
current_case="AE60 hostile pin"
# Each of these is a shape that must die in validation, before the pin is
# compared, interpolated, or allowed anywhere near a command line: shell
# metacharacters, command substitution, a path traversal, a URL, a leading `v`,
# the wrong number of fields, a pre-release suffix, an absolute path.
hostile_pins=(
    '1.2.3;touch CANARY'
    '1.2.3 && touch CANARY'
    '$(touch CANARY)'
    '`touch CANARY`'
    '../../../../etc/passwd'
    'https://evil.example/ss-magic'
    'v1.2.3'
    '1.2'
    '1.2.3.4'
    '0.10.0-beta.1'
    '-1.2.3'
    '/absolute/1.2.3'
    ''
)
i=0
for pin in "${hostile_pins[@]}"; do
    i=$((i + 1))
    new_sandbox "ae60-$i" "$pin"
    publish_release 0.10.0
    run_bootstrap
    assert_never_fails_session "AE60 [$pin]"
    assert_eq 1 "$(stderr_lines)" "AE60 [$pin]: exactly one stderr line"
    assert_eq 0 "$(wc -c <"$sb/curl.log" | tr -d ' ')" "AE60 [$pin]: no URL was ever composed"
    assert_file_absent "$sb/data/bin/ss-magic" "AE60 [$pin]: nothing installed"
    # CANARY would appear wherever the substitution ran: the sandbox, the
    # plugin copy, or the directory this harness was started from.
    if [ -e "$sb/CANARY" ] || [ -e "$sb/plugin/CANARY" ] || [ -e "./CANARY" ] ||
       [ -e "$REPO_ROOT/CANARY" ]; then
        fail "AE60 [$pin]: the pin was executed - CANARY exists"
        rm -f "$sb/CANARY" "$sb/plugin/CANARY" ./CANARY "$REPO_ROOT/CANARY" 2>/dev/null
    else
        pass "AE60 [$pin]: the pin was not executed"
    fi
done

# ==========================================================================
# AE61 - concurrent sessions produce one install and no failures
# ==========================================================================
current_case="AE61 concurrent sessions"
new_sandbox ae61 0.10.0
publish_release 0.10.0
FAKE_CURL_MODE=slow
FAKE_CURL_DELAY=1
pids=""
for i in 1 2 3 4; do
    ( : >"$sb/out.$i"; : >"$sb/err.$i"
      env -i PATH="$sb/shim:/usr/bin:/bin:/usr/sbin:/sbin" HOME="$sb/home" TMPDIR="$sb/tmp" \
          CLAUDE_PLUGIN_ROOT="$sb/plugin" CLAUDE_PLUGIN_DATA="$sb/data" \
          FAKE_RELEASE="$sb/release" FAKE_CURL_LOG="$sb/curl.log" \
          FAKE_CURL_MODE=slow FAKE_CURL_DELAY=1 \
          bash "$sb/plugin/hooks/bootstrap.sh" >"$sb/out.$i" 2>"$sb/err.$i"
      printf '%s\n' "$?" >"$sb/rc.$i" ) &
    pids="$pids $!"
done
for p in $pids; do wait "$p"; done
FAKE_CURL_MODE=serve
FAKE_CURL_DELAY=0
concurrent_ok=yes
for i in 1 2 3 4; do
    [ "$(cat "$sb/rc.$i")" = "0" ] || concurrent_ok=no
    [ -s "$sb/out.$i" ] && concurrent_ok=no
done
assert_eq yes "$concurrent_ok" "AE61: every concurrent session exits 0 with empty stdout"
assert_file_present "$sb/data/bin/ss-magic" "AE61: the binary is installed"
assert_eq "0.10.0" "$("$sb/data/bin/ss-magic" --version | awk '{print $NF}')" "AE61: correct version"
assert_eq "" "$(ls -d "$sb"/data/.ss-magic-stage.* 2>/dev/null)" "AE61: no staging left behind"
if command -v flock >/dev/null 2>&1 || command -v perl >/dev/null 2>&1; then
    assert_eq 1 "$(archive_fetches)" "AE61: the lock collapsed four sessions into one download"
else
    printf '  skip AE61 download-count assertion: no flock(1) and no perl on this machine\n'
fi

# ==========================================================================
# AE62 - a partial or impossible install leaves nothing half-done
# ==========================================================================
current_case="AE62 archive without the binary"
new_sandbox ae62 0.10.0
publish_release 0.10.0
run_bootstrap
kept=$(sha256_of "$sb/data/bin/ss-magic")
printf '%s\n' "0.11.0" >"$sb/plugin/ss-magic.version"
publish_release 0.11.0 --empty-archive
run_bootstrap
assert_never_fails_session "AE62"
assert_eq 1 "$(stderr_lines)" "AE62: exactly one stderr line"
assert_eq "$kept" "$(sha256_of "$sb/data/bin/ss-magic")" "AE62: existing binary untouched"
assert_file_absent "$sb/data/.ss-magic-installed" "AE62: marker cleared"
assert_eq "" "$(ls -d "$sb"/data/.ss-magic-stage.* 2>/dev/null)" "AE62: staging cleaned up"

current_case="AE62 unwritable data directory"
if [ "$(id -u)" = "0" ]; then
    printf '  skip AE62 unwritable-directory case: running as root\n'
else
    new_sandbox ae62b 0.10.0
    publish_release 0.10.0
    chmod 0500 "$sb/data"
    run_bootstrap
    chmod 0700 "$sb/data"
    assert_never_fails_session "AE62b"
    assert_eq 1 "$(stderr_lines)" "AE62b: exactly one stderr line"
    assert_file_absent "$sb/data/bin/ss-magic" "AE62b: nothing installed"
fi

# ==========================================================================
# AE63 - advancing the pin replaces the binary
# ==========================================================================
current_case="AE63 pin advance"
new_sandbox ae63 0.10.0
publish_release 0.10.0
publish_release 0.11.0
run_bootstrap
assert_eq "0.10.0" "$("$sb/data/bin/ss-magic" --version | awk '{print $NF}')" "AE63: starts at the old pin"
printf '%s\n' "0.11.0" >"$sb/plugin/ss-magic.version"
run_bootstrap
assert_never_fails_session "AE63"
assert_eq "0.11.0" "$("$sb/data/bin/ss-magic" --version | awk '{print $NF}')" "AE63: advanced to the new pin"
assert_eq "0.11.0" "$(cat "$sb/data/.ss-magic-installed")" "AE63: marker records the new pin"
run_bootstrap
assert_eq 0 "$(stderr_lines)" "AE63: the run after the advance is silent"

# ==========================================================================
# AE64 - hooks.json runs the bootstrap on `startup` only
# ==========================================================================
current_case="AE64 hook wiring"
if command -v python3 >/dev/null 2>&1; then
    if python3 - "$PLUGIN_SRC/hooks/hooks.json" <<'PY'
import json, sys
spec = json.load(open(sys.argv[1]))
groups = spec["hooks"]["SessionStart"]
boot = [g for g in groups
        if any("bootstrap.sh" in a for h in g["hooks"] for a in h.get("args", []))]
assert len(boot) == 1, f"expected exactly one bootstrap group, found {len(boot)}"
g = boot[0]
assert g.get("matcher") == "startup", (
    f"the bootstrap group matcher is {g.get('matcher')!r}; it must be exactly "
    "'startup' or it re-runs on every resume, clear, compaction and fork")
h = g["hooks"][0]
assert h["type"] == "command" and h["command"] == "bash", "not exec form"
assert h["args"][0].startswith("${CLAUDE_PLUGIN_ROOT}/"), "args[0] is not plugin-root relative"
assert isinstance(h.get("timeout"), int) and 0 < h["timeout"] < 600, "no explicit sub-default timeout"
other = [g for g in groups if g is not boot[0]]
assert other, "the ss-magic session-start handler group is missing"
for g in other:
    m = g.get("matcher")
    assert m is None or all(s in m for s in ("resume", "clear", "compact", "fork")), (
        "the ss-magic session-start handler must still fire on compact")
print("ok")
PY
    then pass "AE64: bootstrap runs on startup only; the handler still covers compact"
    else fail "AE64: hooks.json wiring is wrong (see above)"
    fi
else
    printf '  skip AE64: python3 is not available\n'
fi

# ==========================================================================
# AE65 - the wrapper
# ==========================================================================
current_case="AE65 wrapper resolves through the handoff"
new_sandbox ae65 0.10.0
publish_release 0.10.0
run_bootstrap
: >"$sb/fakebin.log"
WRAPPER_DATA="" run_wrapper checklist list
assert_eq 0 "$RC" "AE65: wrapper exits 0"
assert_eq "plugin checklist list" "$(cat "$sb/fakebin.log")" "AE65: injects the plugin verb, no CLAUDE_PLUGIN_DATA in scope"

current_case="AE65 wrapper with the handoff removed"
handoff_root="/tmp/ss-magic-plugin/$SB_ID"
[ -d "$handoff_root" ] || handoff_root="$sb/tmp/ss-magic-plugin/$SB_ID"
assert_file_present "$handoff_root/data-root" "AE65: the bootstrap published the data root"
assert_eq "$sb/data" "$(cat "$handoff_root/data-root")" "AE65: the handoff names the data directory"
rm -f "$handoff_root/data-root"
: >"$sb/fakebin.log"
WRAPPER_DATA="" run_wrapper checklist list
assert_eq 0 "$RC" "AE65: a missing handoff fails open with exit 0"
assert_eq 1 "$(wc -l <"$sb/werr" | tr -d ' ')" "AE65: one line of explanation on stderr"
assert_eq 0 "$(wc -c <"$sb/wout" | tr -d ' ')" "AE65: nothing on stdout"
assert_eq "" "$(cat "$sb/fakebin.log")" "AE65: the binary was not invoked"

current_case="AE65 wrapper pointed at a directory with no binary"
: >"$sb/fakebin.log"
WRAPPER_DATA="$sb/nowhere" run_wrapper checklist list
assert_eq 0 "$RC" "AE65: an empty data directory fails open with exit 0"
assert_eq 1 "$(wc -l <"$sb/werr" | tr -d ' ')" "AE65: one line of explanation on stderr"
assert_eq "" "$(cat "$sb/fakebin.log")" "AE65: the binary was not invoked"

# ==========================================================================
# AE66 - a platform with no published release target
# ==========================================================================
current_case="AE66 unsupported platform"
new_sandbox ae66 0.10.0
publish_release 0.10.0
cat >"$sb/shim/uname" <<'SHIM'
#!/usr/bin/env bash
case "${1:-}" in
  -s) echo "MINGW64_NT-10.0" ;;
  -m) echo "x86_64" ;;
  *) echo "MINGW64_NT-10.0" ;;
esac
SHIM
chmod 755 "$sb/shim/uname"
run_bootstrap
assert_never_fails_session "AE66"
assert_eq 1 "$(stderr_lines)" "AE66: reports the reason once"
assert_contains "$sb/err" "no published release binary" "AE66: says why"
assert_eq 0 "$(wc -c <"$sb/curl.log" | tr -d ' ')" "AE66: no download attempted"
run_bootstrap
assert_never_fails_session "AE66 second run"
assert_eq 0 "$(stderr_lines)" "AE66: silent on every later session"

# ==========================================================================
# AE67 - the one-time disclosure
# ==========================================================================
current_case="AE67 one-time disclosure"
new_sandbox ae67 0.10.0
publish_release 0.10.0
publish_release 0.11.0
run_bootstrap
assert_never_fails_session "AE67"
assert_contains "$sb/err" "installed ss-magic 0.10.0" "AE67: names the binary and version"
assert_contains "$sb/err" "releases/tag/v0.10.0" "AE67: names the release it came from"
assert_contains "$sb/err" "SessionStart" "AE67: names the hooks it registers"
printf '%s\n' "0.11.0" >"$sb/plugin/ss-magic.version"
run_bootstrap
assert_never_fails_session "AE67 second install"
assert_eq 0 "$(stderr_lines)" "AE67: a later successful install is silent"

# ==========================================================================
printf '\n%s: %d passed, %d failed\n' "$(basename "$0")" "$passed" "$failed"
[ "$failed" -eq 0 ] || exit 1
exit 0
