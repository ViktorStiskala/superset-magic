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
# URL from a hostile pin" is an assertion rather than a hope - and so is the
# positive half, that the URL composed is exactly
# .../download/ss-magic-plugin-v<pin>/ss-magic-plugin-<triple>.tar.gz.
#
# The plugin binary is `ss-magic-plugin`, released on its own
# `ss-magic-plugin-vX.Y.Z` tag line, pinned by `plugin/ss-magic-plugin.version`,
# and installed to `${CLAUDE_PLUGIN_DATA}/bin/ss-magic-plugin`. Its argv IS the
# verb - the shim execs `hook <event>` and the wrapper forwards `"$@"` - so
# there is no `plugin` token anywhere, and several cases below assert its
# absence rather than only asserting the new spelling.
#
# Written for bash 3.2, which is what macOS ships: no associative arrays, no
# `${var^^}`, no `mapfile`.
#
# Usage: bash scripts/test-bootstrap.sh [-v]

set -u

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
PLUGIN_SRC="$REPO_ROOT/plugin"
VERBOSE=${1:-}

# pass/fail, the counters, and every assert_* helper.
. "$REPO_ROOT/scripts/lib/test-harness.sh"

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
    printf '%s\n' "$2" >"$sb/plugin/ss-magic-plugin.version"
    : >"$sb/curl.log"
    # Created up front so every case can `cat` it, including one that runs
    # before any binary has ever been invoked.
    : >"$sb/fakebin.log"

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

# publish_release <version> [--corrupt-digest|--empty-archive|--wrong-version]
#
# Builds the fake release for the plugin's OWN release line: the tag directory is
# `ss-magic-plugin-v<version>` and the asset is `ss-magic-plugin-<triple>.tar.gz`,
# holding `ss-magic-plugin-<triple>/ss-magic-plugin`. The curl shim serves
# $FAKE_RELEASE/<tag dir>/<basename>, so the directory name here is what proves
# the bootstrap composed the tag the way the release publishes it.
#
# The fake binary answers `--version` with `ss-magic-plugin <version>` on one
# line and exits 0 before anything else, exactly as the real binary must - the
# bootstrap gates every install on the last field of that line equalling the pin.
# Any other argv is appended to $SS_MAGIC_FAKE_LOG, which is how a test observes
# what the binary was actually invoked with (`seed-config`, `hook <event>`, a
# skill's verb) without the `--version` probes polluting the record.
#
# --wrong-version is the companion to that gate: a well-formed, correctly
# checksummed archive whose binary simply reports a DIFFERENT version, which is
# what a wrong-architecture or mis-tagged build looks like from here.
publish_release() {
    local ver=$1 variant=${2:-} dir build reported
    dir="$sb/release/ss-magic-plugin-v$ver"
    build="$sb/build-$ver"
    reported=$ver
    [ "$variant" = "--wrong-version" ] && reported="9.9.9"
    rm -rf "$build"; mkdir -p "$dir" "$build/ss-magic-plugin-$TRIPLE"
    if [ "$variant" != "--empty-archive" ]; then
        cat >"$build/ss-magic-plugin-$TRIPLE/ss-magic-plugin" <<FAKEBIN
#!/usr/bin/env bash
case "\${1:-}" in
  --version|-V) echo "ss-magic-plugin $reported"; exit 0 ;;
esac
printf '%s\n' "\$*" >>"\${SS_MAGIC_FAKE_LOG:-/dev/null}"
exit 0
FAKEBIN
        chmod 755 "$build/ss-magic-plugin-$TRIPLE/ss-magic-plugin"
    else
        # A well-formed archive that simply does not contain the binary: the
        # shape a layout change or a truncated build would take.
        printf 'not the binary\n' >"$build/ss-magic-plugin-$TRIPLE/README"
    fi
    tar -czf "$dir/ss-magic-plugin-$TRIPLE.tar.gz" -C "$build" "ss-magic-plugin-$TRIPLE"
    if [ "$variant" = "--corrupt-digest" ]; then
        printf '%s  %s\n' \
            "0000000000000000000000000000000000000000000000000000000000000000" \
            "ss-magic-plugin-$TRIPLE.tar.gz" >"$dir/ss-magic-plugin-$TRIPLE.tar.gz.sha256"
    else
        printf '%s  %s\n' "$(sha256_of "$dir/ss-magic-plugin-$TRIPLE.tar.gz")" \
            "ss-magic-plugin-$TRIPLE.tar.gz" >"$dir/ss-magic-plugin-$TRIPLE.tar.gz.sha256"
    fi
}

