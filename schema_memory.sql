-- aw memory schema v1 — the project-memory store `.aw/memory.db` (companion
-- design: docs/designs/2026-11-05_agent-weave-memory-retrieval.md). Separate
-- file from index.db on purpose: memory is primary data, the index is derived.
-- The binary gates this store on its own compiled MEMORY_SCHEMA_VERSION
-- (src/db.rs stamps PRAGMA user_version; this file declares no PRAGMA, the
-- same contract as schema.sql). The version policy deliberately diverges from
-- the index: an older memory.db that passes the compiled required-structure
-- manifest opens for read and write (additive evolution, never migrated in
-- place); newer than the binary, or missing required structure, exits 5.
-- Secondary indexes live here (not schema_indexes.sql): memory writes are
-- single-row transactions with no bulk-insert phase to post-index.

CREATE TABLE memories(
    id INTEGER PRIMARY KEY,
    key TEXT NOT NULL,              -- caller-chosen stable identity, <= 200 chars, no control chars
    rev INTEGER NOT NULL,           -- 1-based; new rev = old rev + 1 under one transaction
    kind TEXT NOT NULL,             -- closed set: decision | gotcha | fact | task
    scope TEXT NOT NULL DEFAULT 'durable',   -- durable | session (session accepted from Phase 2)
    session TEXT NOT NULL DEFAULT '',        -- AW_SESSION attribution (env, trimmed/capped 64)
    body TEXT NOT NULL,             -- agent-authored prose, hard cap 8192 bytes
    tags TEXT NOT NULL DEFAULT '',  -- whitespace-separated lowercase tokens, <= 16 tags x 32 chars
    refs TEXT NOT NULL DEFAULT '',  -- whitespace-separated qualnames / repo paths, <= 16 x 256 chars
    content_hash INTEGER NOT NULL,  -- FNV-1a 64 over (kind, body, tags, refs, scope) — idempotent replay
    source TEXT NOT NULL,           -- cli | mcp
    created_at INTEGER NOT NULL,    -- unix seconds
    expires_at INTEGER NOT NULL DEFAULT 0,   -- 0 = never; session rows carry the TTL deadline
    superseded_by INTEGER,          -- id of the replacing rev; NULL = current row
    superseded_at INTEGER NOT NULL DEFAULT 0 -- unix secs when superseded; 0 = current; the gc retention clock
);

CREATE UNIQUE INDEX memories_key_rev ON memories(key, rev);
CREATE INDEX memories_current ON memories(key, rev DESC) WHERE superseded_by IS NULL;
CREATE INDEX memories_expiry ON memories(expires_at) WHERE expires_at > 0;

CREATE VIRTUAL TABLE memories_fts USING fts5(text, tokenize='unicode61');
-- rowid = memories.id; text = mem_tokens(key) + mem_tokens(tags) +
-- mem_tokens(body) joined with spaces — the SAME tokenizer function the
-- query side uses, pre-applied at insert so unicode61's whitespace splitting
-- suffices; camelCase in prose (calleeEmptyReason) is already split here.
-- refs are NOT in the FTS text — the ref-overlap signal covers them
-- separately (no double count). Delta-maintained in the same transaction as
-- every insert/supersede/delete. Purity invariant: memories_fts is a pure
-- function of memories (rebuild-and-compare test).
