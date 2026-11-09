-- eg schema v10 secondary indexes.
-- A fresh build creates these after the bulk insert, so each index is sorted
-- once instead of maintained per row. The final schema is identical to the
-- historical schema/index.sql layout (same names, same key order).
CREATE INDEX edges_dst ON edges(dst_id, kind);
-- Covering reverse-edge index: BFS reads src/site without table lookups.
CREATE INDEX edges_dst_cover ON edges(dst_id, kind, src_id, site_line);
CREATE INDEX edges_src ON edges(src_id, kind);
-- Test-symbol anchor (v8): partial index covers only the test set.
CREATE INDEX symbols_is_test ON symbols(is_test, kind, name) WHERE is_test = 1;
CREATE INDEX symbols_file ON symbols(file_id);
-- Path-scope anchor: `--path` range scans stay on file_path.
CREATE INDEX symbols_file_path ON symbols(file_path);
CREATE INDEX symbols_qualname ON symbols(qualname);
CREATE INDEX unresolved_calls_file ON unresolved_calls(file_id);
CREATE INDEX unresolved_calls_kind ON unresolved_calls(kind, dst_name);
CREATE INDEX symbols_name ON symbols(name);
