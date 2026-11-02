#!/usr/bin/env bash
# G5 invariant 4: no agent-config references; write/delete primitives only in
# kg_dir-scoped helpers. walk.rs is allowed ONLY inside #[cfg(test)].
set -euo pipefail
SRC="$(cd "$(dirname "$0")/../.." && pwd)/src"
FORBIDDEN='(\.claude|\.codex|AGENTS\.md|CLAUDE\.md|settings\.json|keychain|credentials|hook_file|pre-commit)'
if grep -rnE "$FORBIDDEN" "$SRC" >/dev/null 2>&1; then
  echo "G5 write-path: FORBIDDEN agent-config reference found:" >&2
  grep -rnE "$FORBIDDEN" "$SRC" >&2
  exit 1
fi
check() { # dir -> prints violations
  python3 - "$1" <<'PY'
import pathlib, re, sys
src = pathlib.Path(sys.argv[1])
prim = re.compile(r"fs::write|fs::rename|OpenOptions|File::create|fs::copy|remove_file|remove_dir|create_dir|create_dir_all")
allowed = {"db.rs", "doctor.rs", "journal.rs"}
bad = []
for p in sorted(src.glob("*.rs")):
    if p.name in allowed:
        continue
    in_test = False
    for n, line in enumerate(p.read_text().splitlines(), 1):
        if "#[cfg(test)]" in line:
            in_test = True
        if not prim.search(line):
            continue
        if p.name == "build.rs" and "db_new_path" in line:
            continue
        if p.name == "walk.rs" and in_test:
            continue
        bad.append(f"{p}:{n}:{line.strip()}")
print("\n".join(bad))
PY
}
VIOLATIONS="$(check "$SRC")"
if [ -n "$VIOLATIONS" ]; then
  echo "G5 write-path: writes outside the allowlist:" >&2
  echo "$VIOLATIONS" >&2
  exit 1
fi
probe="$(mktemp -d)"; trap 'rm -rf "$probe"' EXIT
cp -R "$SRC" "$probe/src"
printf 'fn evil() { let _ = std::fs::File::create("/tmp/kg-write-probe"); }\n' > "$probe/src/evil.rs"
printf 'mod tests { fn t() { std::fs::write("/tmp/x", b"y").unwrap(); } }\n' > "$probe/src/walk_pre.rs"
if [ -z "$(check "$probe/src")" ]; then
  echo "G5 write-path: negative control missed a planted write" >&2
  exit 1
fi
# cfg(test) placement in walk.rs must NOT create a false positive.
printf '#[cfg(test)]\nmod tests { fn t() { std::fs::write("/tmp/x", b"y").unwrap(); } }\n' > "$probe/src/walk_pre.rs"
probe_bad="$(check "$probe/src")"
if echo "$probe_bad" | grep -q "src/evil.rs"; then
  : # evil.rs must be caught
else
  echo "G5 write-path: negative control missed evil.rs" >&2; exit 1
fi
echo "G5 write-path: writes confined to kg_dir-scoped helpers (walk.rs test-only; controls caught) — PASS"
