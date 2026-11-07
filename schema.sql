-- aw schema v10 — v9 -> v10 drops symbols.exported, symbols.effective_loc,
-- and files.generated. The binary opens a DB only when PRAGMA user_version equals its own
-- compiled schema version; older AND newer both exit 5. The DB is never
-- migrated in place.
-- Tables and the FTS table live here; secondary indexes live in
-- schema_indexes.sql and are created after the bulk insert of a fresh build
-- (the final schema is unchanged).
CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT);
-- meta rows: aw_version, grammar_version, built_at, source_rev, file_count,
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
    unresolved INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE symbols(
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    qualname TEXT NOT NULL,
    file_id INTEGER NOT NULL REFERENCES files(id),
    file_path TEXT NOT NULL DEFAULT '',
    line_start INTEGER NOT NULL,
    line_end INTEGER NOT NULL,
    -- One-line signature (Amendments 12a and 18): declaration header with
    -- line-wrapped parameters, whitespace collapsed, hard cap 160 chars.
    -- Empty for module symbols. No bodies.
    signature TEXT NOT NULL DEFAULT '',
    -- Test classification (v8): 1 when the symbol came from a test path or
    -- test-shaped name, or sits under Rust #[test]/#[cfg(test)] or a TS
    -- describe/it callback. Classification happens once at extraction; the
    -- column is authoritative for anti-reinvention test filtering.
    is_test INTEGER NOT NULL DEFAULT 0
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
-- Symbol search (Amendment 12b): standalone FTS5 over split identifier
-- tokens; repopulated by one pass from symbols at the end of every build
-- (the build replaces the whole DB, so no incremental maintenance exists).
CREATE VIRTUAL TABLE symbols_fts USING fts5(text);
-- Unresolved edge sites per file (Amendment 15/19): kind is call|import|inherit;
-- site_line is 1-based. The fast path re-extracts a carried file when a newly
-- added symbol/name matches one of its unresolved rows.
CREATE TABLE unresolved_calls(
    file_id INTEGER NOT NULL REFERENCES files(id),
    dst_name TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT 'call',
    site_line INTEGER NOT NULL DEFAULT 0
);
