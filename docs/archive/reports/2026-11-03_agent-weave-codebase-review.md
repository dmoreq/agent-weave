# Review chuyên sâu agent-weave: mức độ sẵn sàng cho coding agent

## Executive summary

**`aw` có giá trị như công cụ điều hướng bổ trợ, nhưng snapshot được review chưa phù hợp làm nguồn quan hệ mặc định đáng tin cậy cho agent.** Năm gate đều pass, nhưng probe độc lập vẫn phát hiện ghi ngoài repository, edge sai và incremental build không tương đương forced build.

Không cần viết lại architecture hoặc thay tech stack. Cần sửa boundary, binding semantics và invalidation trước khi mở rộng routing bắt buộc hoặc tuyên bố hiệu quả tổng quát.

## 1. Snapshot và giới hạn

| Thuộc tính | Phạm vi thực tế |
|---|---|
| Ngày review | 2026-11-03 |
| Commit | `1439a9e63f6483343ce69bfa2cda1369ed451be3` |
| Sản phẩm | `agent-weave` 0.3.0; binary `aw` |
| Nền tảng kiểm chứng | macOS; `rustc 1.94.1` |
| Isolation | Snapshot tạo bằng `git archive HEAD`; build target và fixture nằm ngoài checkout |
| Runtime source | Đọc đầy đủ 22 file `src/**/*.rs`, tổng 12.741 dòng |
| Integration tests | Đọc đầy đủ ba file Rust, tổng 5.232 dòng |
| Thay đổi chưa commit | Loại khỏi verdict; bao gồm benchmark và agent skill đang được chỉnh sửa |
| Runtime changes từ review | Không có; không sửa code, không commit, không publish |

Lệnh lấy provenance: `git rev-parse HEAD`, `git status --porcelain=v1 -uall`, `git ls-files -z`. Số dòng được đếm trực tiếp từ snapshot, không lấy từ index `aw`.

Reviewer nhận snapshot không có `.git`. Controller xác định commit và tạo content manifest; không coi lời tự nhận revision của reviewer là bằng chứng độc lập.

Toàn bộ 1.859 file tracked được inventory: 98 file đọc đầy đủ, 2 file lấy mẫu, 125 file chỉ kiểm tra metadata và 1.633 artifacts không revalidate. Canonical design được đọc hết line range nhưng một số dòng dài bị tool rút gọn.

Toàn bộ 34 Python/shell files trong baseline, một extensionless harness và các test/safety scripts được đọc. Không coi inventory của historical Markdown hoặc artifacts là chứng nhận đúng từng nội dung.

Chi tiết trạng thái từng file nằm trong [coverage.csv](assets/2026-11-03_agent-weave-codebase-review/coverage.csv). Path nhạy cảm được thay bằng path hash; manifest không phát hành lại định danh nội bộ.

## 2. Cách review và phản biện

Tám luồng đọc độc lập, bốn lượt phản biện chuyên biệt và controller tái hiện qua consumer thực tế. Reviewer không nhận kết luận cũ hoặc lý giải của tác giả ở lượt đọc đầu.

| Luồng | Model đọc chính | Phản biện |
|---|---|---|
| Architecture, storage, build lifecycle | `openai/gpt-6.1-terra` | `openai/gpt-6-luna`; controller so sánh build |
| Extraction và resolution | `openai/gpt-6.1-sol` | `openai/gpt-6-luna`; controller chạy fixture |
| Security, MCP, resource boundaries | `openai/gpt-6.1-sol` | `openai/gpt-6.1-terra` |
| Minimalism và design pattern | `openai/gpt-6-luna` | `openai/gpt-6.1-terra` |
| Simplification và reuse | `openai/gpt-6-astra` | `openai/gpt-6.1-terra` |
| Public documents và contracts | `openai/gpt-6-astra` | `openai/gpt-6.1-sol` |
| Evaluation và historical harness | `openai/gpt-6-luna` | `openai/gpt-6.1-sol` |
| Test integrity và safety enforcement | `openai/gpt-6.1-terra` | `openai/gpt-6.1-sol` |

