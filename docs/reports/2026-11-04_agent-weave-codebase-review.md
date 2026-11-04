# Agent-weave: review toàn diện sau remediation

## Executive summary

**Bản cập nhật sửa được nhiều lỗi cũ, nhưng chưa đạt điều kiện trở thành nguồn quan hệ mặc định đáng tin cậy cho coding agent.** Sáu kiểm tra đều pass; probe mới vẫn tìm thấy graph sai, incremental inconsistency và filesystem/journal hazards.

Giữ architecture Rust/Tree-sitter/SQLite hiện tại. Ưu tiên invariant về binding, candidate identity và incremental equivalence; không cần thêm framework để xử lý các lỗi này.

## 1. Snapshot và provenance

| Thuộc tính | Phạm vi |
|---|---|
| Ngày | 2026-11-04 |
| Commit | `c8eecb3b36f851c78e8a144c08809cb96de674d3` |
| Baseline review trước | `1439a9e63f6483343ce69bfa2cda1369ed451be3` |
| Working tree lúc bắt đầu | Sạch |
| Runtime | 22 Rust files, 13.405 dòng |
| Integration tests | 3 Rust files, 5.843 dòng |
| Tracked inventory | 2.058 files |
| Kiểm chứng runtime | macOS, `rustc 1.94.1`; debug binary từ chính snapshot |
| Thay đổi do review | Chỉ report và machine-readable evidence; không sửa runtime |

Provenance lấy bằng `git rev-parse HEAD`, `git status --short --branch`, `git ls-files -z`; đếm dòng trực tiếp. Snapshot được tạo bằng `git archive`, có content manifest riêng và target directory cô lập.

Không dựa vào bảng “FIXED” của tác giả để đóng finding. Không dùng index `aw` làm oracle duy nhất để đánh giá chính `aw`.

### Concurrent changes tại thời điểm bàn giao

Final workspace-consistency guard **FAIL**: sáu tracked files đã đổi sau khi snapshot được đóng băng: `README.md`, `docs/EVALUATION.md`, `schema.sql`, `src/extract.rs`, `src/extract/walk.rs`, `src/mcp.rs`.

Source hashes của snapshot vẫn khớp manifest. Những uncommitted changes này không được test hoặc review trong verdict; không reset, overwrite hoặc gộp chúng vào acceptance. Báo cáo không chứng nhận working tree mới nhất.

## 2. Phương pháp và coverage

Tám luồng Goldfish: architecture/build; extraction/resolution; security/MCP; minimalism; query/reuse; documents/contracts; tests/enforcement; evaluation. Bốn model đọc chính là `openai/gpt-6.1-terra`, `openai/gpt-6.1-sol`, `openai/gpt-6-luna`, `openai/gpt-6-astra`.

Reviewer đọc source độc lập trước khi xem verdict cũ. Controller replay input cũ, tạo counterexamples mới và kiểm tra normalized graph thay vì chỉ nhìn output quen thuộc.

Hai lượt peer bị provider quota; chúng không được tính là completed reviews. Lượt thay thế bằng Terra đã kiểm tra phần còn lại; peer disagreements và metadata-dependent claim được giữ rõ trong register.

| Vùng | Coverage thực tế |
|---|---|
| Toàn bộ `src/**/*.rs` | Đọc đầy đủ qua hợp các luồng chuyên biệt |
| Rust integration tests, safety/export scripts | Đọc đầy đủ; một lần chạy suite và export regressions |
| Public root docs, sáu guides/reference, agent skill, workflows | Đọc đầy đủ; đối chiếu CLI/MCP/source |
| Historical evaluation implementations | Review collection; không có per-file read-span log cho mọi script |
| Historical transcripts/metrics | Inventory và bounded samples, không revalidate toàn bộ historical results |
| Minimalism riêng | Một số extraction file chỉ đọc phần đầu; không suy “clean” cho phần chưa đọc dưới lens đó |

[coverage.csv](assets/2026-11-04_agent-weave-codebase-review/coverage.csv) phân biệt full read, sampled, collection review và metadata-only. Inventory không đồng nghĩa semantic certification của 2.058 files.

