#!/usr/bin/env python3
"""Mechanical cross-cutting index for the simplify sweep (run from the repository root): every const/static,
every repeated string literal, every large numeric literal, across Rust, shell
and Python. Precise context so no reviewer has to grep the tree itself."""
import re
from collections import defaultdict
from pathlib import Path

REPO = Path(".")
OUT = REPO / "docs/plans/2026-09-10-001-refactor-simplify-sweep/index"

def source_files():
    for base in ("crates", "scripts", "plugin"):
        for p in sorted((REPO / base).rglob("*")):
            if p.is_file() and p.suffix in (".rs", ".py", ".sh") and "target" not in p.parts:
                yield p
    for p in ("plugin/bin/ss-magic-plugin", "plugin/hooks/run-hook.sh"):
        q = REPO / p
        if q.is_file():
            yield q

files = list(dict.fromkeys(source_files()))
is_test = lambda p: p.name == "tests.rs" or "/tests/" in str(p) or p.name.startswith("test-") or p.name == "test-harness.sh"

# ── consts / statics ────────────────────────────────────────────────────────
const_re = re.compile(r'^\s*(pub(?:\([^)]*\))?\s+)?(const|static)\s+([A-Z_][A-Z0-9_]*)\s*:\s*([^=]+?)\s*=\s*(.*)$')
consts = []
for p in files:
    if p.suffix != ".rs":
        continue
    for i, line in enumerate(p.read_text(errors="replace").splitlines(), 1):
        m = const_re.match(line)
        if m:
            consts.append({"file": str(p), "line": i, "vis": (m.group(1) or "").strip() or "private",
                           "kind": m.group(2), "name": m.group(3), "type": m.group(4).strip(),
                           "value": m.group(5).strip().rstrip(";")[:120], "test": is_test(p)})
by_name = defaultdict(list)
for c in consts:
    by_name[c["name"]].append(c)

with open(OUT / "consts.md", "w") as f:
    f.write("# Constants and statics (all crates)\n\nOne row per declaration. `dup-name` marks a name declared in more than one file.\n\n")
    f.write("| name | file:line | vis | type | value | flags |\n|---|---|---|---|---|---|\n")
    for c in sorted(consts, key=lambda c: (c["name"], c["file"])):
        flags = []
        if len(by_name[c["name"]]) > 1: flags.append("dup-name")
        if c["test"]: flags.append("test")
        f.write(f"| `{c['name']}` | `{c['file']}:{c['line']}` | {c['vis']} | `{c['type']}` | `{c['value'].replace('|','\\|')}` | {' '.join(flags)} |\n")

# ── string literals ─────────────────────────────────────────────────────────
str_re = re.compile(r'"((?:[^"\\\n]|\\.){3,})"')
sh_re = re.compile(r"'([^'\n]{3,})'")
lits = defaultdict(lambda: defaultdict(list))  # literal -> file -> [lines]
for p in files:
    text = p.read_text(errors="replace")
    for i, line in enumerate(text.splitlines(), 1):
        s = line.strip()
        if p.suffix == ".rs" and (s.startswith("//") or s.startswith("///") or s.startswith("//!")):
            continue
        if p.suffix in (".sh", ".py", "") and s.startswith("#"):
            continue
        for m in str_re.finditer(line):
            lits[m.group(1)][str(p)].append(i)
        if p.suffix in (".sh", ".py", ""):
            for m in sh_re.finditer(line):
                lits[m.group(1)][str(p)].append(i)

def interesting(s):
    if len(s) < 3: return False
    if s.count(" ") >= 4: return False  # prose, not an identifier-ish value
    return any(ch in s for ch in "/._-:{}$") or s.isupper() or len(s) >= 8

rows = []
for s, per_file in lits.items():
    if not interesting(s): continue
    nfiles = len(per_file); total = sum(len(v) for v in per_file.values())
    non_test_files = [f for f in per_file if not is_test(Path(f))]
    if nfiles >= 2 or total >= 3:
        rows.append((s, nfiles, total, len(non_test_files), per_file))
rows.sort(key=lambda r: (-r[3], -r[1], -r[2], r[0]))
with open(OUT / "literals.md", "w") as f:
    f.write("# Repeated string literals (Rust, shell, Python)\n\nLiterals that look like names, paths, keys or flags and occur in 2+ files or 3+ times. Comments excluded. `nt-files` = files that are not tests. Sorted by non-test file spread.\n\n")
    f.write("| literal | nt-files | files | total | where |\n|---|---|---|---|---|\n")
    for s, nfiles, total, nt, per_file in rows:
        where = "; ".join(f"`{Path(fp).name}`:{','.join(map(str, ls[:6]))}{'…' if len(ls) > 6 else ''}" for fp, ls in sorted(per_file.items()))
        f.write(f"| `{s.replace('|','\\|')}` | {nt} | {nfiles} | {total} | {where} |\n")

# ── numeric literals ────────────────────────────────────────────────────────
num_re = re.compile(r'(?<![\w.])(\d[\d_]*\d|\d)(?:\s*\*\s*\d+)?(?![\w.])')
nums = defaultdict(list)
for p in files:
    if p.suffix != ".rs" or is_test(p): continue
    for i, line in enumerate(p.read_text(errors="replace").splitlines(), 1):
        s = line.strip()
        if s.startswith("//"): continue
        if const_re.match(line): continue  # declared consts are in consts.md
        for m in num_re.finditer(line):
            v = int(m.group(1).replace("_", ""))
            if v >= 60:
                nums[v].append((str(p), i, s[:110]))
with open(OUT / "numbers.md", "w") as f:
    f.write("# Bare numeric literals >= 60 in non-test Rust (outside const declarations)\n\nCandidates for a named constant, or for reuse of one that already exists (see consts.md).\n\n")
    for v in sorted(nums):
        f.write(f"## {v}\n\n")
        for fp, i, s in nums[v]:
            f.write(f"- `{fp}:{i}` – `{s.replace('`','')}`\n")
        f.write("\n")

# ── function signature index (for cross-module duplication leads) ───────────
fn_re = re.compile(r'^\s*(pub(?:\([^)]*\))?\s+)?fn\s+([a-z_][a-z0-9_]*)\s*(<[^>]*>)?\s*\(([^)]*)\)\s*(->\s*[^{]+)?')
fns = []
for p in files:
    if p.suffix != ".rs" or is_test(p): continue
    for i, line in enumerate(p.read_text(errors="replace").splitlines(), 1):
        m = fn_re.match(line)
        if m:
            fns.append({"name": m.group(2), "file": str(p), "line": i, "vis": (m.group(1) or "").strip() or "private",
                        "params": " ".join(m.group(4).split())[:100], "ret": (m.group(5) or "").strip()[:60]})
by_fn = defaultdict(list)
for fn in fns: by_fn[fn["name"]].append(fn)
with open(OUT / "functions.md", "w") as f:
    f.write("# Function names declared in more than one non-test file\n\nSame name in two places is a lead for duplicated logic, or for a name that means two things.\n\n")
    for name, decls in sorted(by_fn.items()):
        if len(decls) < 2: continue
        f.write(f"## `{name}`\n\n")
        for d in decls:
            f.write(f"- `{d['file']}:{d['line']}` {d['vis']} `({d['params']}) {d['ret']}`\n")
        f.write("\n")

print("consts", len(consts), "| repeated literals", len(rows), "| numeric values", len(nums), "| fn names in >1 file", sum(1 for d in by_fn.values() if len(d) > 1))