Đây là phân công, không phải benchmark năng lực model. Bốn model cùng provider vẫn có thể chung điểm mù; số phiếu không chứng minh finding.

Finding có ba mức bằng chứng: `REPRODUCED` qua binary; `CONTROLLED` qua hàm thật với external boundary được thay thế; `STATIC` qua source. Không trình bày hướng sửa là patch đã được kiểm chứng.

## 3. Kết quả kiểm tra nền

| Lệnh trên snapshot | Kết quả | Thời gian của lần chạy này |
|---|---|---:|
| `cargo fmt --all -- --check` | PASS | 0,71 s |
| `cargo clippy --locked --all-targets -- -D warnings` | PASS | 18,37 s |
| `cargo test --locked` | PASS | 18,04 s |
| `bash tests/scripts/allowlist_check.sh Cargo.lock` | PASS | 0,08 s |
| `bash tests/scripts/write_path_check.sh` | PASS | 0,30 s |

Môi trường: `CARGO_NET_OFFLINE=true`, `CARGO_BUILD_JOBS=4`, target directory cô lập. Thời gian bao gồm overhead/build trong môi trường này, không phải performance benchmark của `aw`.

Test runner ghi nhận `26 lib + 26 binary + 115 CLI + 34 fixtures + 8 MCP = 209` lượt pass. Nhóm 26 unit test chạy ở cả lib và binary; không gọi đây là 209 test khác nhau.

Không chạy lại toàn suite sau mỗi finding. Các probe bổ sung dùng binary từ chính lần build này; không sửa snapshot để tạo kết quả xanh.

## 4. Architecture và tech stack

**Architecture hiện tại phù hợp với một local static index.** Các điểm yếu chính nằm ở semantics và enforcement, không phải thiếu framework.

| Thành phần | Đánh giá | Giới hạn cần giữ rõ |
|---|---|---|
| Rust đồng bộ | Phù hợp CLI và MCP stdio theo request | Chưa có bằng chứng cần async runtime |
| Tree-sitter | Phù hợp trích xuất syntax đa ngôn ngữ, không chạy source | Không thay type checker, runtime analysis hoặc language server |
| SQLite + FTS5 | Phù hợp local graph, lookup, rebuild atomic | Schema/version đúng không bảo đảm quan hệ đúng |
| Extraction → resolution → storage → query | Ranh giới trách nhiệm rõ | Binding tables chưa giữ đủ lexical identity và thay đổi binding |
| Temporary DB + fsync + rename | Cách publish hợp lý | Lock và filesystem confinement chưa được bảo vệ đầy đủ |
| Explicit ambiguity và caveats | Có ích cho agent | Một số heuristic phát edge chắc chắn khi receiver chưa chắc chắn |
| CLI/MCP chung query layer | Giảm khác biệt semantics giữa hai surface | Lỗi query và mô tả sai cũng lan ra cả hai surface |

Nguồn: `src/{build,db,extract,query,mcp}.rs`, các module con, `Cargo.toml`, `schema.sql`, `schema_indexes.sql`.

Không bổ sung plugin registry, generic graph framework, async runtime hoặc dependency mới để xử lý các finding hiện tại. Kiểm tra allowlist chỉ xác nhận tên package; review này không chứng nhận CVE, license của toàn bộ transitive graph hoặc MSRV 1.90 trên môi trường thật.

## 5. Findings ảnh hưởng trực tiếp tới agent

Danh sách đầy đủ, source location, tình huống, hướng sửa và regression criteria nằm trong [finding register](2026-11-03_agent-weave-review-findings.md). Các finding trùng được hợp nhất; `A1` trỏ tới `S1`, `R7` trỏ tới `S2`.