## 3. Kiểm tra nền

| Lệnh | Kết quả | Thời gian lần chạy này |
|---|---|---:|
| `cargo fmt --all -- --check` | PASS | 1,08 s |
| `cargo clippy --locked --all-targets -- -D warnings` | PASS | 19,80 s |
| `cargo test --locked` | PASS | 46,38 s |
| `bash tests/scripts/allowlist_check.sh Cargo.lock` | PASS trên valid input | 0,13 s |
| `bash tests/scripts/write_path_check.sh` | PASS | 0,34 s |
| `python3 -I tests/scripts/test_public_export.py` | 8 tests PASS | 1,32 s |

Môi trường dùng offline Cargo, bốn build jobs và bốn Rust test threads. Thời gian có compilation/host overhead; không dùng chúng để kết luận latency regression hoặc agent utility.

Rust runner ghi nhận `27 lib + 27 binary + 136 CLI + 34 fixtures + 10 MCP = 234` lượt pass. Nhóm unit bị chạy hai lần qua lib và binary; không gọi đây là 234 test khác nhau.

Một lần PASS của allowlist với valid file không bác bỏ fail-open trên invalid input. Test suite không bao phủ hết binding/graph transitions; các counterexamples dưới đây không sửa source hoặc giảm gate để tạo kết quả.

## 4. Closure của 37 findings cũ

[closure.csv](assets/2026-11-04_agent-weave-codebase-review/closure.csv) ghi từng ID, status, scope và evidence. Giữ nguyên report cũ vì nó áp dụng snapshot khác.

| Status | Số lượng | Ý nghĩa |
|---|---:|---|
| `VERIFIED_ORIGINAL_CASE` | 29 | Input hoặc contract cũ đã được kiểm chứng trong phạm vi tương ứng |
| `PARTIAL` | 1 | A3: call ambiguity sửa đúng, import graph vẫn khác giữa build modes |
| `IMPROVED_LIMITED` | 2 | R1 decomposition và T3 inspection tốt hơn; không chứng nhận mọi helper/descendant |
| `DEFERRED` | 5 | R3–R6 và M1 là optional maintainability work |

Các probe thực thi cũ có **23/24 assertions pass**. Assertion còn fail là complete normalized-edge equivalence của A3; đây không phải accuracy estimate hoặc kết quả toàn bộ acceptance matrix.

Điểm cải thiện đã thấy:

- Static directory/lock symlinks không còn thay external sentinels; journal symlink không làm đổi target.
- Invalid UTF-8 và hai oversized-frame boundaries giữ được ping tiếp theo.
- Root lock chặn invalidation khi build lock domain đang được giữ.
- Removed-base case và tám resolution fixtures ban đầu không còn lỗi cũ.
- Duplicate scorer và missing/null historical hops bị từ chối đúng.
- Direct caller/callee loaders được chia sẻ mà không compose public queries có journaling side effects.

`VERIFIED_ORIGINAL_CASE` không có nghĩa đã chứng minh universal safety. Hardlinks, FIFOs, source-order changes và additional lexical scopes là counterexamples khác.

## 5. Findings hiện tại

[Finding register](2026-11-04_agent-weave-review-findings.md) có source locations, scenarios, correction và regression criteria. Các severity của reviewer được giữ nguyên; không đổi severity khi contest.

Có **31 accepted findings mới: 3 Critical, 25 Important, 3 Suggestion**. Năm Suggestions cũ vẫn deferred, SNEW4 là threat-model escalation riêng; không cộng alias RNEW5 hoặc các đề xuất evaluation đã rút lại.

