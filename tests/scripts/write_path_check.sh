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
prim = re.compile(r"fs::write|fs::rename|OpenOptions|File::create|File::create_new|File::options|fs::copy|remove_file|remove_dir|create_dir|create_dir_all")
allowed = {"db.rs", "doctor.rs", "journal.rs"}
bad = []
def strip_test_modules(text):
    """Remove #[cfg(test)] mod blocks (brace-matched) before scanning."""
    out, i = [], 0
    while True:
        j = text.find("#[cfg(test)]", i)
        if j < 0:
            out.append(text[i:]); break
        out.append(text[i:j])
        k = text.find("{", j)
        if k < 0:
            out.append(text[j:]); break
        depth, m = 1, k + 1
        while m < len(text) and depth:
            if text[m] == "{": depth += 1
            elif text[m] == "}": depth -= 1
            m += 1
        i = m
    return "".join(out)

for p in sorted(src.glob("*.rs")):
    if p.name in allowed:
        continue
    text = p.read_text()
    scanned = strip_test_modules(text) if p.name == "walk.rs" else text
    for n, line in enumerate(scanned.splitlines(), 1):
        if not prim.search(line):
            continue
        if p.name == "build.rs" and "db_new_path" in line:
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
printf '#[cfg(test)]\nmod tests { fn t() { std::fs::write("/tmp/x", b"y").unwrap(); } }\n\npub fn leaky() { std::fs::write("/tmp/z", b"w").unwrap(); }\n' > "$probe/src/walk.rs"
printf 'fn t() { std::fs::File::options().write(true).create(true).open("/tmp/y").unwrap(); }\n' >> "$probe/src/query.rs"
probe_bad="$(check "$probe/src")"
if ! echo "$probe_bad" | grep -q "src/walk.rs:.*write"; then
  echo "G5 write-path: negative control missed a post-test write in walk.rs" >&2; exit 1
fi
if ! echo "$probe_bad" | grep -q "src/query.rs:.*File::options"; then
  echo "G5 write-path: negative control missed File::options" >&2; exit 1
fi
if ! echo "$probe_bad" | grep -q "src/evil.rs"; then
  echo "G5 write-path: negative control missed evil.rs" >&2; exit 1
fi
echo "G5 write-path: writes confined to kg_dir-scoped helpers (walk.rs test-only; controls caught) — PASS"
