#!/usr/bin/env bash
# Prepare P9 corpora: real git repos with a pre-built aw index.
# Idempotent: initialized corpora with a working index are skipped.
set -euo pipefail

DATE="${P9_DATE:-$(date +%Y-%m-%d)}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
SRC="${P9_SRC:-/tmp/p8-utility-corpus-2026-11-03}"
DST="${P9_DST:-/tmp/p9-corpus-$DATE}"
AW="${P9_AW:-$ROOT/target/release/aw}"
MANIFEST="$HERE/corpora-manifest.txt"

if [ ! -x "$AW" ]; then
    echo "missing aw binary: $AW" >&2
    echo "run: cargo build --release" >&2
    exit 1
fi

mkdir -p "$DST"
printf 'corpus|path|index|head|built_at\n' > "$MANIFEST"

for corpus in kg vihap finguard; do
    src="$SRC/$corpus"
    dst="$DST/$corpus"
    if [ ! -d "$src" ]; then
        echo "missing source corpus: $src" >&2
        exit 1
    fi

    if [ -d "$dst/.git" ] && [ -f "$dst/.aw/index.db" ]; then
        echo "skip $corpus (already initialized)"
    else
        echo "copy $corpus -> $dst"
        mkdir -p "$dst"
        rsync -a \
            --exclude='.git/' \
            --exclude='.kg/' \
            --exclude='.kg.*' \
            --exclude='.aw/' \
            "$src/" "$dst/"
        if [ ! -d "$dst/.git" ]; then
            git -C "$dst" init -q
            git -C "$dst" add -A
            git -C "$dst" -c user.name='P9 Harness' \
                -c user.email='p9@example.invalid' \
                commit -q -m init
        fi
        if [ ! -f "$dst/.aw/index.db" ]; then
            ( cd "$dst" && "$AW" build --no-journal )
        fi
    fi

    if [ ! -f "$dst/.aw/index.db" ]; then
        echo "missing index: $dst/.aw/index.db" >&2
        exit 1
    fi
    if ! ( cd "$dst" && "$AW" stats --no-journal >/dev/null ); then
        echo "aw stats failed in $dst" >&2
        exit 1
    fi
    head="$(git -C "$dst" rev-parse HEAD)"
    printf '%s|%s|%s|%s|%s\n' \
        "$corpus" "$dst" "$dst/.aw/index.db" "$head" \
        "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$MANIFEST"
done

echo "P9 corpora ready under $DST"