| Nhóm | IDs | Kết quả có tính quyết định |
|---|---|---|
| Incremental data integrity | ANEW1, ANEW2, QNEW-A3 | Edge biến mất sau gap healing; removed candidate không heal; import destination phụ thuộc build mode |
| Read boundary | ANEW3 | Tracked path qua locally replaced ancestor symlink index external marker |
| Binding semantics | CNEW1–CNEW10 | Assignment, annotation, arrows, nested receivers và scoped imports vẫn tạo graph sai hoặc thiếu |
| Filesystem/journals | SNEW1–SNEW3, TNEW1 | Hardlink đổi external file; FIFO chặn completion; exporter follow tracked symlink |
| Accounting/verification | RNEW1–RNEW3 | Valid pointer bị FAIL; ba matches được báo thành năm; impact explanation thay theo row IDs |
| CI/contracts | DNEW1–DNEW3, TNEW2–TNEW3 | Missing action input, ambiguous routing wording và safety-control fail-open |
| Resource bounds | ANEW4–ANEW5 | Bind limit vượt; read-time growth không được hard-bound |
| Maintainability/protocol | RNEW4, RNEW6, SNEW5 | Coupled search, repeated payload policy và JSON-RPC envelope gaps |

Các case có capability khác nhau. Hardlinks/FIFOs cần local setup; ancestor case cần modified working tree; exporter file symlink có thể được Git archive giữ nguyên. Không gộp chúng thành một “remote exploit”.

### Counterexamples đã thực thi

```text
Gap healing:
  trước: a.py:3 a.entry
  incremental sau thêm module: 0 callees found
  forced với cùng source: a.py:3 a.entry

Assignment order:
  value = A(); value.run(); value = B(); value.run()
  cả hai call sites bị gán tới B.run

Search:
  3 definitions tên run, --limit 1
  showing 1 of 5

Transitive verification:
  ok=1 changed=0 fail=1
  FAIL ... [depth=2] (line does not mention 'leaf')
```

Source fixtures, exact stdout/stderr và controlled-query results có trong [evidence.json](assets/2026-11-04_agent-weave-codebase-review/evidence.json). Output SQL quá dài được lưu dưới dạng decisive prefix cùng digest/byte count, không copy hàng chục nghìn placeholders vào report.

Không chạy infinite-device/OOM probe hoặc large repository job. FIFO children được timeout, kill và reap; SQLite limit được kiểm tra bằng virtual paths trong một database nhỏ.

## 6. Architecture, tech stack và design patterns

**Tech stack vẫn phù hợp.** Không có finding nào đòi thay Rust, Tree-sitter, SQLite hoặc đưa async runtime vào sản phẩm.

| Thành phần | Giữ | Điều chưa được bảo đảm |
|---|---|---|
| Extraction → resolution → storage → query | Separation rõ, dễ audit từng stage | Binding events chưa giữ đủ lexical identity và source order |
| Root-directory lock | Ổn định cooperating build/invalidate lock domain | Không chống active hostile namespace replacement |
| Temporary DB + close/fsync/rename | Publication boundary hợp lý | Logical graph có thể sai trước khi publish atomic |
| Version/grammar/signature/metric gates | Tránh carry incompatible rows | Schema equality không chứng minh semantic equivalence |
| Side-effect-free direct loaders | Context reuse không ghi payload trung gian | Transitive verifier vẫn áp direct-target rule |
| Explicit ambiguity, caveats, fallback | Cần thiết cho consultative tool | Không bù được false positive tự tin từ binding sai |

Các fixes lexical binding đang giải quyết từng shape nhưng chưa thành invariant thống nhất. Nguyên tắc cần kiểm chứng là nearest visible binding, latest preceding event và receiver identity; không chỉ thêm exception cho fixture mới.

Candidate invalidation cũng cần invariant về thay đổi tập candidate theo cả hai chiều và transitive dirty closure. Không biến chúng thành rule registry hoặc framework nếu helper cụ thể đủ dùng.

## 7. Minimalism và simplification

- Giữ `collect_output`, `PathSearch` và distinct collectors; chúng có contract thật.
- Giữ direct SQL loaders không side effects; không gọi public `callers` từ `context` để tái sử dụng.
- Decomposition của impact/clones/test traversal đã tốt hơn. Physical lengths không tự chứng minh effective-LOC hoặc cognitive-complexity compliance.
- Search vẫn coupling token, phrase, morphology, ranking, exact merge, counts và rendering; sửa accounting trước, refactor riêng.
- R3–R6/M1 được defer là hợp lý về scope. Chúng không nên bị nâng thành correctness blocker chỉ vì chưa refactor.
- Không bỏ validation, error handling, journal opt-out hoặc caveats để giảm số dòng.