Register giữ **37 accepted findings: 1 Critical, 28 Important, 8 Suggestion**. Hai đề xuất khác được `CONTEST` bằng contract đọc được và không cộng vào defect totals; không tự đổi severity khi phản biện.

| Nhóm | Finding | Ảnh hưởng |
|---|---|---|
| Filesystem boundary | `S1` | `.aw` hoặc lock-file symlink gây ghi ngoài repository; sentinel ngoài index bị thay đổi |
| Incremental correctness | `A2`, `A3` | Cùng source cuối cùng nhưng build thường và `--force` cho graph khác nhau |
| Lexical binding | `C1`, `C2`, `C3`, `C7` | Import, parameter hoặc assignment làm edge trỏ tới đích sai |
| Symbol/receiver identity | `C4`, `C6`, `C8` | Factory Rust, local class Python và class ambiguity làm mất identity |
| Call ownership | `C5` | Arrow function tạo thêm caller ở module dù không có execution tương ứng |
| Query failure | `S2` | `unresolved --path` có kết quả thoát `6` vì thiếu SQL binding |
| MCP recovery | `S3`, `S4` | Invalid UTF-8 làm server thoát; frame đúng biên cap làm mất request kế tiếp |
| Concurrent maintenance | `S5` | Invalidation cho phép đồng thời giữ hai lock inode khác nhau |
| Build work bound | `A4` | Scan comment ranges có quadratic work; chưa đo throughput |

**False positive nguy hiểm hơn một kết quả thiếu đã được cảnh báo.** Agent có thể dùng edge sai để bỏ sót source cần đọc, chọn sai test hoặc suy luận sai về impact.

Ví dụ đã tái hiện:

```text
Sau khi bỏ B(A): build thường vẫn trả caller của A.foo.
Cùng source, build --force: 0 callers; unresolved tăng từ 0 lên 1.

Hai local import khác nhau: cả left và right bị gán tới b.f.
Arrow function: một call site xuất hiện với cả owner run và module.

unresolved --path src:
aw: internal: database error: Wrong number of parameters passed to query. Got 0, needed 1
```

Chi tiết input, command và stdout/stderr nằm trong [evidence.json](assets/2026-11-03_agent-weave-codebase-review/evidence.json). Đây là bug reproduction trên fixture nhỏ, không đo tần suất lỗi trong repository thực tế.

## 6. Minimalism, design pattern và simplification

**Giữ các abstraction đang mang contract; giảm phần orchestration và cơ chế phải sửa đồng bộ.** Không áp dụng DRY khi hai collector có semantics khác nhau.

| Giữ | Lý do |
|---|---|
| `collect_output` | Chung result cap, displayed-file staleness và payload capture |
| `PathSearch` | Phân biệt graph cạn với depth cap |
| `TestTop` và `ImpactTop` riêng | Một bên dedupe symbol, một bên chọn đại diện theo file |
| Side-effect boundary tập trung | Dễ audit hơn khi không phân tán filesystem writes |
| Caveats, ambiguity, fallback | Là safeguard, không phải wording dư thừa |

| Cải thiện | Finding | Điều kiện |
|---|---|---|
| Tách validation/traversal/rendering theo phase hiện có | `R1` | Giữ output bytes, counts, caps và journal |
| Chia sẻ direct-edge loaders giữa context và direct queries | `R2` | Loader không có side effect; không gọi public query từ context |
| Dùng helper empty-detail hiện có | `R4` | Không đổi thông điệp theo edge kind |
| Giảm lặp registration fresh/carried | `M1` | Characterize identity và ordering trước |
| Giảm drift scope/extension/target guards | `R3`, `R5`, `R6` | Giữ khác biệt semantics có chủ ý |

`impact`, `clones`, `test_impact` vượt hard threshold 50 effective lines của repository. Metric review dùng lower bound dòng bắt đầu bằng control/declaration, không gọi đó là cyclomatic hoặc cognitive complexity.

