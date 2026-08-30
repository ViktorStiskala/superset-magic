#!/usr/bin/env python3
"""Deterministic builder for the ss-magic plugin zip (R96), plus the release
assertions CI runs against it (R95, R98, R101).

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
    build-plugin-zip.py --check               # the three release assertions
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

REPO_ROOT = Path(__file__).resolve().parent.parent


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


def _dist_artifact_versions(path: Path) -> list[str]:
    return ARTIFACT_RE.findall(path.read_text(encoding="utf-8"))


def version_surfaces(root: Path) -> dict[str, str]:
    """Every place a release has to advance together (R95)."""
    surfaces: dict[str, str] = {}
    surfaces["Cargo.toml"] = _cargo_toml_version(root / "Cargo.toml")
    surfaces["Cargo.lock"] = _cargo_lock_version(root / "Cargo.lock", "ss-magic")

    plugin_manifest = json.loads(
        (root / "plugin" / ".claude-plugin" / "plugin.json").read_text(encoding="utf-8")
    )
    surfaces["plugin/.claude-plugin/plugin.json"] = plugin_manifest.get("version", "")

    pin = (root / "plugin" / "ss-magic.version").read_text(encoding="utf-8").strip()
    if not VERSION_RE.match(pin):
        raise BuildError(
            f"plugin/ss-magic.version: {pin!r} is not a bare MAJOR.MINOR.PATCH literal. "
            f"The bootstrap interpolates this value into a release URL, so it is "
            f"validated before use and must not carry a `v` prefix or trailing text."
        )
    surfaces["plugin/ss-magic.version"] = pin

    entry = _marketplace_entry(root)
    url = (entry.get("source") or {}).get("url", "")
    tag = re.search(r"/download/v(\d+\.\d+\.\d+)/", url)
    if not tag:
        raise BuildError(
            f".claude-plugin/marketplace.json: cannot read a release tag out of the "
            f"archive url {url!r}; expected .../releases/download/vX.Y.Z/..."
        )
    surfaces["marketplace.json url (tag)"] = tag.group(1)
    asset = ARTIFACT_RE.search(url)
    if not asset:
        raise BuildError(
            f".claude-plugin/marketplace.json: the archive url {url!r} does not name a "
            f"ss-magic-plugin-vX.Y.Z.zip asset. Per-version asset filenames are what "
            f"stop a later release from overwriting an earlier pin's target."
        )
    surfaces["marketplace.json url (asset)"] = asset.group(1)

    dist_versions = _dist_artifact_versions(root / "dist-workspace.toml")
    if not dist_versions:
        raise BuildError(
            "dist-workspace.toml: no [[dist.extra-artifacts]] entry naming "
            "ss-magic-plugin-vX.Y.Z.zip; the plugin zip would not be published."
        )
    for i, v in enumerate(dist_versions):
        surfaces[f"dist-workspace.toml artifact #{i + 1}"] = v
    return surfaces


def default_out_path(root: Path) -> Path:
    """Derive the asset filename from the crate version rather than hand-editing it."""
    return root / f"ss-magic-plugin-v{_cargo_toml_version(root / 'Cargo.toml')}.zip"


# --------------------------------------------------------------------------
# The three release assertions CI runs
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


def check_versions(root: Path) -> list[str]:
    """R95: every version surface a release advances must already agree."""
    surfaces = version_surfaces(root)
    distinct = sorted(set(surfaces.values()))
    if len(distinct) == 1:
        return []
    lines = [f"version surfaces disagree ({', '.join(distinct)}):"]
    for name, value in surfaces.items():
        lines.append(f"    {value:<12} {name}")
    return ["\n".join(lines)]


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
        ("ss-magic.version", "9.9.9\n"),
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
        help="assert the committed pin, the version surfaces, and the sha256 key (R95, R96, R101)",
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
            if args.check:
                for name, fn in (
                    ("R101 marketplace sha256 key", lambda: check_manifest_keys(root)),
                    ("R95 version surfaces", lambda: check_versions(root)),
                    ("R96 committed digest pin", lambda: check_pin(root, plugin_dir)),
                ):
                    found = fn()
                    problems.extend(f"{name}: {p}" for p in found)
                    print(f"{'FAIL' if found else 'ok  '} {name}")
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
