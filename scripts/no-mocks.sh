#!/usr/bin/env bash
# docs-lint (FR-7.2 / T-49): ban mock/fixture/sample literals in RUNTIME code.
# Fixtures are legal ONLY in:
#   * crates/*/tests/** (integration tests + fixtures corpus)
#   * .rs files in a #[cfg(test)] test module (conventionally the bottom of the
#     file) — a hit line must sit BELOW the file's first #[cfg(test)] directive
#     that begins a `mod` (python3 scope check below)
#   * negation comments ("no mock", "0-mock", "without mock", …) — self-
#     describing rules/bans are not fixture usage
#
# Output: violation list; exit 1 if any.
set -euo pipefail
cd "$(dirname "$0")/.."

python3 - <<'PY'
import os, re, sys

viols = []
neg = re.compile(r"no[ _-]mock|not[ -]mock|0-mock|mock-free|no fake|never|without (any )?mock|no mock/fixture", re.I)
tok = re.compile(r"mock|fixture|sample", re.I)

def sweep_rs(path):
    lines = open(path, encoding="utf-8", errors="replace").read().splitlines()
    # first #[cfg(test)] that is (or is followed by) a mod — test zone starts there
    test_zone = len(lines) + 1
    for i, ln in enumerate(lines):
        if "#[cfg(test)]" in ln:
            nxt = lines[i + 1] if i + 1 < len(lines) else ""
            if re.search(r"mod\s+\w+", nxt) or re.search(r"mod\s", ln):
                test_zone = i + 1
                break
    for i, ln in enumerate(lines, 1):
        if tok.search(ln) and not neg.search(ln) and i < test_zone:
            viols.append(f"{path}:{i}: {ln.strip()[:120]}")

def sweep_other(path):
    for i, ln in enumerate(open(path, encoding="utf-8", errors="replace").read().splitlines(), 1):
        if tok.search(ln) and not neg.search(ln):
            viols.append(f"{path}:{i}: {ln.strip()[:120]}")

for root in ("crates", "web/src", "docker", "scripts"):
    for dirpath, _, files in os.walk(root):
        if "/tests" in dirpath or dirpath.endswith("/tests") or "fixtures" in dirpath:
            continue
        for fn in files:
            p = os.path.join(dirpath, fn)
            if fn == "no-mocks.sh":
                # the linter's own rule text is exempt (it would self-match)
                continue
            if fn.endswith(".rs"):
                sweep_rs(p)
            elif fn.endswith((".ts", ".tsx", ".css", ".sh", ".yml", ".yaml")) or fn.startswith("Dockerfile"):
                sweep_other(p)

if viols:
    print("docs-lint FAILED: mock/fixture/sample literals outside fixture-legal zones:")
    for v in viols:
        print(" ", v)
    sys.exit(1)
print("docs-lint OK: no mock/fixture/sample literals in runtime paths")
PY