Sửa behavioral defects và refactor phải là các thay đổi riêng. Trừu tượng hóa binding trước khi hiểu scope và invalidation có thể chỉ che lỗi hiện tại.

## 7. Tests và safety gates

**Test suite hữu ích nhưng chưa chứng minh các invariant mạnh nhất.** Một lần pass không bác bỏ lỗi được tái hiện trên input khác.

- `T1`: hai cặp test dùng chung directory theo tên và PID; có race khi chạy song song.
- `T2`: exporter xóa destination có dữ liệu trước khi xác nhận archive thành công.
- `T3`: assertion về process survival có thể pass khi inspection không hoạt động.
- `T4`: privacy scanner không phân biệt lỗi thực thi với không có match.
- `T5`: fixture Rust làm mất indentation Python nên negative case không đáng tin như mô tả.

Write-path checker kiểm tra vị trí primitive trong source, không chứng minh containment sau symlink resolution. Network-denied CI chứng minh build hoạt động khi chặn egress, không chứng minh binary chưa từng thử kết nối.

Các regression ưu tiên: fresh/incremental equivalence sau sửa base hoặc thêm candidate; local binding/shadowing; byte framing recovery; external sentinels; lock identity xuyên maintenance. Chưa kiểm chứng process-kill ở thời điểm publish, Linux runtime, MSRV hoặc mọi MCP host.

## 8. Public documents và wording

Public guides đã có caveat tốt: static navigation không chứng minh runtime behavior, complete coverage hoặc test adequacy. Cần sửa các mâu thuẫn ngay tại nơi agent khám phá tool.

| Finding | Mâu thuẫn | Hậu quả |
|---|---|---|
| `D1` | Release job thiếu repository context | Rủi ro publish thất bại; workflow chưa được chạy trong review |
| `D2` | `force` help chỉ nói re-extraction | Agent không biết parse-quality gate cũng bị bỏ qua |
| `D3` | Invalidation help nói xóa index | Không thể hiện việc xóa toàn bộ `.aw/`, gồm journals |
| `D4` | Changelog còn định danh corpus nội bộ | Vi phạm public-release hygiene; không lặp lại định danh trong báo cáo |
| `D5` | Context quảng cáo resolved-only | Thực tế có ambiguous callee, dù output có tag |
| `D6` | Docs phủ nhận qualified suffix | Resolver thực tế hỗ trợ dotted suffix khi exact lookup rỗng |
| `D7` | Repository rules gán parse abort exit `4` | Runtime và CLI reference dùng `7`; không tự đổi runtime theo rule cũ |

Đây không phải đề nghị viết lại toàn bộ tài liệu. Đồng bộ discovery descriptions, error contract và claim scope trước; giữ lịch sử benchmark nhưng không giữ thông tin nhạy cảm.

## 9. Bằng chứng về agent utility

**Chưa có benchmark mới chứng minh lợi ích ròng của snapshot này.** Kết quả lịch sử là tín hiệu có điều kiện, không phải acceptance proof cho current binary.

`docs/evaluation.md` phân biệt navigation baseline, feature ablation, host/model dependence và private evidence. Pointer authenticity không đo missing edges; unresolved-site share không bằng query recall.

`E1` cho thấy scorer chấp nhận duplicate dù protocol cấm. Probe chứng minh bug acceptance logic; không chứng minh dữ liệu lịch sử thực sự chứa duplicate hoặc mọi số đo đều sai.

`H1` cho thấy historical search scorer chấp nhận missing/null `hops` dù contract yêu cầu `0`. Ngược lại, không nhận `H2` và `H3` làm defects: pointer-shape metric đã công bố giới hạn; all-cell economics có thể đúng theo estimator đã công bố.

Scorer không tìm thấy answer vẫn trả thành công; summarizer không tự chứng minh đủ planned cells. Đây là giới hạn của công cụ chứng nhận, không phải bằng chứng rằng báo cáo đã che mất run.

