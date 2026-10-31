-- kg schema v1 — additive-only evolution policy.
-- The binary opens a DB only when PRAGMA user_version equals its own compiled
-- schema version; older AND newer both exit 5. The DB is never migrated in place.
CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT);
-- meta rows: kg_version, grammar_version, built_at, source_rev, file_count,
-- symbol_count, edge_count, parse_error_count, unresolved_callee_count
CREATE TABLE files(
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    size INTEGER NOT NULL,
    mtime INTEGER NOT NULL,
    content_hash INTEGER NOT NULL,
    parse_status TEXT NOT NULL,
    error_nodes INTEGER NOT NULL
);
CREATE TABLE symbols(
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    qualname TEXT NOT NULL,
    file_id INTEGER NOT NULL REFERENCES files(id),
    line_start INTEGER NOT NULL,
    line_end INTEGER NOT NULL
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
CREATE INDEX symbols_name ON symbols(name);
