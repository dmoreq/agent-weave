#!/usr/bin/env bash
# G5 invariant 1 (CI): dependency allowlist check. No crate in Cargo.lock may
# link a resolver/TLS/network stack. The five allowed roots are clap,
# tree-sitter(+grammars), rusqlite, ignore, and their transitive deps — the
# check rejects known network families instead of enumerating the world.
set -euo pipefail
LOCK="${1:-Cargo.lock}"
FORBIDDEN='(reqwest|hyper|tokio|native-tls|rustls|openssl|curl|hickory|trust-dns|hyperfine|libcurl|ssh|quinn|surf|ureq|isahc|attohttpc|mio$)'
if grep -E "^name = \"?$FORBIDDEN" "$LOCK" >/dev/null 2>&1; then
  echo "G5 allowlist: FORBIDDEN network crate found in $LOCK:" >&2
  grep -E "^name = \"?$FORBIDDEN" "$LOCK" >&2
  exit 1
fi
echo "G5 allowlist: no network crates in $LOCK — PASS"
