# Handoff — tối ưu `aw build --force`

- Ngày: 2026-11-03
- Baseline: `main` @ `6dbe57a` (`fix(p8): close R12 review findings`)
- Binary: `target/release/aw` v0.2.1, schema v7
- Phạm vi: tối ưu fresh full build (`aw build --force`); không đổi query contract, schema, hay fast path.
- Trạng thái handoff: chỉ ghi nhận hiện trạng; chưa sửa code trong file này.

## 1. Hiện trạng đo được

Corpus chuẩn: `/tmp/aw-eval`, 2,025 file Python synth, 85,050 symbols, 164,025 edges.

Ba run `build --force` gần nhất trên host hiện tại:

| run | wall | walk | parse | extract | db |
|---|---:|---:|---:|---:|---:|
| 1 | 3.18 s | 0.06 s | 1.07 s | 2.93 s | 1.89 s |
| 2 | 3.94 s | 0.01 s | 1.03 s | 2.64 s | 2.72 s |
| 3 | 6.28 s | 0.02 s | 1.83 s | 4.27 s | 4.12 s |

Host lúc đo: 8 CPU, load average 8.64. Đây là host nhiễu; baseline worktree trước đó cũng chậm tương tự. Các run sạch trước đó ghi nhận best-of-7 **2.70-2.79 s**; budget hiện tại là **2.91 s** (1.25x mốc 2.33 s).

Kết luận tạm thời: chưa đủ bằng chứng để nói có regression. Phải đo lại trên host quiet trước khi tối ưu.

## 2. Luồng build hiện tại

File chính: `src/build.rs`.

Thứ tự trong `run_build`:

1. `walk::walk` — quét file.
2. `probe_old` + `old_files` — chỉ dùng khi không `--force`.
3. Phân loại carried/fresh; dirty closure; no-op fast path.
4. `extract_units` — đọc file, tree-sitter parse, extract, file metrics, clone shingles, hash.
5. `resolve` — resolve symbols/edges trong bộ nhớ.
6. `create_fresh` — tạo `.aw/index.db.new`.
7. `BEGIN`; carry-forward; `insert_files`; `insert_unresolved_calls`; `insert_symbols`; `insert_edge_rows`; `populate_symbols_fts`; `insert_clone_groups`; ghi meta counts; `COMMIT`.
8. `rename(2)` `.new` -> `.aw/index.db`.

Các điểm chính:

| Vị trí | Vai trò |
|---|---|
| `src/build.rs::run_build` | orchestration, phase timing |
| `src/build.rs::extract_units` | parallel parse/extract, worker mặc định `available_parallelism` clamp 1-8, stack 64 MB |
| `src/db.rs::create_fresh` | mở DB mới, chạy `schema.sql`, stamp `user_version` |
| `src/db.rs::insert_files` | insert 500 row/chunk, statement reuse |
| `src/db.rs::insert_symbols` | insert 500 row/chunk, clone `file_path` mỗi symbol |
| `src/db.rs::insert_edge_rows` | insert 500 row/chunk |
| `src/db.rs::populate_symbols_fts` | SELECT toàn bộ symbols, `split_ident`, insert FTS từng row |
| `src/db.rs::insert_clone_groups` | clone shingles |
| `src/db.rs::set_meta` | nhiều câu COUNT/SUM độc lập sau insert |
| `src/extract.rs::extract_file` | symbols, imports, calls, metrics |
| `src/extract.rs::callable_metrics` | cyclomatic/branches/nesting/effective LOC |
| `src/extract.rs::file_code_lines` | mask comment theo file, thêm một traversal |
| `src/parse.rs` | parse + `decision_nodes` + `non_comment_loc` |

## 3. Chi phí quan sát được

Phase breakdown hiện tại:

- `db`: 1.89-4.12 s — lớn nhất và biến động mạnh.
- `extract`: 2.64-4.27 s — số liệu là tổng thời gian worker, chồng lấn với wall.
- `parse`: 1.03-1.83 s — wall của phase parallel.
- `walk`: 0.01-0.06 s.

`resolve` có timing `_resolve_secs` nhưng hiện không được in ra. Cần đo riêng trước khi tối ưu.

## 4. Ứng viên tối ưu, theo thứ tự điều tra

### A. DB write path — ưu tiên cao