# run_bootstrap -> writes $sb/out, $sb/err; sets RC
#
# SS_MAGIC_FAKE_LOG is passed through so the INSTALLED binary records what the
# bootstrap invoked it with after the install lands - which is how the R3a
# seed-config cases below count the call. The `--version` probes never reach the
# log (the fake binary answers and exits before writing), so a log holding
# exactly `seed-config` means exactly one post-install invocation happened.
RC=0
# run_bootstrap ; honours $RUN_CWD (the working directory the bootstrap runs
# in; default: the current one). A parameter rather than `( cd … && run_bootstrap )`
# at the call site, because RC is set inside this function and a subshell
# would take the assignment with it – the assertion after such a call would
# then read the PREVIOUS run's status and pass vacuously.
run_bootstrap() {
    : >"$sb/out"; : >"$sb/err"
    ( cd "${RUN_CWD:-.}" && env -i \
        PATH="$sb/shim:/usr/bin:/bin:/usr/sbin:/sbin" \
        HOME="$sb/home" \
        TMPDIR="$sb/tmp" \
        CLAUDE_PLUGIN_ROOT="$sb/plugin" \
        CLAUDE_PLUGIN_DATA="$sb/data" \
        FAKE_RELEASE="$sb/release" \
        FAKE_CURL_LOG="$sb/curl.log" \
        FAKE_CURL_MODE="${FAKE_CURL_MODE:-serve}" \
        FAKE_CURL_DELAY="${FAKE_CURL_DELAY:-0}" \
        SS_MAGIC_FAKE_LOG="$sb/fakebin.log" \
        bash "$sb/plugin/hooks/bootstrap.sh" >"$sb/out" 2>"$sb/err" )
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

# The hook shim, run the way the harness runs it: `bash <root>/hooks/run-hook.sh
# <event>`. SHIM_DATA empty means CLAUDE_PLUGIN_DATA is absent from the
# environment, which is the reload/first-session case the shim exists for.
run_shim() { # event
    : >"$sb/sout"; : >"$sb/serr"
    if [ -n "${SHIM_DATA:-}" ]; then
        env -i PATH="/usr/bin:/bin" HOME="$sb/home" TMPDIR="$sb/tmp" \
            CLAUDE_PLUGIN_DATA="$SHIM_DATA" \
            SS_MAGIC_FAKE_LOG="$sb/fakebin.log" \
            bash "$sb/plugin/hooks/run-hook.sh" "$@" >"$sb/sout" 2>"$sb/serr"
    else
        env -i PATH="/usr/bin:/bin" HOME="$sb/home" TMPDIR="$sb/tmp" \
            SS_MAGIC_FAKE_LOG="$sb/fakebin.log" \
            bash "$sb/plugin/hooks/run-hook.sh" "$@" >"$sb/sout" 2>"$sb/serr"
    fi
    RC=$?
}

# The shim's whole contract in one place: it never breaks a session and it never
# says anything. Stdout silence is load-bearing (a SessionStart hook's stdout
# enters the model's context); stderr silence is what separates it from the
# wrapper, since PreToolUse fires on nearly every tool call.
assert_shim_inert() { # label
    assert_eq 0 "$RC" "$1: exit status is 0"
    assert_eq 0 "$(wc -c <"$sb/sout" | tr -d ' ')" "$1: stdout is empty"
    assert_eq 0 "$(wc -c <"$sb/serr" | tr -d ' ')" "$1: stderr is empty"
    assert_eq "" "$(cat "$sb/fakebin.log")" "$1: the binary was not invoked"
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
assert_file_present "$sb/data/bin/ss-magic-plugin" "AE57: binary installed"
before=$(sha256_of "$sb/data/bin/ss-magic-plugin")
fetches_before=$(archive_fetches)

# The URL the install actually composed, asserted as a fixed string. The plugin
# rides its own release line, so both halves carry the `ss-magic-plugin` prefix:
# the tag is `ss-magic-plugin-v<pin>` (not the CLI's bare `v<pin>`) and the asset
# is `ss-magic-plugin-<triple>.tar.gz`. Getting either half wrong is a 404 that
# looks exactly like "offline" from the user's side, so it is worth pinning here
# rather than discovering on a release day.
assert_contains_fixed "$sb/curl.log" \
    "/download/ss-magic-plugin-v0.10.0/ss-magic-plugin-$TRIPLE.tar.gz" \
    "AE57: composed the plugin line's tag and asset name"
assert_contains_fixed "$sb/curl.log" \
    "/download/ss-magic-plugin-v0.10.0/ss-magic-plugin-$TRIPLE.tar.gz.sha256" \
    "AE57: fetched the digest sibling from the same tag"
assert_lacks_fixed "$sb/curl.log" "/download/v0.10.0/" \
    "AE57: never reached for the CLI line's bare v<pin> tag"

run_bootstrap                                  # second run must be a no-op
assert_never_fails_session "AE57"
assert_eq 0 "$(stderr_lines)" "AE57: stderr is empty on the no-op path"
assert_eq "$fetches_before" "$(archive_fetches)" "AE57: no further download"
assert_eq "$before" "$(sha256_of "$sb/data/bin/ss-magic-plugin")" "AE57: binary untouched"

# ==========================================================================
# AE58 - no network at all
# ==========================================================================
current_case="AE58 offline"
new_sandbox ae58 0.10.0
publish_release 0.10.0
FAKE_CURL_MODE=offline run_bootstrap
assert_never_fails_session "AE58"
assert_eq 1 "$(stderr_lines)" "AE58: exactly one stderr line"
assert_file_absent "$sb/data/bin/ss-magic-plugin" "AE58: nothing installed"
assert_file_absent "$sb/data/.ss-magic-installed" "AE58: no success marker"
assert_eq "" "$(ls -d "$sb"/data/.ss-magic-stage.* 2>/dev/null)" "AE58: no staging left behind"

# An existing older install must survive an offline session untouched.
current_case="AE58 offline with an older install present"
new_sandbox ae58b 0.10.0
publish_release 0.10.0
run_bootstrap
old_digest=$(sha256_of "$sb/data/bin/ss-magic-plugin")
printf '%s\n' "0.11.0" >"$sb/plugin/ss-magic-plugin.version"
FAKE_CURL_MODE=offline run_bootstrap
assert_never_fails_session "AE58b"
assert_eq 1 "$(stderr_lines)" "AE58b: exactly one stderr line"
assert_eq "$old_digest" "$(sha256_of "$sb/data/bin/ss-magic-plugin")" "AE58b: old binary untouched"
assert_file_absent "$sb/data/.ss-magic-installed" "AE58b: marker cleared so the next session retries"

# ==========================================================================
# AE59 - the archive does not match its published digest
# ==========================================================================
current_case="AE59 checksum mismatch"
new_sandbox ae59 0.10.0
publish_release 0.10.0
run_bootstrap
good_digest=$(sha256_of "$sb/data/bin/ss-magic-plugin")
printf '%s\n' "0.11.0" >"$sb/plugin/ss-magic-plugin.version"
publish_release 0.11.0 --corrupt-digest
run_bootstrap
assert_never_fails_session "AE59"
assert_eq 1 "$(stderr_lines)" "AE59: exactly one stderr line"
assert_contains "$sb/err" "checksum mismatch" "AE59: says what went wrong"
assert_eq "$good_digest" "$(sha256_of "$sb/data/bin/ss-magic-plugin")" "AE59: existing binary untouched"
assert_eq "0.10.0" "$("$sb/data/bin/ss-magic-plugin" --version | awk '{print $NF}')" "AE59: still the old version"
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
    assert_file_absent "$sb/data/bin/ss-magic-plugin" "AE60 [$pin]: nothing installed"
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
assert_file_present "$sb/data/bin/ss-magic-plugin" "AE61: the binary is installed"
assert_eq "0.10.0" "$("$sb/data/bin/ss-magic-plugin" --version | awk '{print $NF}')" "AE61: correct version"
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
kept=$(sha256_of "$sb/data/bin/ss-magic-plugin")
printf '%s\n' "0.11.0" >"$sb/plugin/ss-magic-plugin.version"
publish_release 0.11.0 --empty-archive
run_bootstrap
assert_never_fails_session "AE62"
assert_eq 1 "$(stderr_lines)" "AE62: exactly one stderr line"
assert_eq "$kept" "$(sha256_of "$sb/data/bin/ss-magic-plugin")" "AE62: existing binary untouched"
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
    assert_file_absent "$sb/data/bin/ss-magic-plugin" "AE62b: nothing installed"
fi

# ==========================================================================
# AE63 - advancing the pin replaces the binary
# ==========================================================================
current_case="AE63 pin advance"
new_sandbox ae63 0.10.0
publish_release 0.10.0
publish_release 0.11.0
run_bootstrap
assert_eq "0.10.0" "$("$sb/data/bin/ss-magic-plugin" --version | awk '{print $NF}')" "AE63: starts at the old pin"
printf '%s\n' "0.11.0" >"$sb/plugin/ss-magic-plugin.version"
run_bootstrap
assert_never_fails_session "AE63"
assert_eq "0.11.0" "$("$sb/data/bin/ss-magic-plugin" --version | awk '{print $NF}')" "AE63: advanced to the new pin"
assert_eq "0.11.0" "$(cat "$sb/data/.ss-magic-installed")" "AE63: marker records the new pin"
run_bootstrap
assert_eq 0 "$(stderr_lines)" "AE63: the run after the advance is silent"

# ==========================================================================
# AE63b - the staged binary's `--version` decides the install, both ways
#
# Every install is gated on `"$staged_bin" --version | head -1 | awk '{print
# $NF}'` equalling the pin: the checksum proves the archive is the published
# artifact for this TRIPLE, and this proves it is the artifact for this MACHINE
# (a wrong-architecture build passes every earlier check and then fails to run).
#
# Both directions are asserted here on purpose. A negative-only test - "a binary
# reporting the wrong version installs nothing" - stays green even if the binary
# never answered `--version` at all, or answered with usage text and exit 2,
# because an empty last field also fails to equal the pin. The positive case is
# what pins the contract that the flag ANSWERS: one line, `ss-magic-plugin
# <version>`, exit 0, ahead of any verb parsing.
# ==========================================================================
current_case="AE63b a binary whose --version equals the pin installs"
new_sandbox ae63b 0.10.0
publish_release 0.10.0
run_bootstrap
assert_never_fails_session "AE63b"
assert_file_present "$sb/data/bin/ss-magic-plugin" "AE63b: the matching binary was installed"
assert_eq "ss-magic-plugin 0.10.0" \
    "$("$sb/data/bin/ss-magic-plugin" --version | head -1)" \
    "AE63b: --version answers 'ss-magic-plugin <version>' on one line"
"$sb/data/bin/ss-magic-plugin" --version >/dev/null 2>&1
assert_eq 0 "$?" "AE63b: --version exits 0"
assert_eq "0.10.0" "$(cat "$sb/data/.ss-magic-installed")" "AE63b: marker records the pin"

current_case="AE63b a binary whose --version differs from the pin installs nothing"
new_sandbox ae63c 0.10.0
publish_release 0.10.0
run_bootstrap
kept_digest=$(sha256_of "$sb/data/bin/ss-magic-plugin")
printf '%s\n' "0.11.0" >"$sb/plugin/ss-magic-plugin.version"
publish_release 0.11.0 --wrong-version
run_bootstrap
assert_never_fails_session "AE63c"
assert_eq 1 "$(stderr_lines)" "AE63c: exactly one stderr line"
assert_contains "$sb/err" "did not run as 0.11.0" "AE63c: says the staged binary reported the wrong version"
assert_eq "$kept_digest" "$(sha256_of "$sb/data/bin/ss-magic-plugin")" "AE63c: existing binary untouched"
assert_file_absent "$sb/data/.ss-magic-installed" "AE63c: marker cleared so the next session retries"
assert_eq "" "$(ls -d "$sb"/data/.ss-magic-stage.* 2>/dev/null)" "AE63c: no staging left behind"

# ==========================================================================
# R3a - the configuration pre-seed
#
# After a successful install the bootstrap invokes the binary it just installed
# once, as `seed-config`, so a `plugin` block of gate defaults appears in an
# existing .superset/magic.json and the settings are discoverable without any
# terminal verb (the CLI no longer has a `plugin` subcommand). What the seed
# writes - and everything it refuses to write, above all the `enabled` key - is
# the binary's business and is tested in Rust. What is testable HERE is the
# invocation itself, and its siting: on every session that reaches a usable
# pinned binary, and never on one that does not.
#
# The frequency is the subtle half, and an earlier version of this suite got it
# exactly backwards. The binary is installed once per MACHINE; the block has to
# be seeded once per REPOSITORY, and a person opens many repositories on one
# machine. A seed call placed only after a fresh install therefore seeds the
# first repository and silently skips every later one, because the
# already-installed fast path returns above it - which is the steady state, so
# the failure is the common case rather than an edge. The "no-op session"
# assertion below used to require exactly that behaviour, certifying the bug.
# It now requires the opposite, and a separate case opens a SECOND repository on
# an already-provisioned machine, which is the shape that actually failed.
# ==========================================================================
current_case="R3a a successful install seeds the config exactly once"
new_sandbox seed 0.10.0
publish_release 0.10.0
: >"$sb/fakebin.log"
run_bootstrap
assert_never_fails_session "R3a"
assert_eq "seed-config" "$(cat "$sb/fakebin.log")" "R3a: invoked seed-config exactly once after the install"

current_case="R3a a session that installs nothing still seeds"
# The second session returns at the already-installed check. It must STILL seed:
# it has a usable binary, and it may be sitting in a repository that has never
# been seeded. Idempotence is the binary's job, not the caller's - it writes
# only when there is no `plugin` key at all - so calling it every session costs
# a read and an exit, and needs no marker file kept in sync.
: >"$sb/fakebin.log"
run_bootstrap
assert_never_fails_session "R3a no-op"
assert_eq "seed-config" "$(cat "$sb/fakebin.log")" "R3a: an already-installed session still seeds"

current_case="R3a a SECOND repository on the same machine is seeded"
# The case the once-per-install siting got wrong. Same machine, same installed
# binary, a different working directory: this is what every session after the
# first one looks like, and it is where the whole point of R3a lands - with no
# terminal path to the plugin config, a repository that is never seeded has no
# discoverable configuration at all.
second_repo="$sb/second-repo"
mkdir -p "$second_repo"
: >"$sb/fakebin.log"
RUN_CWD="$second_repo" run_bootstrap
assert_never_fails_session "R3a second repo"
assert_contains_fixed "$sb/fakebin.log" "seed-config" \
    "R3a: a second repository on an already-provisioned machine is seeded"

current_case="R3a a failed install seeds nothing"
# Offline: the run leaves through give_up long before the install-success marker,
# and the seed call sits after that marker, so it is unreachable. This is the
# assertion that pins the SITING - a seed call placed earlier in the file would
# still satisfy every other case in this suite.
new_sandbox seedfail 0.10.0
publish_release 0.10.0
: >"$sb/fakebin.log"
FAKE_CURL_MODE=offline run_bootstrap
assert_never_fails_session "R3a failed install"
assert_file_absent "$sb/data/bin/ss-magic-plugin" "R3a: nothing was installed"
assert_eq "" "$(cat "$sb/fakebin.log")" "R3a: a failed install invokes seed-config zero times"

current_case="R3a a failed UPGRADE over a working install seeds nothing"
# The sharper version of the same property: here a runnable binary IS present,
# so a seed call that ran unconditionally would find something to execute. The
# upgrade fails on its checksum, so the run must still leave through give_up.
new_sandbox seedfail2 0.10.0
publish_release 0.10.0
run_bootstrap
printf '%s\n' "0.11.0" >"$sb/plugin/ss-magic-plugin.version"
publish_release 0.11.0 --corrupt-digest
: >"$sb/fakebin.log"
run_bootstrap
assert_never_fails_session "R3a failed upgrade"
assert_eq 1 "$(stderr_lines)" "R3a failed upgrade: exactly one stderr line"
assert_eq "" "$(cat "$sb/fakebin.log")" "R3a: a failed upgrade invokes seed-config zero times"

# ==========================================================================
# AE64 - hooks.json runs the bootstrap on `startup` only
# ==========================================================================
current_case="AE64 hook wiring"
if command -v python3 >/dev/null 2>&1; then
    if python3 - "$PLUGIN_SRC/hooks/hooks.json" <<'PY'
import json, sys
spec = json.load(open(sys.argv[1]))

# The event token each shim entry must pass as args[1]. These are the tokens
# HookEvent::from_token parses in crates/ss-magic-plugin/src/main.rs; a rename
# on either side without the other is exactly the drift this asserts against.
SHIM_TOKENS = {
    "SessionStart": "session-start",
    "PreToolUse": "pre-tool-use",
    "PreCompact": "pre-compact",
    "SubagentStop": "subagent-stop",
    "SessionEnd": "session-end",
}
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

# The manifest invariant, over EVERY entry rather than the bootstrap alone.
# A command naming ${CLAUDE_PLUGIN_DATA} is a path the bootstrap creates at
# runtime, so on a first install the harness posix_spawns something that is not
# there yet and the session dies with ENOENT instead of running inert (R77).
# Fail-open has to live in a shim that always exists, not inside the artifact
# that may be missing. This assertion used to cover the bootstrap group only,
# which is exactly why the other five drifted.
for ev, egroups in spec["hooks"].items():
    for g in egroups:
        for h in g["hooks"]:
            c = h["command"]
            assert "CLAUDE_PLUGIN_DATA" not in c, (
                f"{ev}: command names a runtime-created artifact ({c}); "
                "spawn a ${CLAUDE_PLUGIN_ROOT} script instead")
            assert c == "bash", f"{ev}: command is {c!r}, expected the exec form 'bash'"
            a = h.get("args") or [None]
            a0 = a[0]
            assert isinstance(a0, str) and a0.startswith("${CLAUDE_PLUGIN_ROOT}/"), (
                f"{ev}: args[0] is {a0!r}; it must be a path under the plugin root")
            # args[0] alone is not the property that matters. Every shim entry also
            # has to dispatch ITS OWN event: the token in args[1] is what run-hook.sh
            # passes to the binary as `hook <token>`, so a manifest that names the right script with
            # the wrong token routes the event to the wrong handler and every other
            # assertion here still passes.
            if "run-hook.sh" in a0:
                expected = SHIM_TOKENS.get(ev)
                assert expected, f"{ev}: no expected shim token is defined for this event"
                assert len(a) == 2, f"{ev}: expected exactly 2 args, got {a!r}"
                assert a[1] == expected, (
                    f"{ev}: dispatches {a[1]!r}, expected {expected!r}")
            # Spelling is not the property that matters - the spawned path has to
            # EXIST in the packaged tree, or the harness posix_spawns a missing
            # file and we are back to the ENOENT this whole shape exists to stop.
            import pathlib
            rel = a0[len("${CLAUDE_PLUGIN_ROOT}/"):]
            target = pathlib.Path(sys.argv[1]).parent.parent / rel
            assert target.is_file() and target.stat().st_size > 0, (
                f"{ev}: args[0] names {rel}, which is missing or empty in the packaged tree")
print("ok")
PY
    then pass "AE64: startup-only bootstrap, compact-covering handler, no runtime-path commands"
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
# The wrapper forwards its argv VERBATIM - `exec "$bin" "$@"`. The verb tree is
# its own binary now, so a skill's `ss-magic-plugin checklist list` IS the
# binary's argv; the wrapper used to prepend a `plugin` token because the same
# code then lived inside the `ss-magic` CLI.
assert_eq "checklist list" "$(cat "$sb/fakebin.log")" "AE65: forwards argv verbatim, no CLAUDE_PLUGIN_DATA in scope"
# Asserted negatively as well as positively. The expectation above would still
# be satisfiable by a wrapper that stripped and re-added tokens; this one says
# the word `plugin` never reaches the binary's argv at all.
assert_lacks_fixed "$sb/fakebin.log" "plugin" "AE65: no 'plugin' token is injected"

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

current_case="AE65 wrapper: a DIRECTORY where the binary should be"
# `[ -x ]` is true for a directory carrying the search bit, so before the shared
# exec guard this reached `exec`, and bash answered with its own diagnostic and
# exit 126 - breaking the wrapper's promise to exit 0 with one line of
# explanation rather than failing the skill mid-run. This is the sibling of the
# same defect fixed earlier in hooks/run-hook.sh; it survived there for a whole
# release because the guard was duplicated instead of shared.
mkdir -p "$sb/data/bin/ss-magic-plugin.dir/bin"
mkdir -p "$sb/data/bin/ss-magic-plugin.dir/bin/ss-magic-plugin"
: >"$sb/fakebin.log"
WRAPPER_DATA="$sb/data/bin/ss-magic-plugin.dir" run_wrapper checklist list
assert_eq 0 "$RC" "AE65: a directory in the binary's place still exits 0"
assert_eq 1 "$(wc -l <"$sb/werr" | tr -d ' ')" "AE65: directory case explains in one line"
assert_eq 0 "$(wc -c <"$sb/wout" | tr -d ' ')" "AE65: directory case says nothing on stdout"
assert_eq "" "$(cat "$sb/fakebin.log")" "AE65: directory case did not invoke the binary"
rm -rf "$sb/data/bin/ss-magic-plugin.dir"

current_case="AE65 wrapper: a present, +x, non-loadable binary"
# The ENOEXEC shape: bash would reinterpret the damaged bytes as a shell script
# and exit with whatever they parse to, spraying that at a person running a skill.
mkdir -p "$sb/data/bin/ss-magic-plugin.bad/bin"
head -c 512 /dev/urandom >"$sb/data/bin/ss-magic-plugin.bad/bin/ss-magic-plugin"
chmod 755 "$sb/data/bin/ss-magic-plugin.bad/bin/ss-magic-plugin"
: >"$sb/fakebin.log"
WRAPPER_DATA="$sb/data/bin/ss-magic-plugin.bad" run_wrapper checklist list
assert_eq 0 "$RC" "AE65: a damaged binary still exits 0"
assert_eq 1 "$(wc -l <"$sb/werr" | tr -d ' ')" "AE65: damaged binary explains in one line"
assert_eq "" "$(cat "$sb/fakebin.log")" "AE65: damaged binary did not invoke anything"
rm -rf "$sb/data/bin/ss-magic-plugin.bad"

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
assert_contains "$sb/err" "installed ss-magic-plugin 0.10.0" "AE67: names the binary and version"
assert_contains "$sb/err" "releases/tag/ss-magic-plugin-v0.10.0" "AE67: names the release it came from"
assert_contains "$sb/err" "SessionStart" "AE67: names the hooks it registers"
printf '%s\n' "0.11.0" >"$sb/plugin/ss-magic-plugin.version"
run_bootstrap
assert_never_fails_session "AE67 second install"
assert_eq 0 "$(stderr_lines)" "AE67: a later successful install is silent"

# ==========================================================================
# AE9 - the hook shim is inert when the pinned binary is absent
#
# The case that shipped broken: hooks.json used to name the binary directly, so
# a first session posix_spawned a path the bootstrap had not created yet and the
# user saw ENOENT. R77 and AE9 both specify "inert", and nothing tested it -
# every Rust test exercises the binary BY RUNNING IT, so none of them can observe
# the state where it is missing.
# ==========================================================================
current_case="AE9 no binary installed at all"
new_sandbox ae9 0.10.0
: >"$sb/fakebin.log"
SHIM_DATA="" run_shim pre-tool-use
assert_shim_inert "AE9 (no handoff, no binary)"

current_case="AE9 CLAUDE_PLUGIN_DATA set but nothing installed there"
: >"$sb/fakebin.log"
SHIM_DATA="$sb/data" run_shim session-start
assert_shim_inert "AE9 (data dir set, binary absent)"

current_case="AE9 execs the installed binary through the handoff"
publish_release 0.10.0
run_bootstrap
: >"$sb/fakebin.log"
SHIM_DATA="" run_shim pre-tool-use
assert_eq 0 "$RC" "AE9: exits 0 with the binary present"
# `hook <event>` and nothing more. The verb tree is its own binary, so its argv
# IS the verb; the shim used to exec `plugin hook <event>` because the same code
# then lived inside the `ss-magic` CLI behind a `plugin` token.
assert_eq "hook pre-tool-use" "$(cat "$sb/fakebin.log")" "AE9: execs with exactly 'hook <event>'"
assert_lacks_fixed "$sb/fakebin.log" "plugin" "AE9: no 'plugin' token precedes the event"
assert_eq 0 "$(wc -c <"$sb/sout" | tr -d ' ')" "AE9: the shim itself adds nothing to stdout"

current_case="AE9 execs through CLAUDE_PLUGIN_DATA, the production path"
# A real hook process always has CLAUDE_PLUGIN_DATA in its environment, so this
# branch - not the handoff fallback - is what runs in production. It was only
# covered in its failure direction.
: >"$sb/fakebin.log"
SHIM_DATA="$sb/data" run_shim session-start
assert_eq 0 "$RC" "AE9: exits 0 on the CLAUDE_PLUGIN_DATA fast path"
assert_eq "hook session-start" "$(cat "$sb/fakebin.log")" "AE9: fast path execs with the right argv"
assert_eq 0 "$(wc -c <"$sb/sout" | tr -d ' ')" "AE9: fast path adds nothing to stdout"
assert_eq 0 "$(wc -c <"$sb/serr" | tr -d ' ')" "AE9: fast path adds nothing to stderr"

current_case="AE9 binary path exists but is a directory"
# `-x` alone is true for a directory with the search bit; exec would then print a
# diagnostic and exit 126 from the one script that must never do either.
mv "$sb/data/bin/ss-magic-plugin" "$sb/data/bin/ss-magic-plugin.real"
mkdir -p "$sb/data/bin/ss-magic-plugin"
: >"$sb/fakebin.log"
SHIM_DATA="$sb/data" run_shim pre-tool-use
assert_shim_inert "AE9 (binary path is a directory)"
rmdir "$sb/data/bin/ss-magic-plugin"
mv "$sb/data/bin/ss-magic-plugin.real" "$sb/data/bin/ss-magic-plugin"

current_case="AE9 handoff removed after install"
shim_root="/tmp/ss-magic-plugin/$SB_ID"
[ -d "$shim_root" ] || shim_root="$sb/tmp/ss-magic-plugin/$SB_ID"
rm -f "$shim_root/data-root"
: >"$sb/fakebin.log"
SHIM_DATA="" run_shim session-end
assert_shim_inert "AE9 (handoff removed)"

current_case="AE9 no event argument"
: >"$sb/fakebin.log"
SHIM_DATA="$sb/data" run_shim
assert_shim_inert "AE9 (missing event token)"

current_case="AE9 binary is present and +x but is not a loadable executable"
# `-f` and `-x` both pass here: the file is a regular file with the execute bit.
# What they cannot see is execve failing on the way in. On the ENOEXEC path bash
# does NOT report an exec failure - it falls back to reinterpreting the file as a
# shell script, so `shopt -s execfail` never fires, and the process exits with
# whatever those bytes parse to. Measured over 30 corrupted binaries on bash 3.2:
# exit 2 roughly half the time, and exit 2 from PreToolUse means BLOCK the tool
# call. That is why run-hook.sh checks the magic number before exec.
head -c 512 /dev/urandom >"$sb/data/bin/ss-magic-plugin"
chmod 755 "$sb/data/bin/ss-magic-plugin"
: >"$sb/fakebin.log"
SHIM_DATA="$sb/data" run_shim pre-tool-use
assert_shim_inert "AE9 (binary is not a loadable executable)"

current_case="AE9 a shebang wrapper is still accepted"
# The magic check must not become a rule that only real compiled binaries pass:
# an interpreted wrapper is a legitimate shape, and the suite's own fake binary
# is one, so a guard that rejected `#!` would make every other case vacuous.
printf '#!/usr/bin/env bash\nprintf "%%s\\n" "$*" >>"${SS_MAGIC_FAKE_LOG:-/dev/null}"\nexit 0\n' >"$sb/data/bin/ss-magic-plugin"
chmod 755 "$sb/data/bin/ss-magic-plugin"
: >"$sb/fakebin.log"
SHIM_DATA="$sb/data" run_shim pre-tool-use
assert_eq 0 "$RC" "AE9: a shebang wrapper still execs"
assert_eq "hook pre-tool-use" "$(cat "$sb/fakebin.log")" "AE9: shebang wrapper receives the right argv"

# ==========================================================================
# AE64 (dynamic) - every manifest entry dispatches ITS OWN event, end to end
#
# The static assertion above reads hooks.json as JSON. That is not the same as
# running it: nothing else in the suite expands ${CLAUDE_PLUGIN_ROOT} and spawns
# the command the way the harness does, because AE9 invokes the shim directly
# with a token this script chose. So a manifest could name the right script with
# the wrong token and every other test would stay green.
# ==========================================================================
current_case="AE64 dynamic: each entry reaches the binary with its own event"
if command -v python3 >/dev/null 2>&1; then
    manifest_entries=$(python3 - "$sb/plugin/hooks/hooks.json" <<'PY_ENTRIES'
import json, sys
spec = json.load(open(sys.argv[1]))
TOK = {"SessionStart": "session-start", "PreToolUse": "pre-tool-use",
       "PreCompact": "pre-compact", "SubagentStop": "subagent-stop",
       "SessionEnd": "session-end"}
for ev, groups in spec["hooks"].items():
    for g in groups:
        for h in g["hooks"]:
            a = h.get("args") or []
            if not a or "run-hook.sh" not in a[0]:
                continue          # the bootstrap entry is spawned directly, by design
            print("%s\t%s\t%s\t%s" % (ev, TOK[ev], h["command"], " ".join(a)))
PY_ENTRIES
)
    entry_count=0
    while IFS="$(printf '\t')" read -r ev tok cmd argv; do
        [ -n "$ev" ] || continue
        entry_count=$((entry_count + 1))
        : >"$sb/fakebin.log"
        expanded=$(printf '%s' "$argv" | sed "s|\${CLAUDE_PLUGIN_ROOT}|$sb/plugin|g")
        # Deliberately unquoted: $expanded is the manifest's argv vector, and the
        # harness spawns it as separate arguments, not as one string.
        env -i PATH="/usr/bin:/bin" HOME="$sb/home" TMPDIR="$sb/tmp" \
            CLAUDE_PLUGIN_DATA="$sb/data" SS_MAGIC_FAKE_LOG="$sb/fakebin.log" \
            "$cmd" $expanded </dev/null >"$sb/sout" 2>"$sb/serr"
        assert_eq 0 "$?" "AE64 dynamic: $ev exits 0"
        assert_eq "hook $tok" "$(cat "$sb/fakebin.log")" \
            "AE64 dynamic: $ev reaches the binary as '$tok'"
        assert_eq 0 "$(wc -c <"$sb/sout" | tr -d ' ')" "AE64 dynamic: $ev adds nothing to stdout"
    done <<MANIFEST_ENTRIES
$manifest_entries
MANIFEST_ENTRIES
    assert_eq 5 "$entry_count" "AE64 dynamic: all five event hooks were exercised"
else
    printf '  skip AE64 dynamic: python3 is not available\n'
fi

# ==========================================================================
printf '\n%s: %d passed, %d failed\n' "$(basename "$0")" "$passed" "$failed"
[ "$failed" -eq 0 ] || exit 1
exit 0