Các artifacts lịch sử có trong checkout không đồng nghĩa public distribution cho phép tái hiện tất cả số đo. Không chạy private corpora, không đọc thay đổi benchmark chưa commit và không chuyển một kết quả task riêng thành verdict toàn sản phẩm.

Một evaluation mới cần ba nhánh `no aw`, `aw available`, `aw routed`; cùng binary, source và ngân sách. Oracle độc lập từ source, task holdout, isolation, failed-run accounting và cold/warm costs phải được đóng băng trước.

Không đặt ngưỡng tiết kiệm mới sau khi thấy kết quả. Owner cần chốt mức cải thiện thực tế và ngân sách trước chiến dịch đo; review này không tự khởi chạy chiến dịch đó.

## 10. Có đáng để agent sử dụng không?

| Workflow | Verdict cho snapshot được review | Điều kiện |
|---|---|---|
| Điều hướng bổ trợ trên repository tin cậy | Có điều kiện | Đọc source tại pointer; kiểm tra scope, ambiguity và index freshness |
| Query-first bắt buộc trên mọi repository | Chưa nên | Sửa boundary và false-positive resolution trước |
| Tự động chọn đầy đủ tests hoặc impact | Không dùng làm nguồn duy nhất | Graph tĩnh thiếu quan hệ và hiện có edge sai |
| Xóa dead code hoặc quyết định tương đương clone | Không | Candidate list không phải proof |
| MCP session cần recovery ổn định | Chưa nên coi là robust | Sửa framing `S3`, `S4`; kiểm tra host thực tế |
| Index repository không tin cậy | Không với filesystem permissions rộng | `S1` đã ghi được ra ngoài repository |
| Khẳng định giảm token/cost tổng quát | Chưa đủ bằng chứng | Cần same-snapshot evaluation độc lập và reproducible |

“Repository tin cậy” không tự sửa lỗi resolution. Đây chỉ là một điều kiện giảm rủi ro filesystem; source verification vẫn bắt buộc.

## 11. Thứ tự xử lý

1. **Boundary và data preservation:** `S1`, `S5`, `T2`, `T4`. Chặn ghi ngoài root, lock split và export fail-open.
2. **Graph correctness:** `A2`, `A3`, `C1`–`C8`, `S2`. Thêm cả positive và negative assertions.
3. **MCP recovery và build cost:** `S3`, `S4`, `A4`. Kiểm tra delimiter, UTF-8 và complexity bound.
4. **Test/release/contracts:** `T1`, `T3`, `T5`, `D1`–`D7`, `E1`, `H1`. Sửa bằng chứng trước khi dùng nó làm gate.
5. **Behavior-preserving simplification:** `R1`–`R6`, `M1`, sau characterization và tách khỏi bug fixes.
6. **Đo utility mới:** chỉ sau khi protocol, oracle và ngân sách được owner chốt.

Đây là backlog review, không phải authorization triển khai. Audit hoàn tất không có nghĩa snapshot đã được chấp nhận để release; các finding quan trọng vẫn mở.

## 12. Acceptance cho báo cáo

- Source locations thuộc commit được review, không thuộc working tree đang thay đổi.
- Từng finding có evidence level và regression criterion trong register.
- Findings trùng không bị tính hai lần; phản biện không tự đổi severity.
- Coverage nêu rõ full read, sampled, metadata-only và data inventory.
- Kiểm tra liên kết tương đối, evidence schema, finding IDs, manifest hashes và `git diff --check` trước bàn giao.

Final independent report review: `openai/gpt-6-astra`, **No findings**. Reviewer kiểm tra counts, artifacts, privacy patterns và source references; không chạy lại tests hoặc probe.

Không có code fix, benchmark utility mới, release operation hoặc durable-memory mutation trong phiên review này.
