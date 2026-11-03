-- aw schema v7 secondary indexes.
-- A fresh build creates these after the bulk insert, so each index is sorted
-- once instead of maintained per row. The final schema is identical to the
-- historical schema.sql layout (same names, same key order).
CREATE INDEX edges_dst ON edges(dst_id, kind);
-- Covering reverse-edge index: BFS reads src/site without table lookups.
CREATE INDEX edges_dst_cover ON edges(dst_id, kind, src_id, site_line);
CREATE INDEX edges_src ON edges(src_id, kind);
CREATE INDEX symbols_file ON symbols(file_id);
CREATE INDEX symbols_qualname ON symbols(qualname);
CREATE INDEX symbols_complexity_idx
    ON symbols(cyclomatic DESC, effective_loc DESC, qualname, file_path, line_start);
CREATE INDEX unresolved_calls_file ON unresolved_calls(file_id);
CREATE INDEX unresolved_calls_kind ON unresolved_calls(kind, dst_name);
CREATE INDEX clone_groups_hash ON clone_groups(shingle_hash, file_id, line_start);
CREATE INDEX symbols_name ON symbols(name);
