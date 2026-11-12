-- eg memory schema (user_version 2) — the project-memory store `.engram/memory.db`
-- (companion design: docs/architecture/memory-store.md). Separate
-- file from index.db on purpose: memory is primary data, the index is derived.
-- The binary gates this store on its own compiled MEMORY_SCHEMA_VERSION
-- (src/db.rs stamps PRAGMA user_version; this file declares no PRAGMA, the
-- same contract as schema/index.sql). The version policy deliberately diverges from
-- the index: an older memory.db that passes the compiled required-structure
-- manifest opens for read and write (additive evolution, never migrated in
-- place); newer than the binary, or missing required structure, exits 5.
-- Secondary indexes live here (not schema/indexes.sql): memory writes are
-- single-row transactions with no bulk-insert phase to post-index.

CREATE TABLE memories(
    id INTEGER PRIMARY KEY,
    key TEXT NOT NULL,              -- caller-chosen stable identity, <= 200 chars, no control chars
    rev INTEGER NOT NULL,           -- 1-based; new rev = old rev + 1 under one transaction
    kind TEXT NOT NULL,             -- closed set: decision | gotcha | fact | task | procedure
    scope TEXT NOT NULL DEFAULT 'durable',   -- durable | session (session accepted from Phase 2)
    session TEXT NOT NULL DEFAULT '',        -- ENGRAM_SESSION attribution (env, trimmed/capped 64)
    body TEXT NOT NULL,             -- agent-authored prose, hard cap 8192 bytes
    tags TEXT NOT NULL DEFAULT '',  -- whitespace-separated lowercase tokens, <= 16 tags x 32 chars
    refs TEXT NOT NULL DEFAULT '',  -- whitespace-separated qualnames / repo paths, <= 16 x 256 chars
    content_hash INTEGER NOT NULL,  -- FNV-1a 64 over (kind, body, tags, refs, scope) — idempotent replay
    source TEXT NOT NULL,           -- cli | mcp | import
    created_at INTEGER NOT NULL,    -- unix seconds
    anchored_at INTEGER NOT NULL DEFAULT 0, -- unix secs of the last verify; 0 = never (readers fall back to created_at)
    expires_at INTEGER NOT NULL DEFAULT 0,   -- 0 = never; session rows carry the TTL deadline
    superseded_by INTEGER,          -- id of the replacing rev; NULL = current row
    superseded_at INTEGER NOT NULL DEFAULT 0, -- unix secs when superseded; 0 = current; the gc retention clock
    fts_version INTEGER NOT NULL DEFAULT 0    -- 0 = legacy writer (v2 index may lag); 2 = indexed in both FTS tables
);

CREATE UNIQUE INDEX memories_key_rev ON memories(key, rev);
CREATE INDEX memories_current ON memories(key, rev DESC) WHERE superseded_by IS NULL;
CREATE INDEX memories_expiry ON memories(expires_at) WHERE expires_at > 0;
CREATE INDEX memories_created ON memories(created_at);
CREATE INDEX memories_pending ON memories(id) WHERE fts_version < 2;

-- Internal key/value state for the derived FTS indexes: fts_epoch (tokenizer
-- contract) and fts_v2_complete ("1" only after a clean reconcile). Missing
-- rows mean "not usable"; this table is not part of MEMORY_REQUIRED.
CREATE TABLE memory_meta(
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE VIRTUAL TABLE memories_fts USING fts5(text, tokenize='unicode61');
-- Legacy index: rowid = memories.id; text = mem_fts_text_legacy(key, tags,
-- body). It is the 1.0.4 contract and must keep the frozen ASCII tokenizer.
-- refs are NOT in the FTS text. Delta-maintained in the same transaction as
-- every insert/supersede/delete. Purity invariant: memories_fts is a pure
-- function of the current memories rows (rebuild-and-compare test).

CREATE VIRTUAL TABLE memories_fts_v2 USING fts5(text, tokenize='unicode61 remove_diacritics 2');
-- Unicode index: rowid = memories.id; text = mem_fts_text(key, tags, body).
-- The SQLite tokenizer folds Vietnamese tones; mem_tokens_unicode maps đ -> d.
-- Reads use this table only when memory_meta says fts_epoch = 2 and
-- fts_v2_complete = 1. Maintained in lockstep with memories_fts.