RNEW5 là old R6, không phải một defect mới. Report không cộng findings trùng để phóng đại backlog.

## 8. Tests, documents và evaluation

Test suite mạnh hơn ở exact old fixtures, symlink guards, MCP recovery và data-preservation checks. Nhưng một số assertions chỉ xác nhận phần call graph hoặc một lexical shape; chưa chứng minh general binding correctness hay cross-file carry-forward.

Process inspection hiện kiểm tra command status và seeded marked child. Nó vẫn không chứng minh mọi detached/unmarked descendant được theo dõi; network negative control cũng cần outside-isolation positive control.

Public docs đã sửa nhiều mâu thuẫn cũ. Tính năng `callers --depth` thêm semantics mới cần ghi rõ shortest-symbol filtering, seed exclusion và verifier behavior; trust wording không được cấm safety/review verification.

Review evaluation bổ sung không giữ ENEW1–ENEW3 làm findings: unknown filename crash là fail-closed; missing recorded answers được tính incorrect; transcript-shape accusation chưa có recorded counterexample. Không đổi estimator sang successful-only chỉ để số đẹp hơn.

P9 evidence được lấy mẫu có sáu validation cells, không phải toàn bộ planned 80-cell battery. Kết quả đó và số đo lịch sử không chứng minh current commit có net agent-utility win.

Chưa chạy utility benchmark mới, Linux runtime, MSRV 1.90, live GitHub release hoặc full dependency vulnerability/license audit. Chỉ fetch pinned public action metadata để kiểm tra required input; không cài hoặc thực thi action.

## 9. Có đáng để agent dùng không?

| Workflow | Verdict |
|---|---|
| Navigation hints trên repository tin cậy, source được kiểm tra | Có điều kiện |
| Ground truth mặc định, cấm agent re-verify | Chưa nên |
| Incremental graph làm basis cho automated impact/test completeness | Chưa đạt; equivalence còn counterexamples |
| “Không có caller/path” suy ra không có quan hệ thật | Không hợp lệ |
| Repository/working tree hoặc `.aw` có actor khác kiểm soát | Không dựa vào blanket confinement claim |
| MCP production session với local log filesystem không kiểm soát | Chưa coi là robust vì special-file blocking |
| Khẳng định giảm token/cost tổng quát | Chưa đủ current-snapshot evidence |

Forced build tránh được một số incremental discrepancies nhưng **không sửa binding bugs, hardlink/FIFO hazards hoặc query accounting**. Không dùng nó như workaround toàn diện.

## 10. Ưu tiên và điểm dừng

1. Data preservation và consistency: ANEW1–ANEW3, QNEW-A3, SNEW1–SNEW3, TNEW1.
2. Binding invariant: CNEW1–CNEW10, có exact source-site positives và negatives.
3. Accounting và contract synchronization: RNEW1–RNEW3, DNEW1–DNEW3, TNEW2–TNEW3.
4. Bounded inputs: ANEW4–ANEW5; kiểm chứng bằng controlled tests trước large jobs.
5. Behavior-preserving simplification và evaluation mới chỉ sau khi correctness ổn định.

Đây là backlog, không phải authorization sửa code. Active-local-attacker threat model của SNEW4 cần owner quyết định; review không tự thu hẹp safety claim hoặc tự chọn architecture mới.

Acceptance của report: source hash/provenance, closure cardinality, unique finding IDs, evidence schema, privacy patterns, relative links và whitespace checks. Không chạy lại toàn suite chỉ vì thêm Markdown.

Final independent report review bằng `openai/gpt-6.1-terra`: không có Important/Critical report findings. Reviewer xác nhận readable action metadata, closure cardinality, finding exclusions và relative links; không chạy lại probes hoặc tests.
