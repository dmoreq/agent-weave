# Kết quả Đợt 1 — baseline 0.4.0

- Ngày: 2026-11-05
- Commit: `365863a` (branch `feat/graft-learning`)
- Binary: `target/release/aw`, baseline sha256 `f84ff77fbb18c7b5dd14a7b7b82244d6498a19316404b974b56f44c9852ac507`; post-fix sha256 `b8c661329c90b704bb4745bfa1c067db064989c8406a84fddb28d6f336e79e84`
- Schema: v8
- Index lúc đo: 85 files, 1.010 symbols, 3.323 edges (2.294 calls, 78 imports, 21 inherits), 6.474 unresolved, db 2.801.664 B

## E1.1 — incremental/forced edge-set equivalence

Lệnh:

```sh
python3 -I tests/scripts/incremental_equivalence.py \
  --aw target/release/aw \
  --json docs/baselines/2026-11-05_0.4.0/e1.1.json
```

Kết quả: **18/18 case equal**, `negative_control=true`.

| Topology | Case | Kết quả |
|---|---|---|
| flat | add, delete, rename, touch-only, gap-heal, candidate-shrink | 6/6 equal |
| package | add, delete, rename, touch-only, gap-heal, candidate-shrink | 6/6 equal |
| inheritance | add, delete, rename, touch-only, gap-heal, candidate-rename | 6/6 equal |

Chi tiết đáng chú ý:

- Hai case `candidate-shrink` có **2 ambiguous edges** trong build đầu, nên mutation xóa một candidate thực sự exercise candidate invalidation; incremental và forced vẫn bằng nhau.
- Case `gap-heal` đi từ unresolved site sang resolved edge sau khi thêm module còn thiếu; incremental khớp forced.
- Timing trên fixture nhỏ: incremental median **38,7 ms** (33,4–61,3), forced median **39,5 ms** (30,1–51,2). Không dùng số này để kết luận tốc độ: process startup chiếm phần lớn; cần synth corpus 2.025 file cho timing thật.

## E1.2 — `callees` precision/recall và empty semantics

Lệnh:

```sh
python3 -I tests/scripts/callees_precision.py \
  --aw target/release/aw \
  --json docs/baselines/2026-11-05_0.4.0/e1.2.json
```

| Nhóm | Kết quả |
|---|---|
| Resolved (10 symbol) | precision **1,000** (23/23), recall **1,000** (23/23), 0 FP, 0 FN; Wilson 95% [0,857; 1,000] |
| Ambiguous (5 symbol) | **5/5** trả `[ambiguous]` đúng class (`ambig.D1.m` … `ambig.D5.m`) |
| Unresolved-only (5 symbol) | **silent-empty 0/5 sau fix** (trước fix 5/5); `names_present 5/5` |

Gate theo kế hoạch: **FAIL trước fix** (`silent_empty=5`), **PASS sau fix** (`silent_empty=0`, P/R 1,000, ambiguous 5/5).

### Fix applied

`src/query/callgraph.rs` thêm `callee_empty_reason`: khi `total == 0`, nó lấy các
`unresolved_calls` có `kind='call'` nằm trong khoảng `line_start..line_end`
của symbol, dedupe tên, hiện tối đa 5 tên kèm `(+N more)`, và trỏ tới
`aw unresolved --names --path <file>`. Không có unresolved site thì chuỗi
`0 callees found` giữ nguyên.

Regression tests: `callees_empty_names_unresolved_sites` (có unresolved →
detail nêu tên + hint) và `callees_empty_without_unresolved_stays_plain`
(không có unresolved → không claim sai).

## Diễn giải

- Với 18 mutation có kiểm soát, incremental và forced build cho cùng edge-set và unresolved-set trên binary này. Không tìm thấy counterexample mới trong phạm vi fixture; không suy rộng thành bằng chứng toàn cục.
- `callees` trả đúng hoàn toàn trên call graph đã resolve của fixture, kể cả diamond ambiguity.
- Ngữ nghĩa của câu trả lời rỗng đã được sửa: trước fix agent đọc `0 callees found` sẽ kết luận không có callee dù 5/5 case có unresolved site; sau fix detail nêu tên unresolved và hint `aw unresolved --names`, `silent_empty=0/5`.

## Chưa bao phủ

- Timing incremental trên synth 2.025/4.000 file và so với `--force`.
- Incremental với Rust/TS mutation ngoài inheritance Python.
- P/R `callees` trên corpus thật (fixture chỉ 20 symbol).
- Agent A/B: chưa chạy; E2.1 là bước tiếp theo sau khi fix near-miss.
- MCP conformance, complexity calibration, hotspots invariance, resolve contract: chưa chạy.

## Bước tiếp theo

1. Chạy E1.8 re-baseline trên `bench/` + frozen sets, ghi delta.
2. E1.3 MCP conformance; E1.6/E1.7 contract matrix.
3. E2.1 agent A/B pilot cho `callees`.