1. **PRAGMA cho DB tạm**. `create_fresh` chưa set `journal_mode`, `synchronous`, `temp_store`, `cache_size`. DB `.new` được thay bằng rename và nằm trong một transaction; nếu crash thì bỏ `.new`. Có thể dùng `journal_mode=OFF`/`synchronous=OFF` cho DB build tạm, nhưng phải chứng minh không ảnh hưởng durability của `index.db` sau rename.
2. **Tạo secondary index sau bulk insert**. `schema.sql` tạo index trước khi insert. Với fresh DB, drop/defer các index không cần cho insert rồi `CREATE INDEX` sau khi insert có thể nhanh hơn. Phải giữ nguyên schema cuối và `probe_old`/query contract.
3. **FTS population**. `populate_symbols_fts` SELECT toàn bộ rồi `split_ident` + insert từng row trong Rust. Đo riêng phần này. Có thể batch/transaction tốt hơn, hoặc chỉ rebuild FTS khi cần. Không đổi tokenization.
4. **Meta counts**. Nhiều `COUNT(*)`/`SUM` trên `symbols`, `edges`, `files` sau insert. Có thể gộp hoặc dùng counter đã có trong resolution/metadata. `callable_count` hiện là một COUNT riêng.
5. **`insert_symbols` clone `file_path`**. Mỗi symbol clone path; có thể dùng batch/UPDATE sau insert hoặc tham chiếu file row. Cân nhắc vì schema v7 đã denormalize có chủ đích.

### B. Extract/parse path

1. Đo `AW_WORKERS=1,2,4,8`; xác định scaling thật của `parse` và `extract`.
2. `extract_units` thực hiện nhiều lượt trên cùng file: parse, `extract_file`, `decision_nodes`, `non_comment_loc`, `shingles`, hash. Tìm lượt trùng lặp.
3. `file_code_lines` thêm một traversal toàn file. Có thể gộp comment collection vào traversal có sẵn nếu chứng minh không đổi `effective_loc`.
4. Kiểm tra `callable_metrics` và `extract_file` trên corpus 85k; tránh tối ưu dựa trên synthetic không đại diện.

### C. Resolve path

- Thêm timing cho `resolve` vào output tạm hoặc benchmark ngoài.
- Kiểm tra chi phí `load_carried`/`symbol_names`/`dirty_closure` trên đường `--force` (một số bước chỉ dùng khi `old` tồn tại).
- Không tối ưu `resolve` nếu phase này dưới 5-10% wall.

## 5. Ràng buộc

- Không thêm dependency; std-only.
- Không dùng `unsafe`.
- Không đổi schema v7, output contract, query semantics, hay byte shape.
- Giữ atomic swap, build lock, `.aw/` write containment.
- Giữ determinism: serial/parallel và fresh/carried phải cho rows giống nhau.
- Không sửa `docs/baselines/p8-utility-2026-11-03/`; đây là artifact benchmark chưa commit, không liên quan.
- Kiểm tra process benchmark còn chạy trước khi đo latency:

```bash
ps aux | grep -E 'run_p8|grok|opencode|agy|zcode' | grep -v grep
```

## 6. Quy trình đo đề nghị

```bash
cd /Users/quy.doan/Workspace/aw
cargo build --locked --release
python3 -I tests/scripts/make_synth_corpus.py /tmp/aw-build-force --files 2025

for i in 1 2 3 4 5 6 7; do
  /usr/bin/time -p target/release/aw --root /tmp/aw-build-force build --force \
    2>&1 | grep -E 'wall=|real'
done
```

Ghi lại:

- wall, walk, parse, extract, db
- best-of-7 và median
- peak RSS (`/usr/bin/time -l` trên macOS)
- index size
- `AW_WORKERS=1,2,4,8`
- load average trước/sau mỗi run

Baseline so sánh: commit `6dbe57a`, cùng corpus, cùng host, cùng binary build flags.

Budget hiện tại: **2.91 s** cho 2,025 file. Nếu không đạt, ghi rõ là host noise hay regression; không tự nới budget.

## 7. Acceptance criteria

Một thay đổi chỉ được nhận khi:

1. `cargo fmt --all -- --check` pass.
2. `cargo clippy --all-targets --locked -- -D warnings` pass.
3. `cargo test --locked` pass toàn bộ.
4. Best-of-7 `build --force` trên 2,025 file <= 2.91 s trên host quiet.
5. Serial vs parallel rows identical.
6. Fresh vs carried `symbols`/`edges`/`files`/`unresolved_calls` identical.
7. Index schema v7 và `symbols_complexity_idx` giữ nguyên.
8. Query smoke: `complexity`, `dead`, `imports`, `impact`, `api` không đổi output.

## 8. Bàn giao

Bắt đầu bằng:

1. Đọc `src/build.rs::run_build`, `src/build.rs::extract_units`, `src/db.rs::create_fresh`, `src/db.rs::populate_symbols_fts`.
2. Đo baseline quiet-host trước khi sửa.
3. Chỉ chọn một work package A/B/C; không trộn refactor với tối ưu.
4. Ghi kết quả vào một report mới dưới `docs/archive/reports/` hoặc `docs/baselines/`.

Không có thay đổi source nào được thực hiện trong handoff này.
