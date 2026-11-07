# Experiment plan — bổ sung số liệu cho aw 0.4.0

- Ngày: 2026-11-05
- Baseline: commit `365863a`, branch `feat/graft-learning`, aw 0.4.0, schema v8
- Manifest: [`docs/baselines/2026-11-05_0.4.0/manifest.md`](../baselines/2026-11-05_0.4.0/manifest.md)
- Trạng thái: Đợt 1 hoàn tất (E0, E1.1, E1.2); các đợt sau chưa chạy

## Mục tiêu

Biến các ô "chưa đo" trong bảng feature × giá trị agent thành số thật, và
re-baseline các feature chỉ có số lịch sử. Mỗi thí nghiệm phải có oracle
đóng băng trước khi chạy, negative control, và kết quả ghi thành JSON.

## Nguyên tắc

- Oracle và fixture đóng băng trước khi chạy; ghi commit + binary hash.
- Mỗi thí nghiệm deterministic có negative control (fixture cố tình sai phải fail).
- Agent A/B tối thiểu 3 runs/cell; không trộn host trong một aggregate.
- Tách số 0.4.0 khỏi số historical trong mọi báo cáo.
- Không thêm dependency Rust mới. Script Python chỉ dùng stdlib.

## Bảng thí nghiệm

| ID | Feature | Thí nghiệm | Fixture / oracle | N | Metric | Ngưỡng đạt | Trạng thái |
|---|---|---|---|---|---|---|---|
| E0 | all | Baseline manifest | commit + release binary + `aw stats` | 1 | manifest | đầy đủ hash/version/stats | **DONE** |
| E1.1 | `build` | Incremental equivalence | 6 mutation × 3 topology | 18 | edge-set + unresolved-set equality | 18/18 + negative control | **DONE 18/18** |
| E1.2 | `callees` | Precision/recall + empty semantics | 20 symbol (10 resolved, 5 ambiguous, 5 unresolved-only) | 20 | P/R, silent-empty, ambiguous hits | P ≥ 0,95; R ≥ 0,95; silent-empty = 0; ambiguous 5/5 | **DONE — gate PASS sau fix (silent-empty 0/5)** |
| E1.3 | `mcp` | Frame conformance + parity | 20 frame JSON-RPC; 10 query | 30 | pass count | 20/20 frame; 10/10 parity | TODO |
| E1.4 | `complexity` | Calibration | 30 hàm Python/TS/Rust | 30 | Spearman ρ vs reference CCN | ρ ≥ 0,8 | TODO |
| E1.5 | `hotspots` | Scope invariance | 1 repo × 3 scope | 3 | raw metric equality | 100% | TODO |
| E1.6 | `resolve` | Contract matrix | 50 tên | 50 | exit code + class | 100% | TODO |
| E1.7 | `files`/`stats`/`report` | Consistency | index hiện tại + `git ls-files` | 3 | exact match | 100% | TODO |
| E1.8 | `path`/`imports`/`impact`/`context`/`search`/`clones`/`tests`/`implementers`/`dead` | Re-baseline static accuracy | frozen sets sẵn có + `bench/` | theo harness | recall/precision vs historical | không regression | TODO |
| E2.1 | `callees` | Agent A/B pilot | 3 task frozen (2-hop trace) | 18 runs | set-match, tool calls, tokens, silent-empty | aw ≥ no-aw; 0 silent-empty | TODO |
| E2.2 | `path` | Confirmation A/B | 3 route task | 18 runs | calls/tokens delta | ≥20% call reduction | TODO |
| E3 | all | Phân tích + cập nhật EVALUATION | kết quả E0–E2 | — | bảng số + Wilson CI | publish | TODO |

## Đợt 1 — kết quả

### E1.1 `build` incremental equivalence

- Command: `python3 -I tests/scripts/incremental_equivalence.py --aw target/release/aw --json docs/baselines/2026-11-05_0.4.0/e1.1.json`
- Kết quả: **18/18 case equal**, negative control fire.
- 6 mutation: add, delete, rename, touch-only, gap-heal, candidate-shrink (flat/package) hoặc candidate-rename (inheritance).
- Hai case candidate-shrink có **2 ambiguous edges** trước mutation, nên thực sự exercise candidate identity.
- Timing trên fixture nhỏ: incremental median 38,7 ms vs force 39,5 ms — không phân biệt được vì process startup chiếm ưu thế; cần synth corpus cho timing.

### E1.2 `callees` precision/recall + empty semantics

- Command: `python3 -I tests/scripts/callees_precision.py --aw target/release/aw --json docs/baselines/2026-11-05_0.4.0/e1.2.json`
- Resolved: **precision 1,000, recall 1,000** (23/23, 0 FP, 0 FN); Wilson 95% [0,857; 1,000].
- Ambiguous: **5/5** hit `[ambiguous]` đúng class.
- Unresolved-only: **silent-empty 0/5 sau fix** (trước fix 5/5) — `callees` giờ nêu tên unresolved site và hint `aw unresolved --names`.
- Gate: **PASS sau fix** (P/R 1,000; ambiguous 5/5; silent-empty 0); fix nằm ở `callee_empty_reason` trong `src/query/callgraph.rs`.

## Thứ tự thực thi tiếp

1. (đã xong) Fix `callees` near-miss và chạy lại E1.2 `--strict`; gate PASS.
2. E1.8 re-baseline trên `bench/` + frozen sets hiện có; ghi delta so với historical.
3. E1.3 MCP conformance; E1.6/E1.7 contract matrix.
4. E2.1 agent A/B pilot cho `callees`; E2.2 xác nhận `path`.
5. E3 cập nhật `docs/EVALUATION.md` mục "0.4.0 measurements".

## Không làm

- Không agent A/B cho `files`, `stats`, `report`, `resolve`, `complexity`, `hotspots`.
- Không chạy lại toàn bộ P9 80-cell; pilot trước, mở rộng khi có tín hiệu.
- Không dùng cache-warm làm số chính; không trộn host.
