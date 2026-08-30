#!/usr/bin/env bash
# PLACEHOLDER - NOT THE SHIPPING BOOTSTRAP.
#
# The real SessionStart bootstrap is owned by U23. It reads
# ${CLAUDE_PLUGIN_ROOT}/ss-magic.version, compares it against the binary already
# installed at ${CLAUDE_PLUGIN_DATA}/bin/ss-magic, validates the pin against a
# strict MAJOR.MINOR.PATCH pattern before composing any URL, takes a lock,
# installs into a staging directory and moves the result into place.
#
# Until U23 lands, this file exists only so the plugin tree is complete: the
# hooks manifest names it, `claude plugin validate` resolves it, and the zip
# builder has a real file to package. It installs nothing.
#
# Deliberately no `set -e`: every path in the shipping bootstrap exits 0 so a
# failed install can never block a session start (R72). One stderr line at most,
# never stdout, because a hook's stdout enters the model's context.
set -u
echo "ss-magic: bootstrap placeholder - the pinned binary was not installed (U23 pending)." >&2
exit 0
