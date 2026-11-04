#!/usr/bin/env bash
# docs-lint (FR-7.2 / T-49): ban mock/fixture/sample literals in RUNTIME code.
# Legal fixtures live ONLY in tests/ or #[cfg(test)] modules / test files:
#   * crates/*/tests/** — integration tests + fixtures corpus
#   * web: no occurrences at all outside negation comments
#
# Rule: a line counts as a violation when it contains mock|fixture|sample
# WITHOUT a negation ("no mock", "not.*mock", "never", "0-mock", "no fake",
# "mock-free") on the same line. Runtime .rs files are additionally allowed to
# mention fixtures inside #[cfg(test)]-only blocks — the pragmatic check: a hit
# line must ALSO carry cfg(test) / test module markers, else violation.
set -euo pipefail

cd "$(dirname "$0")/.."

violations=0
check_file() {
  local f="$1"
  # grep -v filters my own rule's negation comments; remaining hits violate.
  local hits
  hits=$(grep -niE 'mock|fixture|sample' "$f" 2>/dev/null \
    | grep -viE 'no mock|not[ -]mock|0-mock|mock-free|no fake|never|without (any )?mock|FR-7\.2|grep -niE|grep -v|iE .mock' \
    || true)
  if [ -n "$hits" ]; then
    echo "VIOLATION $f:"
    echo "$hits"
    violations=$((violations + 1))
  fi
}

# Runtime rust sources (tests/ dirs are fixtures-legal by construction).
for f in $(find crates/*/src -name '*.rs' 2>/dev/null); do
  check_file "$f"
done

# Web runtime (page components, libs, routes).
for f in $(find web/src -name '*.ts' -o -name '*.tsx' -o -name '*.css' 2>/dev/null); do
  check_file "$f"
done

# Shell/compose runtime.
for f in $(find docker scripts -type f 2>/dev/null); do
  check_file "$f"
done

if [ "$violations" -gt 0 ]; then
  echo "docs-lint FAILED: $violations file(s) with mock/fixture/sample literals in runtime paths"
  exit 1
fi
echo "docs-lint OK: no mock/fixture/sample literals in runtime paths"
