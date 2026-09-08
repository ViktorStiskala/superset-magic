#!/usr/bin/env python3
"""Deterministic builder for the ss-magic plugin zip (R96), plus the release
assertions CI runs against it (R2, R95, R98, R101).

The workspace publishes TWO release lines out of one repository -- `ss-magic` on
a bare `vX.Y.Z` tag, `ss-magic-plugin` on `ss-magic-plugin-vX.Y.Z` -- so every
version surface below belongs to exactly one of them, and `--check` asserts each
line's surfaces separately as well as the rule that the two versions must never
be equal (see `check_versions`).

The marketplace manifest at .claude-plugin/marketplace.json pins the plugin zip
by SHA-256, and that digest has to be committed *before* the release tag exists
-- so the zip must be a pure function of the file contents and paths under
plugin/, and of nothing else. Everything this script does follows from that:

  * entries are sorted explicitly, never left in directory-iteration order,
    which differs between filesystems;
  * every entry is stamped 1980-01-01 (the ZIP epoch), never an mtime and never
    a clock;
  * modes are normalised to 0644, or 0755 under bin/ and for *.sh, so a stray
    chmod or a different umask cannot reach the bytes;
  * create_system is forced to unix (3), so the archive does not record which OS
    built it;
  * entries are STORED, never deflated, so no zlib build difference can reach
    the bytes;
  * .DS_Store is excluded;
  * a symlink or a non-ASCII filename is a loud refusal rather than a
    best-effort archive: macOS normalises filenames to NFD and Linux to NFC, and
    the two hash differently, so a non-ASCII name would make the digest a
    function of who built it.

No directory entries are emitted at all. Extraction recreates parents from the
file paths, and leaving directories out means an empty directory (or a
directory-creation order difference) can never perturb the digest.

`git archive` is deliberately not used: its tree-ish form stamps the current
time into every entry and its commit-ish form binds them to the committer time,
which reintroduces exactly the self-pinning problem the archive source was
adopted to escape.

Standard library only, so the same file runs unchanged on a developer machine
and on the Linux CI runner.

Usage:
    build-plugin-zip.py                       # build to the default output path
    build-plugin-zip.py --out FILE            # build to FILE
    build-plugin-zip.py --print-digest        # compute only, print the digest
    build-plugin-zip.py --update-manifest     # write the digest into marketplace.json
    build-plugin-zip.py --check               # the release assertions (see main)
    build-plugin-zip.py --check-bump REF      # content-changed-without-version-bump (R98)
    build-plugin-zip.py --selftest            # the builder's own tests
"""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
import re
import stat
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path

# The ZIP epoch. Any earlier value is unrepresentable in the DOS timestamp field.
FIXED_DATE_TIME = (1980, 1, 1, 0, 0, 0)
UNIX_CREATE_SYSTEM = 3
MODE_FILE = 0o644
MODE_EXEC = 0o755
EXCLUDED_NAMES = frozenset({".DS_Store"})
VERSION_RE = re.compile(r"^\d+\.\d+\.\d+$")
ARTIFACT_RE = re.compile(r"ss-magic-plugin-v(\d+\.\d+\.\d+)\.zip")
SHA256_RE = re.compile(r"^[0-9a-fA-F]{64}$")
TAG_RE = re.compile(r"^v(\d+\.\d+\.\d+)$")

REPO_ROOT = Path(__file__).resolve().parent.parent

# The two release lines. They are named here because almost everything below is
# indexed by them: the version surfaces, the two `--check` version assertions,
# and the "must differ" rule between them.
CLI_LINE = "ss-magic"
PLUGIN_LINE = "ss-magic-plugin"


class BuildError(Exception):
    """A refusal: the tree cannot be packaged reproducibly, or a check failed."""


# --------------------------------------------------------------------------
# Collecting the tree
# --------------------------------------------------------------------------


def _reject_name(rel: str, entry: Path) -> None:
    """Refuse anything whose name would make the digest platform-dependent."""
    for ch in rel:
        if ch == "/":
            continue
        if not (0x20 <= ord(ch) <= 0x7E):
            raise BuildError(
                f"{entry}: non-ASCII or non-printable character {ch!r} in the path "
                f"{rel!r}. macOS stores such names decomposed (NFD) and Linux composed "
                f"(NFC); the two hash differently, so the zip's digest would depend on "
                f"which machine built it. Rename the file to plain ASCII."
            )


def collect_entries(plugin_dir: Path) -> list[tuple[str, Path]]:
    """Return (arcname, path) pairs, sorted by arcname, for every packaged file.

    Refuses loudly on a symlink, a non-ASCII name, or anything that is neither a
    regular file nor a directory.
    """
    plugin_dir = Path(plugin_dir)
    if not plugin_dir.is_dir():
        raise BuildError(
            f"{plugin_dir}: the plugin tree does not exist (or is not a directory). "
            f"Nothing to package."
        )

    entries: list[tuple[str, Path]] = []

    def walk(directory: Path, prefix: str) -> None:
        # scandir order is filesystem-dependent; the sort at the end is what
        # makes the output deterministic, but recursing in sorted order also
        # keeps error messages stable.
        for child in sorted(directory.iterdir(), key=lambda p: p.name):
            rel = f"{prefix}{child.name}"
            _reject_name(rel, child)
            if child.is_symlink():
                raise BuildError(
                    f"{child}: symlinks cannot be packaged reproducibly. A symlink's "
                    f"stored target and mode vary by platform, and an extractor may "
                    f"follow it, so the archive would no longer be a pure function of "
                    f"the tree. Replace it with a regular file."
                )
            st = child.stat()
            if stat.S_ISDIR(st.st_mode):
                walk(child, f"{rel}/")
            elif stat.S_ISREG(st.st_mode):
                if child.name in EXCLUDED_NAMES:
                    continue
                entries.append((rel, child))
            else:
                raise BuildError(
                    f"{child}: not a regular file or directory "
                    f"(mode {stat.filemode(st.st_mode)}); refusing to package it."
                )

    walk(plugin_dir, "")
    if not entries:
        raise BuildError(f"{plugin_dir}: contains no packageable files.")
    # Names are ASCII-only by the check above, so a str sort is a byte sort.
    entries.sort(key=lambda pair: pair[0])
    return entries


def mode_for(arcname: str) -> int:
    """0755 under bin/ and for *.sh; 0644 for everything else.

    Normalising rather than reading the mode off disk is what keeps a stray
    chmod, a different umask, or a filesystem that does not carry an exec bit
    out of the digest.
    """
    if arcname.startswith("bin/") or arcname.endswith(".sh"):
        return MODE_EXEC
    return MODE_FILE


def build_zip_bytes(plugin_dir: Path) -> bytes:
    """Produce the archive's bytes. Pure in the tree's contents and paths."""
    entries = collect_entries(plugin_dir)
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", compression=zipfile.ZIP_STORED) as zf:
        for arcname, path in entries:
            info = zipfile.ZipInfo(arcname, date_time=FIXED_DATE_TIME)
            info.compress_type = zipfile.ZIP_STORED
            info.create_system = UNIX_CREATE_SYSTEM
            info.create_version = zipfile.DEFAULT_VERSION
            info.extract_version = zipfile.DEFAULT_VERSION
            info.flag_bits = 0
            info.internal_attr = 0
            # The high 16 bits are the unix mode; S_IFREG marks it a regular file.
            info.external_attr = (stat.S_IFREG | mode_for(arcname)) << 16
            info.extra = b""
            info.comment = b""
            zf.writestr(info, path.read_bytes())
    return buf.getvalue()


