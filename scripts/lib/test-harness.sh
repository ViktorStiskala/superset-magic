# The assertion harness shared by scripts/test-bootstrap.sh and
# scripts/test-mark-latest.sh. Sourced, never executed.
#
# One file rather than a copy in each script, for the reason plugin/lib/ holds
# one exec guard: two byte-identical copies drift the moment one is fixed. The
# caller sets VERBOSE (`-v` for per-assertion output) before sourcing, names
# each case in `current_case`, and prints its own summary from `passed` and
# `failed` at the end.
#
# Written for bash 3.2, which is what macOS ships: no associative arrays, no
# `${var^^}`, no `mapfile`.

passed=0
failed=0
current_case="(none)"

pass() { passed=$((passed + 1)); [ "${VERBOSE:-}" = "-v" ] && printf '  ok   %s\n' "$1"; return 0; }
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
assert_empty_file() { # file label
    if [ -s "$1" ]; then fail "$2 (not empty: $(cat "$1"))"; else pass "$2"; fi
}
assert_contains() { # haystack-file needle label
    if grep -q -- "$2" "$1" 2>/dev/null; then pass "$3"; else fail "$3 (no [$2] in $1)"; fi
}
# The fixed-string variant. A URL is mostly punctuation, and `.` and `-` in a
# basic regular expression would let a near-miss match, which is the one thing a
# "the URL is spelled exactly this way" assertion must not do.
assert_contains_fixed() { # haystack-file needle label
    if grep -qF -- "$2" "$1" 2>/dev/null; then pass "$3"; else fail "$3 (no [$2] in $1)"; fi
}
# The negative form: the needle must NOT appear. Used for "no leading `plugin`
# token", where asserting the new argv alone would still pass if the wrapper
# grew a second, differently-spelled injection.
assert_lacks_fixed() { # haystack-file needle label
    if grep -qF -- "$2" "$1" 2>/dev/null; then fail "$3 (found [$2] in $1)"; else pass "$3"; fi
}
