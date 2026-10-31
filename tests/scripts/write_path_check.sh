#!/usr/bin/env bash
# G5 invariant 4 (CI grep): the source must contain no references to agent
# config/hooks surfaces, and every filesystem write must flow through the
# kg_dir/db_path helpers (constant write-path allowlist per spec).
set -euo pipefail
SRC="$(cd "$(dirname "$0")/../.." && pwd)/src"
FORBIDDEN='(\.claude|\.codex|AGENTS\.md|CLAUDE\.md|settings\.json|keychain|credentials|hook_file|pre-commit)'
if grep -rnE "$FORBIDDEN" "$SRC" >/dev/null 2>&1; then
  echo "G5 write-path: FORBIDDEN agent-config reference found:" >&2
  grep -rnE "$FORBIDDEN" "$SRC" >&2
  exit 1
fi
# Write primitives must appear only in db.rs (kg_dir-scoped) and doctor (invalidate).
# Allowed outside db.rs/doctor.rs: the atomic swap rename in main.rs (both
# paths are kg_dir-scoped helpers) and walk.rs's #[cfg(test)] fixture writes.
VIOLATIONS=$(grep -rn "fs::write\|fs::rename\|OpenOptions" "$SRC" \
  | grep -v "src/db.rs" | grep -v "src/doctor.rs" \
  | grep -v "fs::rename(db::db_new_path" \
  | grep -v 'src/walk.rs.*fs::write' || true)
if [ -n "$VIOLATIONS" ]; then
  echo "G5 write-path: writes outside db.rs/doctor.rs:" >&2
  echo "$VIOLATIONS" >&2
  exit 1
fi
echo "G5 write-path: writes confined to .kg/ helpers — PASS"
