#!/usr/bin/env bash
# tests/scripts/hygiene_check.sh
# Repository Hygiene & Anti-Smell Verification Gate
# Enforces:
# 1. Zero personal paths (/Users/, /home/<user>/, /Workspace/) & secret patterns in tracked text files.
# 2. Zero Unicode emojis & sparkline/bar characters (except deliberate '⚡ eg:' MCP KPI footer token).
# 3. Rust Code-Smells Gate:
#    - Hard fail on ANY .rs file > 500 lines (Zero exception / Zero allowlist).
#    - Forbids 'unsafe { ... }', 'unsafe fn', 'unsafe trait', 'unsafe impl' blocks in src/.
#    - Forbids stray 'dbg!' macros in src/.

set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

# Target files: if arguments passed, use them; otherwise scan git tracked files.
if [ "$#" -gt 0 ]; then
  FILES=("$@")
else
  # Read tracked files into array safely
  FILES=()
  while IFS= read -r f; do
    [ -n "$f" ] && FILES+=("$f")
  done < <(git ls-files)
fi

ERRORS=0

# 1. Zero Private Leakage Check
LEAK_PATTERNS='(/Users/|/home/[A-Za-z0-9._-]+/|/Workspace/|quy\.doan@|@be\.com\.vn|(AKIA|ASIA)[0-9A-Z]{16}|gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{22,}|-----BEGIN [A-Z ]*PRIVATE KEY-----)'

for f in "${FILES[@]}"; do
  [ -f "$f" ] || continue
  case "$f" in
    docs/baselines/*|docs/archive/*|tests/scripts/public_export.py|tests/scripts/test_public_export.py|tests/scripts/hygiene_check.sh)
      continue
      ;;
  esac

  # In src/memory.rs, skip the explicit secret-rejection test lines (which test that secrets are blocked)
  if [ "$f" = "src/memory.rs" ]; then
    matches=$(grep -n -E "$LEAK_PATTERNS" "$f" | grep -v -E "(AKIAIOSFODNN7EXAMPLE|BEGIN RSA PRIVATE KEY)" || true)
  else
    matches=$(grep -n -E "$LEAK_PATTERNS" "$f" || true)
  fi

  if [ -n "$matches" ]; then
    echo "HYGIENE ERROR [private-leakage]: Forbidden private path or secret pattern in $f:" >&2
    echo "$matches" >&2
    ERRORS=$((ERRORS + 1))
  fi
done

# 2. Zero-Emoji & Sparkline Discipline (allowing deliberate '⚡ eg:' MCP footer token)
python3 - "${FILES[@]}" <<'PY' || ERRORS=$((ERRORS + 1))
import re, sys
emoji_re = re.compile(r'[\U0001F300-\U0001F9FF\u2600-\u26FF\u2700-\u27BF\U0001F1E6-\U0001F1FF\u2581-\u258F]')
failed = False
for arg in sys.argv[1:]:
    p = arg
    if not (p.startswith("src/") or p.startswith("skills/") or p.endswith(".md")):
        continue
    try:
        with open(p, "r", encoding="utf-8", errors="replace") as fh:
            for i, line in enumerate(fh, 1):
                if emoji_re.search(line):
                    if "⚡" in line and "eg:" in line:
                        continue
                    print(f"HYGIENE ERROR [zero-emoji]: Forbidden emoji or sparkline in {p}:{i}:\n  {line.strip()}", file=sys.stderr)
                    failed = True
    except FileNotFoundError:
        pass
if failed:
    sys.exit(1)
PY

# 3. Rust Code-Smells Gate: Hard Cap 500 LOC (Zero Allowlist), Unsafe & Debug Macros
MAX_LOC=500

for f in "${FILES[@]}"; do
  [ -f "$f" ] || continue
  case "$f" in
    *.rs)
      ;;
    *)
      continue
      ;;
  esac

  # Check LOC - Strict enforcement on ALL rust files
  lines=$(wc -l < "$f" | tr -d ' ')
  if [ "$lines" -gt "$MAX_LOC" ]; then
    echo "CODE-SMELL ERROR [oversized-file]: $f has $lines lines (exceeds 500 LOC threshold by $((lines - MAX_LOC)) lines)." >&2
    ERRORS=$((ERRORS + 1))
  fi

  # Check for forbidden unsafe code blocks / declarations: 'unsafe fn', 'unsafe {', 'unsafe trait', 'unsafe impl'
  if grep -n -E '\bunsafe\s*(\{|\b(fn|trait|impl)\b)' "$f" >/dev/null 2>&1; then
    echo "CODE-SMELL ERROR [unsafe-code]: 'unsafe' block or declaration is strictly forbidden in $f:" >&2
    grep -n -E '\bunsafe\s*(\{|\b(fn|trait|impl)\b)' "$f" >&2
    ERRORS=$((ERRORS + 1))
  fi

  # Check for stray dbg! macros
  if grep -n -E '\bdbg!\(' "$f" >/dev/null 2>&1; then
    echo "CODE-SMELL ERROR [dbg-macro]: Stray dbg!(...) macro found in $f:" >&2
    grep -n -E '\bdbg!\(' "$f" >&2
    ERRORS=$((ERRORS + 1))
  fi
done

if [ "$ERRORS" -gt 0 ]; then
  echo "hygiene_check: $ERRORS code-smell check(s) failed." >&2
  exit 1
fi

echo "hygiene_check: all files clean (leakage, emoji, LOC <= 500, unsafe, dbg) — PASS"
