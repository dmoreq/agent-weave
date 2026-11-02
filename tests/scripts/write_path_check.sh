#!/usr/bin/env bash
# G5 invariant 4 (CI grep): no agent-config/hooks references, and every
# filesystem write/delete must flow through kg_dir/db_path helpers.
set -euo pipefail
SRC="$(cd "$(dirname "$0")/../.." && pwd)/src"
FORBIDDEN='(\.claude|\.codex|AGENTS\.md|CLAUDE\.md|settings\.json|keychain|credentials|hook_file|pre-commit)'
if grep -rnE "$FORBIDDEN" "$SRC" >/dev/null 2>&1; then
  echo "G5 write-path: FORBIDDEN agent-config reference found:" >&2
  grep -rnE "$FORBIDDEN" "$SRC" >&2
  exit 1
fi
# Allowed: db.rs/doctor.rs/journal.rs (kg_dir-scoped helpers and journals),
# build.rs's atomic cleanup+swap (kg_dir-scoped paths), and walk.rs's
# cfg(test) fixture writes (module test only).
VIOLATIONS=$(grep -rnE "fs::write|fs::rename|OpenOptions|File::create|fs::copy|remove_file|remove_dir|create_dir" "$SRC" \
  | grep -v "src/db.rs" | grep -v "src/doctor.rs" | grep -v "src/journal.rs" \
  | grep -vE "fs::rename\((\w+::)?db::db_new_path" \
  | grep -vE "src/build\.rs:.*remove_file\((\w+::)?db::db_new_path" \
  | grep -v 'src/walk.rs' || true)
if [ -n "$VIOLATIONS" ]; then
  echo "G5 write-path: writes outside the allowlist:" >&2
  echo "$VIOLATIONS" >&2
  exit 1
fi
# Negative control: a planted File::create outside the allowlist must be caught.
probe="$(mktemp -d)"
trap 'rm -rf "$probe"' EXIT
cp -R "$SRC" "$probe/src"
printf 'fn evil() { let _ = std::fs::File::create("/tmp/kg-write-probe"); }\n' > "$probe/src/evil.rs"
if ! SRC="$probe/src" bash -c '
  grep -rnE "fs::write|fs::rename|OpenOptions|File::create|fs::copy|remove_file|remove_dir|create_dir" "$SRC" \
    | grep -v "src/db.rs" | grep -v "src/doctor.rs" | grep -v "src/journal.rs" \
    | grep -vE "fs::rename\((\w+::)?db::db_new_path" \
    | grep -vE "src/build\.rs:.*remove_file\((\w+::)?db::db_new_path" \
    | grep -v "src/walk.rs" | grep -q .'; then
  echo "G5 write-path: negative control missed a planted File::create" >&2
  exit 1
fi
echo "G5 write-path: writes confined to kg_dir-scoped helpers (negative control caught) — PASS"