def digest_of(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


# --------------------------------------------------------------------------
# Version surfaces (R95)
# --------------------------------------------------------------------------


def _cargo_toml_version(path: Path) -> str:
    """Read [package] version without a TOML parser (tomllib is 3.11+ only)."""
    table = None
    for line in path.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if stripped.startswith("[") and stripped.endswith("]"):
            table = stripped
            continue
        if table == "[package]":
            m = re.match(r'^version\s*=\s*"([^"]+)"', stripped)
            if m:
                return m.group(1)
    raise BuildError(f"{path}: no [package] version found.")


def _cargo_lock_version(path: Path, crate: str) -> str:
    text = path.read_text(encoding="utf-8")
    m = re.search(
        r'^name\s*=\s*"%s"\s*\nversion\s*=\s*"([^"]+)"' % re.escape(crate),
        text,
        re.MULTILINE,
    )
    if not m:
        raise BuildError(f"{path}: no [[package]] entry for {crate!r}.")
    return m.group(1)


def _marketplace_entry(root: Path) -> dict:
    path = root / ".claude-plugin" / "marketplace.json"
    doc = json.loads(path.read_text(encoding="utf-8"))
    plugins = doc.get("plugins")
    if not isinstance(plugins, list) or len(plugins) != 1:
        raise BuildError(
            f"{path}: expected exactly one entry in `plugins`; a plugin name may be "
            f"declared only once per marketplace and there is no fallback source."
        )
    return plugins[0]


def _read_text(root: Path, rel: Path | str, what: str) -> str:
    """Read a file that must exist, turning a missing one into a named refusal."""
    path = root / rel
    try:
        return path.read_text(encoding="utf-8")
    except FileNotFoundError:
        raise BuildError(f"{rel}: {what} is missing.") from None


# The two crate manifests. The repository root is a VIRTUAL workspace manifest
# (no [package] table), so a crate version lives on its member; reading the root
# would raise "no [package] version found" from every entry point.
CLI_MANIFEST = Path("crates") / "ss-magic" / "Cargo.toml"
PLUGIN_MANIFEST = Path("crates") / "ss-magic-plugin" / "Cargo.toml"
CORE_MANIFEST = Path("crates") / "ss-magic-core" / "Cargo.toml"

# The bootstrap's version pin, named for the line it pins. It used to be
# `plugin/ss-magic.version`, back when the plugin rode the CLI's release.
PLUGIN_PIN = Path("plugin") / "ss-magic-plugin.version"

# The one surface in the ss-magic group that is NOT compared for equality. It
# holds the RAW `v`-prefixed tag README's documented install command interpolates,
# and `check_versions` compares it with `<=` instead -- see the reasoning there.
README_SURFACE = "README.md installer tag"

# README pins the installer to a specific published release rather than using
# `releases/latest/download/...`; this reads whatever tag it names, so a
# malformed one is reported rather than silently skipped.
README_INSTALLER_RE = re.compile(r"releases/download/([^/\s)]+)/ss-magic-installer\.sh")


def _artifact_versions(text: str) -> list[str]:
    return ARTIFACT_RE.findall(text)


def _extra_artifact_versions(root: Path) -> tuple[str, list[str]]:
    """The zip filename cargo-dist is told to publish, and where it was declared.

    The entry lives on the plugin crate as `[[package.metadata.dist.extra-artifacts]]`
    -- the zip is the plugin's release asset, so it must ride the plugin's tag and
    not the CLI's. The workspace-level fallback exists only so a tree that still
    declares it in `dist-workspace.toml` is still CHECKED: dropping the fallback
    would make a stale workspace-level entry silently unchecked rather than wrong.
    """
    plugin_text = _read_text(root, PLUGIN_MANIFEST, "the plugin crate manifest")
    versions = _artifact_versions(plugin_text)
    if versions:
        return str(PLUGIN_MANIFEST), versions

    dist_path = root / "dist-workspace.toml"
    if dist_path.is_file():
        versions = _artifact_versions(dist_path.read_text(encoding="utf-8"))
        if versions:
            return "dist-workspace.toml", versions

    raise BuildError(
        f"{PLUGIN_MANIFEST}: no [[package.metadata.dist.extra-artifacts]] entry naming "
        f"ss-magic-plugin-vX.Y.Z.zip (and none in dist-workspace.toml either); the "
        f"plugin zip would not be published at all."
    )


def _readme_installer_tag(root: Path) -> str:
    """The RAW tag README's documented installer URL names, or "" if it names none."""
    m = README_INSTALLER_RE.search(_read_text(root, "README.md", "the README"))
    return m.group(1) if m else ""


def version_surfaces(root: Path) -> dict[str, dict[str, str]]:
    """Every place a release has to advance, GROUPED BY RELEASE LINE (R95).

    Returns `{"ss-magic": {surface: version}, "ss-magic-plugin": {surface: version}}`.
    The two binaries release independently, on distinct tag shapes, so a surface
    belongs to exactly one line and the two lines' versions are unrelated numbers.

    One entry is deliberately not an equality surface: `README_SURFACE` carries the
    raw `v`-prefixed installer tag, which `check_versions` pops out by key and
    compares with `<=` rather than `==`.
    """
    cli: dict[str, str] = {}
    cli[str(CLI_MANIFEST)] = _cargo_toml_version(root / CLI_MANIFEST)
    cli["Cargo.lock (ss-magic)"] = _cargo_lock_version(root / "Cargo.lock", CLI_LINE)
    cli[README_SURFACE] = _readme_installer_tag(root)

    plugin: dict[str, str] = {}
    plugin[str(PLUGIN_MANIFEST)] = _cargo_toml_version(root / PLUGIN_MANIFEST)
    plugin["Cargo.lock (ss-magic-plugin)"] = _cargo_lock_version(
        root / "Cargo.lock", PLUGIN_LINE
    )

    plugin_manifest = json.loads(
        _read_text(root, Path("plugin") / ".claude-plugin" / "plugin.json", "the plugin manifest")
    )
    plugin["plugin/.claude-plugin/plugin.json"] = plugin_manifest.get("version", "")

    pin = _read_text(root, PLUGIN_PIN, "the bootstrap's version pin").strip()
    if not VERSION_RE.match(pin):
        raise BuildError(
            f"{PLUGIN_PIN}: {pin!r} is not a bare MAJOR.MINOR.PATCH literal. "
            f"The bootstrap interpolates this value into a release URL, so it is "
            f"validated before use and must not carry a `v` prefix or trailing text."
        )
    plugin[str(PLUGIN_PIN)] = pin

    entry = _marketplace_entry(root)
    url = (entry.get("source") or {}).get("url", "")
    tag = re.search(r"/download/ss-magic-plugin-v(\d+\.\d+\.\d+)/", url)
    if not tag:
        raise BuildError(
            f".claude-plugin/marketplace.json: cannot read a plugin release tag out of "
            f"the archive url {url!r}; expected "
            f".../releases/download/ss-magic-plugin-vX.Y.Z/... The plugin rides its own "
            f"tag shape, not the CLI's bare vX.Y.Z."
        )
    plugin["marketplace.json url (tag)"] = tag.group(1)
    asset = ARTIFACT_RE.search(url)
    if not asset:
        raise BuildError(
            f".claude-plugin/marketplace.json: the archive url {url!r} does not name a "
            f"ss-magic-plugin-vX.Y.Z.zip asset. Per-version asset filenames are what "
            f"stop a later release from overwriting an earlier pin's target."
        )
    plugin["marketplace.json url (asset)"] = asset.group(1)

    source, dist_versions = _extra_artifact_versions(root)
    for i, v in enumerate(dist_versions):
        plugin[f"{source} artifact #{i + 1}"] = v

    return {CLI_LINE: cli, PLUGIN_LINE: plugin}


def default_out_path(root: Path) -> Path:
    """Derive the asset filename from the PLUGIN crate's version.

    The zip is the plugin's release asset and carries the plugin's version, so
    this reads `crates/ss-magic-plugin/Cargo.toml` -- not the CLI's manifest and
    not the virtual root. A bare `python3 scripts/build-plugin-zip.py` (CI's asset
    step, cargo-dist's extra-artifacts command) goes through here and only here,
    so naming the wrong manifest would break the release while `--check` and
    `--selftest` stayed green.
    """
    return root / f"ss-magic-plugin-v{_cargo_toml_version(root / PLUGIN_MANIFEST)}.zip"


# --------------------------------------------------------------------------
# The release assertions CI runs
# --------------------------------------------------------------------------


def check_manifest_keys(root: Path) -> list[str]:
    """R101: the entry must actually carry a `sha256`, spelled correctly.

    `sha256` is optional in the marketplace schema and `claude plugin validate`
    silently ignores unknown keys inside a source object, so `"sha"` instead of
    `"sha256"` validates cleanly and installs the plugin with no integrity check
    at all. Nothing warns and nothing verifies, which is why this is checked
    mechanically rather than by review.
    """
    problems: list[str] = []
    entry = _marketplace_entry(root)
    source = entry.get("source")
    if not isinstance(source, dict):
        return [
            "marketplace entry `source` is not an object; an `archive` source with a "
            "`sha256` is the only pinned form."
        ]
    if source.get("source") != "archive":
        problems.append(
            f"marketplace entry source.source is {source.get('source')!r}, expected "
            f"'archive' (the only source shape that pins by content digest)."
        )
    url = source.get("url", "")
    if not url.startswith("https://"):
        problems.append(f"marketplace entry source.url is not https: {url!r}")
    if "sha256" not in source:
        near = [k for k in source if k != "sha256" and "sha" in k.lower()]
        hint = f" Did you mean `sha256` rather than {near[0]!r}?" if near else ""
        problems.append(
            "marketplace entry source has NO `sha256` key. The plugin would install "
            "unpinned, with no integrity check, and `claude plugin validate` would "
            "report no problem." + hint
        )
    else:
        value = source["sha256"]
        if not isinstance(value, str) or not SHA256_RE.match(value):
            problems.append(
                f"marketplace entry source.sha256 is not 64 hex characters: {value!r}"
            )
    unknown = set(source) - {"source", "url", "sha256"}
    if unknown:
        problems.append(
            f"marketplace entry source carries unknown key(s) {sorted(unknown)}; "
            f"validation ignores them silently, so a typo hides here."
        )
    return problems


def _group_disagreement(line: str, surfaces: dict[str, str]) -> list[str]:
    distinct = sorted(set(surfaces.values()))
    if len(distinct) <= 1:
        return []
    lines = [f"{line} version surfaces disagree ({', '.join(distinct)}):"]
    for name, value in surfaces.items():
        lines.append(f"    {value:<12} {name}")
    return ["\n".join(lines)]


def _readme_pin_problems(pin: str, cli_version: str) -> list[str]:
    """The README installer pin is `<=` the crate version, NOT equal to it.

    README documents `releases/download/v<V>/ss-magic-installer.sh` rather than
    `releases/latest/download/...`, so the command a reader copies is reproducible
    and cannot be repointed by a later release. That means the pin names the last
    PUBLISHED CLI release, while the crate manifest names the one being PREPARED.
    The release procedure is bump -> merge -> tag, so between a merged bump and a
    pushed tag the crate version is legitimately AHEAD of the pin, and requiring
    equality would fail `--check` for that whole window -- or, if the pin were
    bumped to match, would 404 the documented install command for exactly as long,
    since the release it names does not exist yet.

    So: the pin must be a well-formed `v` + MAJOR.MINOR.PATCH that DOES NOT EXCEED
    the crate version. A pin AHEAD of the crate version is always wrong -- it names
    a release that can never be built from this tree. The comparison is on integer
    triples, never on strings: "0.9.0" sorts after "0.11.0" lexically.
    """
    if not pin:
        return [
            "README.md documents no pinned installer release. Expected a "
            "`releases/download/vX.Y.Z/ss-magic-installer.sh` URL naming the last "
            "published CLI release; `releases/latest/download/...` is deliberately "
            "not used, because the command a reader copies must be reproducible."
        ]
    m = TAG_RE.match(pin)
    if not m:
        return [
            f"README.md's pinned installer tag {pin!r} is not a well-formed "
            f"`v` + MAJOR.MINOR.PATCH release tag."
        ]
    if _semver(m.group(1)) > _semver(cli_version):
        return [
            f"README.md pins the installer to v{m.group(1)}, which is AHEAD of "
            f"{CLI_MANIFEST}'s {cli_version}. That release cannot exist yet, so the "
            f"documented install command 404s. The pin may lag the crate version "
            f"(bump -> merge -> tag), never lead it."
        ]
    return []


def check_versions(root: Path) -> list[tuple[str, list[str]]]:
    """R95, as three separately-reported assertions.

    Returns `(label, problems)` pairs so `--check` prints one ok/FAIL line each:

      (a) each release line's own surfaces agree;
      (b) the two lines' versions DIFFER;
      (c) README's pinned installer tag does not exceed the CLI crate version.
    """
    groups = version_surfaces(root)
    cli = dict(groups[CLI_LINE])
    plugin = groups[PLUGIN_LINE]

    readme_pin = cli.pop(README_SURFACE)
    cli_version = cli[str(CLI_MANIFEST)]
    plugin_version = plugin[str(PLUGIN_MANIFEST)]

    cli_problems = _group_disagreement(CLI_LINE, cli)
    plugin_problems = _group_disagreement(PLUGIN_LINE, plugin)

    # A crate version that is not MAJOR.MINOR.PATCH is reported here rather than
    # left to blow up in `_semver` below: a manifest carrying "0.11.1-rc1" would
    # otherwise reach the integer comparison and raise a ValueError traceback
    # instead of the named refusal every other malformed surface gets.
    for problems, line, manifest, value in (
        (cli_problems, CLI_LINE, CLI_MANIFEST, cli_version),
        (plugin_problems, PLUGIN_LINE, PLUGIN_MANIFEST, plugin_version),
    ):
        if not VERSION_RE.match(value):
            problems.append(
                f"{manifest} declares version {value!r}, which is not a bare "
                f"MAJOR.MINOR.PATCH. The {line} release tag is derived from it, and "
                f"every other surface on this line is compared against it."
            )

    if VERSION_RE.match(cli_version):
        cli_problems.extend(_readme_pin_problems(readme_pin, cli_version))

    # (b) The two versions must never be equal. cargo-dist parses a release tag as
    # `[PACKAGE_NAME-]VERSION`, so a bare `v0.11.1` announces EVERY dist-able
    # package that sits at 0.11.1 -- there is no way to say "this tag means the CLI
    # only". Keeping the numbers apart is therefore the whole mechanism by which a
    # bare CLI tag releases the CLI alone: at equal versions, one `v1.0.0` push
    # would publish a plugin release nobody asked for, under a tag the plugin's own
    # line does not use.
    distinct: list[str] = []
    if cli_version == plugin_version:
        distinct.append(
            f"both release lines sit at {cli_version}. cargo-dist reads a tag as "
            f"`[PACKAGE_NAME-]VERSION`, so a bare `v{cli_version}` tag announces EVERY "
            f"dist-able package at that version -- pushing the CLI's tag would publish "
            f"a `ss-magic-plugin` release too, under a tag shape the plugin line never "
            f"uses. The two versions must always differ; bump one of them."
        )

    return [
        (f"R95 version surfaces ({CLI_LINE})", cli_problems),
        (f"R95 version surfaces ({PLUGIN_LINE})", plugin_problems),
        ("distinct release lines", distinct),
    ]


# The only two scripts a hook entry may spawn. Both ship inside
# ${CLAUDE_PLUGIN_ROOT}, which the harness materialises before any hook fires.
HOOK_ENTRYPOINTS = (
    "${CLAUDE_PLUGIN_ROOT}/hooks/run-hook.sh",
    "${CLAUDE_PLUGIN_ROOT}/hooks/bootstrap.sh",
)


def check_hooks_shim(root: Path) -> list[str]:
    """Every hook entry must spawn `bash` on a shipped script, never a binary.

    `${CLAUDE_PLUGIN_DATA}/bin/ss-magic-plugin` does not exist until the
    SessionStart bootstrap fetches it, and hooks on one event fire CONCURRENTLY --
    so a manifest naming the binary directly makes the harness `posix_spawn` a
    missing path and the session dies with ENOENT on a first install. The shim
    (`hooks/run-hook.sh`) ships inside ${CLAUDE_PLUGIN_ROOT} and is therefore
    always present; it does nothing when it cannot resolve a binary, which is the
    "every hook inert for that session" behaviour a first install requires. The
    bootstrap is the one entry that may not go through the shim, because it is the
    thing that installs the binary the shim looks for.

    This is a DELIBERATE DUPLICATE of an assertion scripts/test-bootstrap.sh
    already makes over every hooks.json entry. It is duplicated so the
    one-command release gate (`--check`) covers it without running the bash
    suite. The two must be changed together: weakening one and leaving the other
    is how a manifest regression reaches a release through whichever gate a
    given run happens to skip.
    """
    problems: list[str] = []
    rel = Path("plugin") / "hooks" / "hooks.json"
    text = _read_text(root, rel, "the hook manifest")
    try:
        doc = json.loads(text)
    except json.JSONDecodeError as exc:
        raise BuildError(f"{rel}: not valid JSON ({exc}).") from None

    hooks = doc.get("hooks")
    if not isinstance(hooks, dict) or not hooks:
        return [f"{rel}: no `hooks` object; the plugin would register nothing."]

    for event in sorted(hooks):
        groups = hooks[event]
        if not isinstance(groups, list):
            problems.append(f"{rel}: {event} is not a list of matcher groups.")
            continue
        for gi, group in enumerate(groups):
            entries = (group or {}).get("hooks") if isinstance(group, dict) else None
            if not isinstance(entries, list):
                problems.append(f"{rel}: {event}[{gi}] carries no `hooks` list.")
                continue
            for ei, entry in enumerate(entries):
                where = f"{rel}: {event}[{gi}].hooks[{ei}]"
                if not isinstance(entry, dict):
                    problems.append(f"{where} is not an object.")
                    continue
                if entry.get("type") != "command":
                    problems.append(
                        f"{where} has type {entry.get('type')!r}, expected 'command'."
                    )
                command = entry.get("command")
                if command != "bash":
                    problems.append(
                        f"{where} spawns {command!r}, not 'bash'. The binary does not "
                        f"exist until the SessionStart bootstrap fetches it, so a hook "
                        f"naming it makes the harness posix_spawn a missing path and the "
                        f"session dies with ENOENT on a first install."
                    )
                args = entry.get("args")
                first = args[0] if isinstance(args, list) and args else None
                if first not in HOOK_ENTRYPOINTS:
                    problems.append(
                        f"{where} runs {first!r} as its first argument; it must be one "
                        f"of {list(HOOK_ENTRYPOINTS)}. Both ship inside "
                        f"${{CLAUDE_PLUGIN_ROOT}} and therefore always exist."
                    )
    return problems


# Absent by requirement, not by accident (R2). A `cargo build` proves a
# dependency is PRESENT and can say nothing about one being ABSENT, so the
# absence is asserted here (and by `cargo tree -i` in CI).
FORBIDDEN_PLUGIN_DEPS = ("self_update", "inquire", "ratatui")
_DEP_TABLES = frozenset({"dependencies", "dev-dependencies", "build-dependencies"})


def _declared_dependency_names(text: str) -> set[str]:
    """Crate names a manifest declares as a dependency, in any dependency table.

    Line-oriented rather than a TOML parse (`tomllib` is 3.11+). Comments are
    skipped -- which matters here, because the plugin manifest's own comment names
    all three forbidden crates in prose. Renames (`foo = { package = "bar" }`) are
    resolved to the real crate name, since the whole point is that the crate is
    not linked in, whatever it is called locally. Hyphens are folded to
    underscores so `self-update` and `self_update` are one name.
    """
    names: set[str] = set()
    table: list[str] = []
    # True inside `[dependencies]` itself (each line names a crate) and inside
    # a per-crate sub-table `[dependencies.foo]` (whose body may rename it via
    # `package = "..."`); false everywhere else.
    in_dep_table = False
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("["):
            inner = line.strip("[]").strip()
            table = [seg.strip().strip("\"'") for seg in inner.split(".")]
            # A dotted dependency table names its crate directly:
            # `[dependencies.ratatui]`, `[target.'cfg(unix)'.dev-dependencies.x]`.
            for i, seg in enumerate(table):
                if seg in _DEP_TABLES and i + 1 < len(table):
                    names.add(table[i + 1].replace("-", "_"))
            in_dep_table = any(seg in _DEP_TABLES for seg in table)
            continue
        if not in_dep_table:
            continue
        if table[-1] in _DEP_TABLES:
            # `foo = ...`, `foo.workspace = true`, `"foo" = ...`: the key up to
            # the first dot is the crate name (`foo.workspace` is a key path
            # into the `foo` table, not a crate called `foo.workspace`).
            m = re.match(r'^"?([A-Za-z0-9_\-]+)"?(?:\.[A-Za-z0-9_.\-]+)?\s*=', line)
            if m:
                names.add(m.group(1).replace("-", "_"))
        # A rename applies in an inline table (`foo = { package = "bar" }`)
        # and in the body of a dotted sub-table (`[dependencies.foo]` then
        # `package = "bar"`); the real crate is what gets linked, whatever it
        # is called locally.
        renamed = re.search(r'\bpackage\s*=\s*"([^"]+)"', line)
        if renamed:
            names.add(renamed.group(1).replace("-", "_"))
    return names


def check_workspace_shape(root: Path) -> list[str]:
    """The plugin's inability to self-update or open a TUI is STRUCTURAL (R2).

    "The plugin never self-updates and never opens a TUI" is meant to be a fact
    about what is linked into the binary, not a convention reviewers uphold: a
    mid-session self-update would leave the running binary and the shipped skills
    describing different behaviour, and a hook that opened a prompt would hang the
    harness. `cargo build` can only show that a dependency is PRESENT, so the
    absence is asserted mechanically here.

    `ss-magic-core` is checked for the same three crates because the plugin links
    it: a dependency added there would reach the plugin transitively and the
    guarantee would be gone without the plugin's own manifest changing a line.
    `publish = false` on core is asserted alongside because core is a shared
    implementation detail of two binaries, never a release surface of its own.
    """
    problems: list[str] = []
    for manifest, why in (
        (PLUGIN_MANIFEST, "the plugin must not link it"),
        (CORE_MANIFEST, "the plugin would reach it transitively through core"),
    ):
        declared = _declared_dependency_names(
            _read_text(root, manifest, "a workspace member manifest")
        )
        for dep in FORBIDDEN_PLUGIN_DEPS:
            if dep in declared:
                problems.append(
                    f"{manifest} declares a `{dep}` dependency; {why}. The plugin's "
                    f"inability to self-update or open a TUI is structural, not a "
                    f"convention -- remove the dependency rather than relying on no "
                    f"code calling it."
                )

    core_text = _read_text(root, CORE_MANIFEST, "the core crate manifest")
    if not re.search(r"^\s*publish\s*=\s*false\s*(#.*)?$", core_text, re.MULTILINE):
        problems.append(
            f"{CORE_MANIFEST} does not carry `publish = false`. Core is a shared "
            f"implementation detail of the two binaries and never a release surface "
            f"of its own."
        )
    return problems


def check_pin(root: Path, plugin_dir: Path) -> list[str]:
    """R96/AE82: the committed pin must equal the digest of the tree as it stands."""
    computed = digest_of(build_zip_bytes(plugin_dir))
    entry = _marketplace_entry(root)
    pinned = (entry.get("source") or {}).get("sha256")
    if not isinstance(pinned, str) or not SHA256_RE.match(pinned):
        # check_manifest_keys reports the shape problem; do not double-report.
        return []
    if pinned.lower() != computed:
        return [
            "the committed sha256 does not match the plugin tree:\n"
            f"    committed  {pinned.lower()}\n"
            f"    re-derived {computed}\n"
            "    Run: python3 scripts/build-plugin-zip.py --update-manifest"
        ]
    return []


# --------------------------------------------------------------------------
# R98 / AE81: a content change requires a version bump
# --------------------------------------------------------------------------


def _semver(v: str) -> tuple[int, int, int]:
    return tuple(int(part) for part in v.split("."))  # type: ignore[return-value]


def bump_verdict(
    base_digest: str, base_version: str, cur_digest: str, cur_version: str
) -> str | None:
    """Pure decision behind --check-bump. Returns an error message, or None.

    The resolved version, not the digest, is the update signal: the plugin cache
    path is keyed on the version and `claude plugin update` skips a plugin whose
    resolved version already matches. Publishing new bytes under an unchanged
    version therefore leaves every installed user silently on the cached copy --
    nothing errors, the digest they hold still verifies, and the only symptom is
    that the change never arrives.
    """
    if base_digest == cur_digest:
        return None
    if base_version == cur_version:
        return (
            f"the plugin tree's contents changed since the baseline, but the declared "
            f"version is still {cur_version}.\n"
            f"    baseline digest {base_digest}\n"
            f"    current digest  {cur_digest}\n"
            "    The resolved version is the update signal, not the digest: "
            "`claude plugin update` skips a plugin whose version already matches, so "
            "every installed user would stay silently on the cached copy. Bump the "
            "version on every surface (R95) in the same commit."
        )
    if _semver(cur_version) < _semver(base_version):
        return (
            f"the declared version moved backwards, {base_version} -> {cur_version}. "
            f"Installed users resolve the higher version and would never update."
        )
    return None


def check_bump(root: Path, ref: str) -> list[str]:
    """Compare the current tree against the plugin tree at `ref`."""

    def git(*args: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            ["git", "-C", str(root), *args],
            capture_output=True,
            text=False,
        )

    if git("rev-parse", "--verify", "--quiet", f"{ref}^{{commit}}").returncode != 0:
        print(f"note: baseline ref {ref!r} does not resolve; skipping the bump check.")
        return []
    if git("cat-file", "-e", f"{ref}:plugin").returncode != 0:
        print(f"note: {ref} carries no plugin/ tree; skipping the bump check.")
        return []

    with tempfile.TemporaryDirectory() as tmp:
        archive = git("archive", "--format=tar", f"{ref}:plugin")
        if archive.returncode != 0:
            raise BuildError(
                f"git archive {ref}:plugin failed: "
                f"{archive.stderr.decode('utf-8', 'replace').strip()}"
            )
        extracted = Path(tmp) / "plugin"
        extracted.mkdir()
        untar = subprocess.run(
            ["tar", "-x", "-C", str(extracted)], input=archive.stdout, capture_output=True
        )
        if untar.returncode != 0:
            raise BuildError(
                f"extracting {ref}:plugin failed: "
                f"{untar.stderr.decode('utf-8', 'replace').strip()}"
            )
        base_digest = digest_of(build_zip_bytes(extracted))
        base_version = json.loads(
            (extracted / ".claude-plugin" / "plugin.json").read_text(encoding="utf-8")
        )["version"]

    cur_digest = digest_of(build_zip_bytes(root / "plugin"))
    cur_version = json.loads(
        (root / "plugin" / ".claude-plugin" / "plugin.json").read_text(encoding="utf-8")
    )["version"]

    verdict = bump_verdict(base_digest, base_version, cur_digest, cur_version)
    return [verdict] if verdict else []


# --------------------------------------------------------------------------
# Writing the digest back into the manifest
# --------------------------------------------------------------------------


def update_manifest(root: Path, digest: str) -> bool:
    """Rewrite only the sha256 value, so the file's formatting survives verbatim."""
    path = root / ".claude-plugin" / "marketplace.json"
    text = path.read_text(encoding="utf-8")
    new_text, count = re.subn(
        r'("sha256"\s*:\s*")[0-9a-fA-F]{64}(")', rf"\g<1>{digest}\g<2>", text
    )
    if count != 1:
        raise BuildError(
            f"{path}: expected exactly one 64-hex `sha256` value to rewrite, found "
            f"{count}. Fix the manifest by hand."
        )
    if new_text == text:
        return False
    path.write_text(new_text, encoding="utf-8")
    # Re-read through the JSON parser so a botched substitution cannot land.
    json.loads(path.read_text(encoding="utf-8"))
    return True


# --------------------------------------------------------------------------
# Selftest (AE79, AE80, and the surrounding edge cases)
# --------------------------------------------------------------------------


def _write(path: Path, data: str, mode: int) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(data, encoding="utf-8")
    os.chmod(path, mode)


def _sample_tree(root: Path, *, reverse: bool, mode: int, mtime: float) -> Path:
    """Build a small plugin-shaped tree, perturbing everything that must not matter."""
    plugin = root / "plugin"
    files = [
        (".claude-plugin/plugin.json", '{"name":"ss-magic","version":"9.9.9"}\n'),
        ("ss-magic-plugin.version", "9.9.9\n"),
        ("hooks/hooks.json", '{"hooks":{}}\n'),
        ("hooks/bootstrap.sh", "#!/usr/bin/env bash\nexit 0\n"),
        ("bin/ss-magic-plugin", "#!/usr/bin/env bash\nexit 0\n"),
        # A sibling file whose name sorts against a directory of the same stem;
        # a naive walk that recursed before comparing would order these two
        # differently on different filesystems.
        ("skills.md", "sibling\n"),
        ("skills/scratchpad/SKILL.md", "# scratchpad\n"),
        ("skills/operator-checklist/SKILL.md", "# checklist\n"),
        ("skills/operator-checklist/reference.md", "# reference\n"),
    ]
    for rel, body in reversed(files) if reverse else files:
        _write(plugin / rel, body, mode)
        os.utime(plugin / rel, (mtime, mtime))
    return plugin


_HOOK_GOOD = {
    "type": "command",
    "command": "bash",
    "args": ["${CLAUDE_PLUGIN_ROOT}/hooks/run-hook.sh", "pre-tool-use"],
    "timeout": 5,
}
# The regression the shim exists to prevent: the binary is absent until the
# SessionStart bootstrap fetches it, so this spelling ENOENTs a first session.
_HOOK_BAD = {
    "type": "command",
    "command": "${CLAUDE_PLUGIN_DATA}/bin/ss-magic-plugin",
    "args": ["hook", "pre-tool-use"],
    "timeout": 5,
}


def _version_repo(
    root: Path,
    *,
    cli: str = "0.11.1",
    plugin: str = "1.0.0",
    readme_pin: str | None = None,
    bad_hook: bool = False,
    plugin_deps: str = "",
    artifact_in_dist: bool = False,
) -> Path:
    """A minimal repository root carrying every surface `check_versions` reads."""
    pin = f"v{cli}" if readme_pin is None else readme_pin
    artifact = f'[[package.metadata.dist.extra-artifacts]]\nartifacts = ["ss-magic-plugin-v{plugin}.zip"]\n'

    (root / "crates" / "ss-magic").mkdir(parents=True)
    (root / "crates" / "ss-magic" / "Cargo.toml").write_text(
        f'[package]\nname = "ss-magic"\nversion = "{cli}"\n', encoding="utf-8"
    )
    (root / "crates" / "ss-magic-plugin").mkdir(parents=True)
    (root / "crates" / "ss-magic-plugin" / "Cargo.toml").write_text(
        f'[package]\nname = "ss-magic-plugin"\nversion = "{plugin}"\n\n'
        # Prose naming all three forbidden crates, so the guard is proved to read
        # dependency tables rather than to grep the file.
        "# No self_update, no inquire, no ratatui: absent by requirement.\n"
        '[dependencies]\nss-magic-core = { path = "../ss-magic-core" }\n'
        f"{plugin_deps}"
        + ("" if artifact_in_dist else f"\n{artifact}"),
        encoding="utf-8",
    )
    (root / "crates" / "ss-magic-core").mkdir(parents=True)
    (root / "crates" / "ss-magic-core" / "Cargo.toml").write_text(
        '[package]\nname = "ss-magic-core"\nversion = "0.1.0"\npublish = false\n\n'
        '[dependencies]\nanyhow = "1"\n',
        encoding="utf-8",
    )
    (root / "Cargo.lock").write_text(
        '[[package]]\nname = "ss-magic"\nversion = "%s"\n\n'
        '[[package]]\nname = "ss-magic-core"\nversion = "0.1.0"\n\n'
        '[[package]]\nname = "ss-magic-plugin"\nversion = "%s"\n' % (cli, plugin),
        encoding="utf-8",
    )
    (root / "dist-workspace.toml").write_text(
        "[dist]\nci = \"github\"\n" + (f"\n{artifact}" if artifact_in_dist else ""),
        encoding="utf-8",
    )
    (root / "README.md").write_text(
        "curl -sSfL https://github.com/o/r/releases/download/"
        f"{pin}/ss-magic-installer.sh | sh\n"
        if pin
        else "no installer documented here\n",
        encoding="utf-8",
    )

    (root / "plugin" / ".claude-plugin").mkdir(parents=True)
    (root / "plugin" / ".claude-plugin" / "plugin.json").write_text(
        json.dumps({"name": "ss-magic", "version": plugin}) + "\n", encoding="utf-8"
    )
    (root / "plugin" / "ss-magic-plugin.version").write_text(f"{plugin}\n", encoding="utf-8")
    (root / "plugin" / "hooks").mkdir(parents=True)
    (root / "plugin" / "hooks" / "hooks.json").write_text(
        json.dumps(
            {
                "hooks": {
                    "SessionStart": [
                        {
                            "matcher": "startup",
                            "hooks": [
                                {
                                    "type": "command",
                                    "command": "bash",
                                    "args": ["${CLAUDE_PLUGIN_ROOT}/hooks/bootstrap.sh"],
                                    "timeout": 90,
                                }
                            ],
                        }
                    ],
                    "PreToolUse": [
                        {"matcher": "Read", "hooks": [_HOOK_BAD if bad_hook else _HOOK_GOOD]}
                    ],
                }
            }
        ),
        encoding="utf-8",
    )

    (root / ".claude-plugin").mkdir(parents=True)
    (root / ".claude-plugin" / "marketplace.json").write_text(
        json.dumps(
            {
                "plugins": [
                    {
                        "name": "ss-magic",
                        "source": {
                            "source": "archive",
                            "url": (
                                "https://github.com/o/r/releases/download/"
                                f"ss-magic-plugin-v{plugin}/ss-magic-plugin-v{plugin}.zip"
                            ),
                            "sha256": "0" * 64,
                        },
                    }
                ]
            }
        ),
        encoding="utf-8",
    )
    return root


def _version_problems(root: Path, fragment: str) -> list[str]:
    """The problems `check_versions` reports under the label containing `fragment`."""
    return [p for label, found in check_versions(root) for p in found if fragment in label]


def _expect_refusal(fn, needle: str, label: str) -> None:
    try:
        fn()
    except BuildError as exc:
        if needle not in str(exc):
            raise AssertionError(
                f"{label}: refusal did not mention {needle!r}: {exc}"
            ) from None
        return
    raise AssertionError(f"{label}: expected a BuildError, got none")


def selftest() -> int:
    checks: list[str] = []

    # -- AE79: two builds, perturbed in every way that must not reach the bytes.
    with tempfile.TemporaryDirectory() as a, tempfile.TemporaryDirectory() as b:
        old_umask = os.umask(0o077)
        try:
            tree_a = _sample_tree(Path(a), reverse=False, mode=0o600, mtime=1)
            digest_a = digest_of(build_zip_bytes(tree_a))
        finally:
            os.umask(0o022)
        try:
            tree_b = _sample_tree(Path(b), reverse=True, mode=0o777, mtime=2_000_000_000)
            digest_b = digest_of(build_zip_bytes(tree_b))
        finally:
            os.umask(old_umask)
        assert digest_a == digest_b, f"AE79: {digest_a} != {digest_b}"
        checks.append("AE79 digest stable across umask, mode, mtime and creation order")

        data = build_zip_bytes(tree_a)
        with zipfile.ZipFile(io.BytesIO(data)) as zf:
            infos = zf.infolist()
            names = [i.filename for i in infos]
            assert names == sorted(names), f"AE79: entries not sorted: {names}"
            assert not any(n.endswith("/") for n in names), "AE79: directory entry emitted"
            for info in infos:
                assert info.compress_type == zipfile.ZIP_STORED, f"{info.filename}: deflated"
                assert info.date_time == FIXED_DATE_TIME, f"{info.filename}: {info.date_time}"
                assert info.create_system == UNIX_CREATE_SYSTEM, f"{info.filename}: create_system"
                got = (info.external_attr >> 16) & 0o7777
                want = mode_for(info.filename)
                assert got == want, f"{info.filename}: mode {got:o} != {want:o}"
            assert (
                (dict((i.filename, (i.external_attr >> 16) & 0o7777) for i in infos))[
                    "bin/ss-magic-plugin"
                ]
                == MODE_EXEC
            ), "bin/ entries must be 0755 (the wrapper lands there)"
        checks.append("AE79 stored-only, 1980 stamps, unix create_system, normalised modes")

    # -- Excluded and inert inputs must not move the digest.
    with tempfile.TemporaryDirectory() as c:
        tree = _sample_tree(Path(c), reverse=False, mode=0o644, mtime=1)
        before = digest_of(build_zip_bytes(tree))
        (tree / ".DS_Store").write_bytes(b"\x00\x01junk")
        (tree / "skills" / ".DS_Store").write_bytes(b"\x00\x02junk")
        assert digest_of(build_zip_bytes(tree)) == before, ".DS_Store reached the digest"
        (tree / "empty-dir").mkdir()
        (tree / "skills" / "empty-nested").mkdir()
        assert digest_of(build_zip_bytes(tree)) == before, "an empty directory reached the digest"
        checks.append(".DS_Store and empty directories excluded from the digest")

    # -- AE80: loud refusals.
    with tempfile.TemporaryDirectory() as d:
        tree = _sample_tree(Path(d), reverse=False, mode=0o644, mtime=1)
        bad = tree / "skills" / "café.md"
        bad.write_text("x\n", encoding="utf-8")
        _expect_refusal(lambda: build_zip_bytes(tree), "non-ASCII", "AE80 non-ASCII file")
        bad.unlink()
        (tree / "skills" / "link.md").symlink_to(tree / "skills.md")
        _expect_refusal(lambda: build_zip_bytes(tree), "symlink", "AE80 symlink")
        (tree / "skills" / "link.md").unlink()
        (tree / "linkdir").symlink_to(tree / "skills", target_is_directory=True)
        _expect_refusal(lambda: build_zip_bytes(tree), "symlink", "AE80 symlinked directory")
        (tree / "linkdir").unlink()
        checks.append("AE80 refuses a non-ASCII name, a symlink, and a symlinked directory")

    # -- A missing tree is a clear error, not a traceback or an empty archive.
    with tempfile.TemporaryDirectory() as e:
        _expect_refusal(
            lambda: build_zip_bytes(Path(e) / "plugin"), "does not exist", "missing plugin dir"
        )
        empty = Path(e) / "empty"
        empty.mkdir()
        _expect_refusal(
            lambda: build_zip_bytes(empty), "no packageable files", "empty plugin dir"
        )
        checks.append("a missing or empty plugin tree is a named refusal")

    # -- AE81: the bump decision, as a truth table.
    assert bump_verdict("aa", "1.0.0", "aa", "1.0.0") is None
    assert bump_verdict("aa", "1.0.0", "aa", "1.1.0") is None
    assert bump_verdict("aa", "1.0.0", "bb", "1.0.1") is None
    verdict = bump_verdict("aa", "1.0.0", "bb", "1.0.0")
    assert verdict is not None and "still 1.0.0" in verdict, verdict
    backwards = bump_verdict("aa", "1.1.0", "bb", "1.0.0")
    assert backwards is not None and "backwards" in backwards, backwards
    checks.append("AE81 content change without a version bump is rejected")

    # -- R95: two release lines, grouped, plus the "must differ" rule.
    with tempfile.TemporaryDirectory() as f:
        root = _version_repo(Path(f) / "agree")
        results = check_versions(root)
        labels = [label for label, _ in results]
        assert labels == [
            "R95 version surfaces (ss-magic)",
            "R95 version surfaces (ss-magic-plugin)",
            "distinct release lines",
        ], labels
        assert all(not found for _, found in results), results
        groups = version_surfaces(root)
        assert set(groups) == {CLI_LINE, PLUGIN_LINE}, groups
        assert str(PLUGIN_MANIFEST) in groups[PLUGIN_LINE], groups
        assert default_out_path(root).name == "ss-magic-plugin-v1.0.0.zip", default_out_path(root)
        checks.append("R95 two agreeing release lines pass, and each reports separately")

    with tempfile.TemporaryDirectory() as f:
        root = _version_repo(Path(f), cli="0.11.1", plugin="0.11.1")
        found = _version_problems(root, "distinct release lines")
        assert found and "0.11.1" in found[0] and "PACKAGE_NAME" in found[0], found
        # The groups themselves are still internally consistent; only (b) fails.
        assert not _version_problems(root, f"({PLUGIN_LINE})"), "the plugin group should agree"
        checks.append("R95 equal versions on the two lines is rejected (one tag would release both)")

    with tempfile.TemporaryDirectory() as f:
        root = _version_repo(Path(f), cli="0.11.1", plugin="0.11.2")
        found = _version_problems(root, f"({PLUGIN_LINE})")
        assert not found, found
        (root / "plugin" / "ss-magic-plugin.version").write_text("0.11.3\n", encoding="utf-8")
        found = _version_problems(root, f"({PLUGIN_LINE})")
        assert found and "0.11.3" in found[0], found
        checks.append("R95 a disagreeing surface inside one line is reported against that line")

    # A crate version that is not a bare triple must be a named refusal, not the
    # ValueError the `<=` comparison would otherwise raise on it.
    with tempfile.TemporaryDirectory() as f:
        root = _version_repo(Path(f), cli="0.11.1")
        (root / CLI_MANIFEST).write_text(
            '[package]\nname = "ss-magic"\nversion = "0.11.1-rc1"\n', encoding="utf-8"
        )
        found = _version_problems(root, f"({CLI_LINE})")
        assert any("MAJOR.MINOR.PATCH" in p for p in found), found
        checks.append("a crate version that is not a bare triple is a named refusal")

    # -- The README installer pin is `<=` the crate version, never `==`.
    with tempfile.TemporaryDirectory() as f:
        for pin, ok, note in (
            ("v0.11.1", True, "equal to the crate version"),
            ("v0.11.0", True, "lagging (bumped, not yet tagged)"),
            ("v0.9.0", True, "lagging across a lexically larger minor"),
            ("v0.11.2", False, "ahead of the crate version"),
            ("v0.12.0", False, "ahead across a minor"),
            ("0.11.1", False, "missing the v prefix"),
            ("v0.11", False, "not a full MAJOR.MINOR.PATCH"),
            ("latest", False, "not a version at all"),
            ("", False, "no pinned installer release documented"),
        ):
            target = Path(f) / f"pin-{pin or 'none'}"
            target.mkdir()
            root = _version_repo(target, cli="0.11.1", readme_pin=pin)
            found = _version_problems(root, f"({CLI_LINE})")
            assert bool(found) != ok, f"README pin {pin!r} ({note}): {found}"
        checks.append("README installer pin may lag the crate version but never lead it")

    # -- The hook manifest must never name the binary.
    with tempfile.TemporaryDirectory() as f:
        good = _version_repo(Path(f) / "good")
        assert not check_hooks_shim(good), check_hooks_shim(good)
        bad = _version_repo(Path(f) / "bad", bad_hook=True)
        found = check_hooks_shim(bad)
        assert found and "ss-magic-plugin" in found[0] and "ENOENT" in found[0], found
        checks.append("a hook entry naming the binary instead of the shim is rejected")

    # -- The plugin must not link self_update / inquire / ratatui, directly or
    #    through core.
    with tempfile.TemporaryDirectory() as f:
        clean = _version_repo(Path(f) / "clean")
        assert not check_workspace_shape(clean), check_workspace_shape(clean)
        for label, deps in (
            ("self_update", 'self_update = { version = "0.44.0" }\n'),
            ("inquire", 'inquire = "0.9"\n'),
            ("ratatui", 'ratatui = "0.30.2"\n'),
            ("a renamed self_update", 'tui = { package = "self_update", version = "0.44" }\n'),
            # A dotted sub-table rename: the crate name is in the body, not the header.
            ("a dotted-table rename", '\n[dependencies.tui]\npackage = "self_update"\nversion = "0.44"\n'),
            # A workspace-inherited dependency: the key path is `ratatui.workspace`.
            ("a workspace key", 'ratatui.workspace = true\n'),
        ):
            target = Path(f) / f"dep-{label.replace(' ', '-')}"
            target.mkdir()
            dirty = _version_repo(target, plugin_deps=deps)
            found = check_workspace_shape(dirty)
            assert found and "structural" in found[0], f"{label}: {found}"
        core_only = _version_repo(Path(f) / "core")
        core_manifest = core_only / CORE_MANIFEST
        core_manifest.write_text(
            core_manifest.read_text(encoding="utf-8")
            .replace("publish = false\n", "")
            .replace('anyhow = "1"', 'anyhow = "1"\ninquire = "0.9"'),
            encoding="utf-8",
        )
        found = check_workspace_shape(core_only)
        assert len(found) == 2, found  # the transitive dependency AND publish = false
        assert any("transitively" in p for p in found), found
        assert any("publish = false" in p for p in found), found
        checks.append("workspace shape rejects self_update/inquire/ratatui and a publishable core")

    # -- The extra-artifact surface falls back to dist-workspace.toml.
    with tempfile.TemporaryDirectory() as f:
        fallback = _version_repo(Path(f) / "fallback", artifact_in_dist=True)
        surfaces = version_surfaces(fallback)[PLUGIN_LINE]
        assert "dist-workspace.toml artifact #1" in surfaces, surfaces
        assert not _version_problems(fallback, f"({PLUGIN_LINE})")
        bare = _version_repo(Path(f) / "bare")
        (bare / "dist-workspace.toml").write_text("[dist]\n", encoding="utf-8")
        (bare / PLUGIN_MANIFEST).write_text(
            '[package]\nname = "ss-magic-plugin"\nversion = "1.0.0"\n', encoding="utf-8"
        )
        _expect_refusal(
            lambda: version_surfaces(bare), "would not be published", "no extra-artifact anywhere"
        )
        checks.append("the extra-artifact filename is read from the plugin crate, then dist-workspace")

    for line in checks:
        print(f"ok   {line}")
    print(f"\nselftest: {len(checks)} checks passed")
    return 0



# --------------------------------------------------------------------------
# Entry point
# --------------------------------------------------------------------------


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Build the reproducible ss-magic plugin zip, and check its pin."
    )
    parser.add_argument(
        "--root", type=Path, default=REPO_ROOT, help="repository root (default: this script's)"
    )
    parser.add_argument(
        "--plugin-dir", type=Path, default=None, help="tree to package (default: <root>/plugin)"
    )
    parser.add_argument("--out", type=Path, default=None, help="write the zip here")
    parser.add_argument(
        "--print-digest", action="store_true", help="print only the digest; write nothing"
    )
    parser.add_argument(
        "--update-manifest",
        action="store_true",
        help="write the computed digest into .claude-plugin/marketplace.json",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help=(
            "assert the sha256 key (R101), both release lines' version surfaces and the "
            "README installer pin (R95), that the two lines' versions differ, that every "
            "hook spawns the shim, the workspace's forbidden-dependency shape (R2), and "
            "the committed digest pin (R96)"
        ),
    )
    parser.add_argument(
        "--check-bump",
        metavar="REF",
        default=None,
        help="assert a content change since REF came with a version bump (R98)",
    )
    parser.add_argument("--selftest", action="store_true", help="run the builder's own tests")
    args = parser.parse_args(argv)

    root: Path = args.root.resolve()
    plugin_dir: Path = (args.plugin_dir or (root / "plugin")).resolve()

    try:
        if args.selftest:
            return selftest()

        if args.check or args.check_bump:
            problems: list[str] = []

            def record(name: str, found: list[str]) -> None:
                problems.extend(f"{name}: {p}" for p in found)
                print(f"{'FAIL' if found else 'ok  '} {name}")

            if args.check:
                # Run and print in sequence rather than collecting first, so a
                # refusal raised by a later check still leaves the earlier ok/FAIL
                # lines in CI's log.
                record("R101 marketplace sha256 key", check_manifest_keys(root))
                for name, found in check_versions(root):
                    record(name, found)
                record("hooks spawn through the shim", check_hooks_shim(root))
                record("workspace shape", check_workspace_shape(root))
                record("R96 committed digest pin", check_pin(root, plugin_dir))
            if args.check_bump:
                found = check_bump(root, args.check_bump)
                problems.extend(f"R98 version bump vs {args.check_bump}: {p}" for p in found)
                print(f"{'FAIL' if found else 'ok  '} R98 version bump vs {args.check_bump}")
            if problems:
                # Flush first so the ok/FAIL lines and the detail below them land
                # in CI's log in the order they were written.
                sys.stdout.flush()
                print("", file=sys.stderr)
                for problem in problems:
                    print(f"error: {problem}", file=sys.stderr)
                return 1
            return 0

        data = build_zip_bytes(plugin_dir)
        digest = digest_of(data)

        if args.update_manifest:
            changed = update_manifest(root, digest)
            print(
                f"{'updated' if changed else 'unchanged'} .claude-plugin/marketplace.json "
                f"sha256 = {digest}"
            )

        if args.print_digest and args.out is None:
            print(digest)
            return 0

        out = args.out.resolve() if args.out else default_out_path(root)
        out.parent.mkdir(parents=True, exist_ok=True)
        # Write via a temporary file in the same directory, then replace, so an
        # interrupted build never leaves a truncated archive behind.
        fd, tmp_name = tempfile.mkstemp(dir=str(out.parent), suffix=".zip.tmp")
        try:
            with os.fdopen(fd, "wb") as handle:
                handle.write(data)
            os.replace(tmp_name, out)
        except BaseException:
            if os.path.exists(tmp_name):
                os.unlink(tmp_name)
            raise
        print(f"{digest}  {out}")
        return 0
    except BuildError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2
    except AssertionError as exc:
        print(f"selftest failure: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
