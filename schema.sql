-- kg schema v4 — additive-only evolution policy (v1 -> v2 adds files.unresolved;
-- v2 -> v3 adds files.loc/files.cyclomatic; v3 -> v4 adds symbols.signature,
-- Amendment 12a: one-line signature, no bodies, captured once at build).
-- The binary opens a DB only when PRAGMA user_version equals its own compiled
-- schema version; older AND newer both exit 5. The DB is never migrated in place.
CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT);
-- meta rows: kg_version, grammar_version, built_at, source_rev, file_count,
-- symbol_count, edge_count, parse_error_count, unresolved_callee_count
CREATE TABLE files(
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    size INTEGER NOT NULL,
    -- Nanosecond-precision stat mtime (fast-path and staleness key).
    mtime INTEGER NOT NULL,
    content_hash INTEGER NOT NULL,
    parse_status TEXT NOT NULL,
    error_nodes INTEGER NOT NULL,
    -- Unresolved-callee count for this file (authoritative per-file split of
    -- the meta unresolved_callee_count total; stable across incremental builds).
    unresolved INTEGER NOT NULL DEFAULT 0,
    -- Hotspots metrics (Amendment 11d), frozen at this file's last fresh
    -- extraction: line count and language-aware decision-node count.
    loc INTEGER NOT NULL DEFAULT 0,
    cyclomatic INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE symbols(
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    qualname TEXT NOT NULL,
    file_id INTEGER NOT NULL REFERENCES files(id),
    line_start INTEGER NOT NULL,
    line_end INTEGER NOT NULL,
    -- One-line signature (Amendment 12a): first line of the symbol's node
    -- text, trimmed, hard cap 160 chars. Empty for module symbols. No bodies.
    signature TEXT NOT NULL DEFAULT ''
);
CREATE TABLE edges(
    id INTEGER PRIMARY KEY,
    src_id INTEGER NOT NULL REFERENCES symbols(id),
    dst_id INTEGER REFERENCES symbols(id),
    dst_name TEXT NOT NULL,
    kind TEXT NOT NULL,
    confidence TEXT NOT NULL,
    site_line INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX edges_dst ON edges(dst_id, kind);
CREATE INDEX edges_src ON edges(src_id, kind);
CREATE INDEX symbols_qualname ON symbols(qualname);
-- Symbol search (Amendment 12b): standalone FTS5 over split identifier
-- tokens; repopulated by one pass from symbols at the end of every build
-- (the build replaces the whole DB, so no incremental maintenance exists).
CREATE VIRTUAL TABLE symbols_fts USING fts5(text);
-- Clone candidates (Amendment 12d): normalized 5-line-window FNV-1a shingles
-- computed during the build parse phase for fresh files and carried forward
-- with the fast path; grouping/pairing happens at query time.
CREATE TABLE clone_groups(
    shingle_hash INTEGER NOT NULL,
    file_id INTEGER NOT NULL REFERENCES files(id),
    line_start INTEGER NOT NULL,
    line_end INTEGER NOT NULL
);
CREATE INDEX clone_groups_hash ON clone_groups(shingle_hash, file_id, line_start);
CREATE INDEX symbols_name ON symbols(name);
