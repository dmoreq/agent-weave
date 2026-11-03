# Finding register: agent-weave deep review

## Contract và reconciliation

Snapshot: `1439a9e63f6483343ce69bfa2cda1369ed451be3`, ngày 2026-11-03. Đây là backlog chỉ đọc, không phải patch hoặc bằng chứng đã sửa lỗi.

`REPRODUCED`: controller chạy binary của snapshot. `CONTROLLED`: chạy hàm thật với fixture hoặc external boundary được thay thế. `STATIC`: trace source; chưa tái hiện end-to-end.

Tất cả accepted findings dưới đây đã qua phản biện độc lập. Disposition là `PROPOSE FIX`; Suggestions là tùy chọn, không phải release blocker. Không có finding nào được đóng bằng việc sửa code trong review này.

`A1` hợp nhất vào `S1`; `R7` hợp nhất vào `S2`. Không tính hai lần các lỗi này. Bằng chứng thực thi nằm trong [evidence.json](assets/2026-11-03_agent-weave-codebase-review/evidence.json).

## Security, protocol và query

### S1 — Critical — Symlink phá filesystem confinement

- Location: `src/db.rs#L48-L54`, `#L64-L100`, `#L146-L150`; `src/journal.rs#L195-L202`.
- Evidence: `REPRODUCED`, cases `directory-symlink`, `lock-symlink`. Directory symlink tạo `.gitignore`, `build.lock`, `index.db` ở target ngoài repository; file symlink làm sentinel ngoài index bị ghi đè.
- Scenario/impact: repository có `.aw` hoặc `build.lock` do người khác tạo; agent gọi build với quyền của host. Không cần race để gây ghi ngoài boundary. Journal symlink là biến thể static, chưa chạy probe riêng.
- Correction: thiết lập directory/file boundary không follow symlink, có chính sách xử lý replacement races; không chỉ kiểm tra chuỗi path.
- Acceptance: directory, lock và journal symlink qua CLI/MCP không thay đổi external sentinels. Kiểm tra cả target đã tồn tại và target chưa tồn tại.

### S2 — Important — `unresolved --path` thiếu SQL parameters

- Location: `src/query/analysis.rs#L468-L507`; các argument rỗng tại `#L479`, `#L497`, `#L505`.
- Evidence: `REPRODUCED`, case `scoped-unresolved`; exit `6`: `Wrong number of parameters passed to query. Got 0, needed 1`.
- Scenario/impact: scope có unresolved call. Ba query dùng `?1` nhưng không bind; command thất bại thay vì trả report. MCP mở rộng gọi cùng query layer.
- Correction: truyền `scope_params` cho cả ba query.
- Acceptance: nonzero scope thành công; không đưa tên/count ngoài scope vào report; giữ unscoped và empty-result behavior.

### S3 — Important — Invalid UTF-8 làm MCP server thoát

- Location: `src/mcp.rs#L113-L119`, recovery JSON tại `#L145-L150`.
- Evidence: `REPRODUCED`, case `invalid-utf8`: bytes `FF 0A`, sau đó ping `id=2`; exit `6`, không có ping response.
- Scenario/impact: đọc vào `String` trả I/O error trước JSON recovery; một frame lỗi kết thúc session.
- Correction: frame bytes có giới hạn; decode từng frame hoàn chỉnh và trả protocol error mà không kết thúc session.
- Acceptance: invalid UTF-8 và multibyte ở cap boundary đều giữ được response cho ping tiếp theo.

### S4 — Important — Oversized frame đúng biên làm mất request kế tiếp

- Location: `src/mcp.rs#L112-L139`.
- Evidence: `REPRODUCED`, cases `oversized-frame-boundary`, `oversized-frame-later-newline`.
- Scenario/impact: `8.388.608` ASCII spaces + newline + ping. Read đầu đã lấy newline nhưng drain tiếp qua newline của ping. Server chỉ trả `frame too large`; control dài hơn bốn spaces vẫn trả ping.
- Correction: chỉ drain khi delimiter chưa nằm trong phần đã đọc.
- Acceptance: test delimiter ngay tại cap + 1 và sau đó; frame bị từ chối không làm mất request kế tiếp.

### S5 — Important — Invalidation phá identity của build lock

- Location: `src/doctor.rs#L11-L15`; `src/db.rs#L64-L100`; build lock acquisition `src/build.rs#L23`.
- Evidence: `REPRODUCED` ở lock primitive, case `locked-invalidate`: giữ flock cũ, invalidate, rồi giữ đồng thời flock mới. Chưa tái hiện corruption do hai build thật.
- Scenario/impact: directory removal unlink inode đang bị lock; builder khác tạo inode mới và vào critical section. Hai publisher không còn dùng cùng lock identity.
- Correction: maintenance và build phải chia sẻ stable lock identity, kể cả khi process đang giữ hoặc đợi lock.
- Acceptance: invalidation refuse/wait khi build hoạt động; builder thứ hai không được vào trong maintenance; thêm deterministic interleaving test.

## Build và invalidation semantics

### A2 — Important — Removed inheritance vẫn tham gia incremental resolution

- Location: `src/db.rs#L524-L536`; `src/extract/resolve.rs#L253-L255`, `#L1040-L1058`.
- Evidence: `REPRODUCED`, case `removed-base`. Sau đổi `class B(A)` thành `class B`, build thường vẫn trả `sample.py:9 sample.invoke` cho `A.foo`; force build trả exit `1`.
- Scenario/impact: old inherits được load từ cả changed files, không chỉ carried files; graph giữ quan hệ không còn trong source.
- Correction: chỉ đưa quan hệ còn hợp lệ của carried sources vào resolver; không merge old relation từ changed source.
- Acceptance: remove/change base và so toàn bộ normalized edge sets giữa incremental và fresh build; giữ inherited-call positive control.

### A3 — Important — Added candidate không invalidate resolved edges

- Location: `src/build.rs#L165-L205`; `src/db.rs#L392-L457`.
- Evidence: `REPRODUCED`, case `added-candidate`: thêm package initializer có cùng qualname; build thường giữ `pkg.py:1 pkg.foo`, force build trả `[ambiguous] foo`.
- Scenario/impact: dirty/gap healing chỉ xem qualname mới và unresolved/ambiguous sites. Candidate multiplicity thay đổi nhưng carried resolved edge vẫn chắc chắn.
- Correction: invalidate affected resolutions khi candidate set thay đổi, kể cả previously resolved sites.
- Acceptance: added duplicate tạo cùng ambiguity ở fresh/incremental build; unrelated addition không gây invalidation quá rộng.

### A4 — Important — Comment scan có quadratic work

- Location: `src/parse.rs#L318-L339`; build consumer `src/build.rs#L588`.
- Evidence: `STATIC`, phản biện xác nhận. Mỗi non-whitespace byte chạy lại `merged.iter().any` từ đầu; nhiều comment ranges riêng làm số lần so sánh tăng bậc hai.
- Scenario/impact: comment-heavy source dưới size cap vẫn tạo chi phí lớn. Chưa đo wall time; không gọi đây là measured latency regression.
- Correction: advance cursor qua sorted merged ranges khi scan source.
- Acceptance: flags giữ nguyên cho mixed/comment-only/touching/multiline cases; instrumentation hoặc bounded measurement chứng minh work tăng gần tuyến tính.

## Extraction và resolution

Tất cả `C1`–`C8` là **Important**, đã `REPRODUCED` bằng source fixture nhỏ và được model khác xác nhận. Các edge sai có thể lan tới callers, path, impact và test candidates; prevalence trên corpus thực tế chưa đo.

### C1 — Function-local import bindings bị gộp theo file

- Location: `src/extract/walk.rs#L647-L650`; `src/extract/resolve.rs#L147-L150`, `#L707-L711`.
- Scenario/actual: `left` import `a.f`, `right` import `b.f`; cả hai trở thành caller của `b.f`, còn `a.f` không có caller. Case `local-imports`.
- Correction/acceptance: giữ lexical scope của import, resolve nearest binding. Assert đúng cả hai destination và không có cross-edge; kiểm tra block-local imports tương ứng.

### C2 — Parameter không shadow module function

- Location: `src/extract/resolve.rs#L679-L711`; binding extraction `src/extract/walk.rs#L668-L731`.
- Scenario/actual: `def run(target): target()` trở thành caller của module-level `target`. Case `parameter-shadow`.
- Correction/acceptance: track parameter/local bindings độc lập với type inference; unknown callable binding phải chặn outer lookup. Thêm Python/TS callback negatives và unshadowed positive.

### C3 — Unknown reassignment không xóa receiver type cũ

- Location: `src/extract/walk.rs#L768-L802`; `src/extract/resolve.rs#L1070-L1083`.
- Scenario/actual: `obj = A(); obj = other; obj.m()` vẫn trỏ tới `A.m`. Case `receiver-reassign`.
- Correction/acceptance: ghi nhận unknown binding khi RHS không xác định type. Assert không còn `A.m` edge, đồng thời giữ constructor-to-constructor positive.

### C4 — Rust associated factory bị coi là constructor

- Location: `src/extract/walk.rs#L823-L890`.
- Scenario/actual: `A::make() -> B`, rồi `value.render()` lại trỏ tới `A.render`. Case `rust-factory`.
- Correction/acceptance: không suy type `A` từ mọi associated function. Explicit annotation hoặc constructor evidence được hỗ trợ mới tạo receiver certainty; assert không có `A.render`, giữ constructor positive.

### C5 — Arrow body bị walk hai lần với owner khác nhau

- Location: `src/extract/walk.rs#L445-L447`, `#L1617-L1619`; arrow-body traversal trong `record_ts_arrow_function`.
- Scenario/actual: `const run = () => helper()` tạo hai caller ở cùng site: `sample.run` và `sample`. Case `arrow-owner` trên TypeScript.
- Correction/acceptance: initializer đã được arrow handler consume không được outer walk lần nữa. So complete call-edge sets, owner và site trên cả TS và JS; JS chưa chạy probe riêng.

### C6 — Methods trong function-local class mất class qualname

- Location: `src/extract/walk.rs#L533-L542`, `#L592-L634`.
- Scenario/actual: `outer.C.m` và `outer.C.run` thành `outer.m`, `outer.run`; `self.m()` không có callee. Case `nested-class`.
- Correction/acceptance: lexical parent ordering phải thể hiện class nằm trong function. Assert qualnames, contains parents và `C.run -> C.m`; giữ nested-function-inside-method control.

### C7 — Typed receiver thất bại rồi fallback tới module bị shadow

- Location: `src/extract/resolve.rs#L786-L801`, `#L883-L886`.
- Scenario/actual: local `tools = C()` không có method `execute`; `tools.execute()` bị gán tới imported module `tools.execute`. Case `receiver-module-shadow`.
- Correction/acceptance: local receiver interpretation phải chặn module interpretation, kể cả khi method unresolved. Assert không có module edge; giữ known receiver method positive.

### C8 — Class ambiguity mất trước method-confidence decision

- Location: `src/extract/resolve.rs#L114-L127`, `#L1040-L1063`, `#L1193-L1197`.
- Scenario/actual: hai conditional class declarations cùng qualname `A`, chỉ một class có `render`; `value: A` được resolve chắc chắn tới method đó. Case `class-ambiguity`.
- Correction/acceptance: giữ class candidate identity, không gộp certainty theo qualname. Assert ambiguous/unresolved khi receiver class không duy nhất; giữ unique-class positive.

## Public documents, packaging và contracts

### D1 — Important — Release publish job thiếu repository context

- Location: `.github/workflows/release.yml#L53-L63`.
- Evidence: `STATIC`. Job mới chỉ download artifacts; không checkout, không `GH_REPO`, không `--repo` cho `gh release create`.
- Scenario/impact: release CLI không có repository local để suy destination; publication có thể thất bại sau build thành công. Chưa chạy GitHub workflow.
- Correction/acceptance: truyền repository context tường minh; controlled tag publication phải chọn đúng repo và đủ assets.

### D2 — Important — `force` discovery descriptions giấu parse-gate bypass

- Location: `src/cli.rs#L29-L36`, `src/mcp.rs#L35-L39`; implementation `src/build.rs#L210-L227`.
- Evidence: `REPRODUCED`, case `parse-force`: malformed fixture exit `7` không force, exit `0` với force dù threshold `0.05`.
- Scenario/impact: agent muốn re-extract nhưng tưởng parse-quality threshold còn được enforce.
- Correction/acceptance: CLI help và MCP schema phải mô tả cả re-extraction và bypass; giữ validation của threshold và test hai chế độ.

### D3 — Important — Invalidation help không nêu toàn bộ deletion scope

- Location: `src/cli.rs#L236-L240`; `src/doctor.rs#L11-L19`.
- Evidence: `STATIC`, đối chiếu generated help được capture. “Delete the index entirely” thực tế là recursive deletion của `.aw/`.
- Scenario/impact: user không biết journals và mọi nội dung khác trong directory cũng bị xóa.
- Correction/acceptance: help nói rõ entire directory and contents; disposable fixture xác nhận phạm vi. Không suy từ finding này rằng invalidation follow symlink để xóa target ngoài root.

### D4 — Important — Changelog vi phạm public-release privacy rule

- Location: `CHANGELOG.md#L474-L500`, `#L576-L580`, `#L668-L675`; rule `AGENTS.md#L53-L56`.
- Evidence: `STATIC`, hai reviewer xác nhận các private corpus identifiers; không lặp lại chúng trong deliverable.
- Scenario/impact: phát hành changelog công khai làm lộ định danh bị rule cấm, dù phép đo được gọi là historical.
- Correction/acceptance: dùng neutral corpus labels nhất quán; quét complete public distribution và xử lý scanner errors theo fail-closed.

### D5 — Important — Context quảng cáo resolved-only nhưng chứa ambiguous callees

- Location: `src/cli.rs#L57-L58`, `src/mcp.rs#L46-L47`; query `src/query/callgraph.rs#L951-L980`, rendering `#L1004-L1013`.
- Evidence: `REPRODUCED`, `context invoke` trong case `added-candidate` trả `[ambiguous] foo` dưới header resolved-only.
- Scenario/impact: tool discovery đưa promise mạnh hơn inclusion semantics. Output có tag, nên không gọi đây là ambiguity bị che giấu.
- Correction/acceptance: đồng bộ descriptions với actual semantics; fixture mixed resolved/ambiguous phải có counts và tags nhất quán.

### D6 — Suggestion — Docs phủ nhận supported dotted suffix

- Location: `docs/cli-reference.md#L34`; `skills/aw-first/SKILL.md#L66-L70`; implementation `src/query.rs#L192-L219`.
- Evidence: `STATIC`. Resolver có suffix fallback khi exact lookup rỗng; multiple matches vẫn ambiguous.
- Correction/acceptance: nói rõ precedence và supported form; unique/multiple suffix và exact precedence phải khớp docs/tests. Không mô tả thành arbitrary substring search.

### D7 — Suggestion — Repository rules và runtime lệch parse-abort exit code

- Location: `AGENTS.md#L40-L41`; `src/error.rs#L72-L83`.
- Evidence: `REPRODUCED` runtime exit `7` qua `parse-force`; rule gán parse abort `4`, trong khi runtime dùng `4` cho lock.
- Correction/acceptance: reconcile rule với approved contract, public reference và tests. Không tự thay runtime để khớp rule cũ.

## Test integrity và evaluation

### E1 — Important — P8 scorer chấp nhận duplicate bị protocol cấm

- Location: `docs/baselines/p8-utility-2026-11-03/score_p8.py#L45-L47`; sibling `protocol.md#L59-L67`.
- Evidence: `CONTROLLED`, case `duplicate-score`: oracle `['x']`, answer `['x','x']` trả `(True, 'exact set match')`.
- Scenario/impact: set conversion bỏ duplicate trước validation; correctness có thể bị tăng nếu dữ liệu chứa dạng answer này. Chưa chứng minh answer lịch sử thực sự có duplicate.
- Correction/acceptance: reject normalized duplicates trước equality; correct answer pass, same answer thêm duplicate fail. Báo planned/missing cells riêng thay vì dùng scorer exit code chứng nhận completeness.

### T1 — Important — Parallel tests dùng chung fixture directories

- Location: `tests/cli.rs#L9-L13`, `#L353-L371`, `#L2635-L2644`, `#L3682-L3695`.
- Evidence: `STATIC`. Hai cặp test dùng cùng tên `path-invalid` và `complexity-path`; PID không phân biệt test trong cùng binary.
- Scenario/impact: `temp_repo` xóa directory của test khác đang chạy. Lần suite pass không loại trừ race.
- Correction/acceptance: directory unique theo invocation; targeted concurrent runs không chia sẻ paths hoặc xóa fixtures của nhau.

### T2 — Important — Exporter xóa nonempty destination trước validation

- Location: `tests/scripts/public_export.py#L93-L101`, `#L210-L216`.
- Evidence: `REPRODUCED`, case `nonempty-export`: sentinel bị xóa trước khi invalid archive source trả exit `128`.
- Scenario/impact: user gõ nhầm `--out` có dữ liệu; destructive action xảy ra dù export không thể hoàn tất. Không chạy probe trên user-owned directories.
- Correction/acceptance: reject nonempty/overlapping destination trước mutation; invalid source không được xóa sentinel hoặc source contents.

### T3 — Important — Process-survival assertion có thể pass vacuously

- Location: `tests/cli.rs#L2452-L2496`.
- Evidence: `STATIC`. Không kiểm tra `ps` exit status; empty `seen` pass assertion; filename marker không bảo đảm có trong descendant command lines.
- Scenario/impact: process inspection thất bại nhưng assertion vẫn được coi là safety evidence. Không nói toàn bộ test vacuous: build success vẫn được assert riêng.
- Correction/acceptance: verify inspection mechanism và descendant identity; seeded surviving child phải bị phát hiện trên hai nền tảng CI.

### T4 — Important — Privacy scanner fail-open khi command lỗi

- Location: `tests/scripts/public_export.py#L171-L193`.
- Evidence: `CONTROLLED`, case `export-scanner-error`: injected scanner exit `128`, empty stdout; `gate()` trả `0`.
- Scenario/impact: lỗi scan bị hiểu là zero matches. Các check khác vẫn có thể fail; không khẳng định mọi export luôn pass.
- Correction/acceptance: phân biệt grep `0`/`1` với lỗi; injected error và forbidden-match fixtures đều làm gate fail.

### T5 — Suggestion — Negative fixture mất Python indentation

- Location: `tests/fixtures.rs#L100-L123`; corroboration `tests/cli.rs#L37-L39`.
- Evidence: `STATIC`. Rust escaped-newline continuation bỏ whitespace tiếp theo, làm source khác intended class/function bodies.
- Correction/acceptance: raw string hoặc preserved escapes; assert valid parse, enclosing symbols, constructor positive và receiver negative.

## Behavior-preserving simplification

### R1 — Important — Orchestration vượt repository hard length threshold

- Location: `src/query/impact.rs#L87-L326`; `src/query/analysis.rs#L966-L1152`; `src/query/callgraph.rs#L139-L301`.
- Evidence: `STATIC`; conservative control/declaration starter-line lower bounds là `70`, `57`, `52`. Không phải cognitive/cyclomatic scores.
- Scenario/impact: option semantics, traversal/merge, caps và rendering gắn trong một hàm; thay phase order dễ đổi kết quả và khó review.
- Correction/acceptance: extract contiguous phases, không generic framework; exact output, totals, cap warnings, merge-before-filter và seed exclusions giữ nguyên.

### R2 — Important — Context lặp direct-edge loaders

- Location: `src/query/callgraph.rs#L375-L415` so với `#L931-L950`; `#L541-L565` so với `#L951-L980`.
- Evidence: `STATIC`. Counts, grouping và synthetic unresolved identities phải được sửa đồng bộ ở hai surface.
- Scenario/impact: bug fix một bên có thể làm `callees --depth 1` và `context` mâu thuẫn về cùng graph.
- Correction/acceptance: private side-effect-free loaders; giữ confidence/rendering differences, empty handling, per-section caps và journaling riêng. Không compose public functions vốn capture payload.

### R3 — Suggestion — Unique-target guards lặp ở bảy sites

- Location: `src/query/callgraph.rs#L165-L178`, `#L361-L374`, `#L434-L447`, `#L521-L532`, `#L639-L662`, `#L899-L910`.
- Correction/acceptance: private unique-target wrapper nếu làm code ngắn hơn; giữ endpoint roles, candidate totals và batch-resolve multi-result behavior.

### R4 — Suggestion — Grouped impact không dùng helper hiện có

- Location: `src/query/impact.rs#L136-L152`; `impact_empty_detail` tại `#L63-L73`.
- Correction/acceptance: gọi helper hiện có; giữ exact empty-result wording cho calls/imports/inherits và grouped/site/deeper modes.

### R5 — Suggestion — Path-scope SQL lặp normalization và predicate

- Location: `src/query/scope.rs#L35-L78`; handwritten predicates `src/query/analysis.rs#L65-L73`, `#L288-L305`.
- Correction/acceptance: share trusted-column helper, không SQL-builder framework; giữ bound values, directory boundaries, literal `%`/`_`, parameter positions và root behavior khác nhau.

### R6 — Suggestion — Source-extension classification lặp allowlist

- Location: `src/query/analysis.rs#L643-L648`; `src/walk.rs#L15-L17`; existing reuse `src/insights.rs#L84-L88`.
- Correction/acceptance: reuse `SOURCE_EXTS`; giữ rsplit behavior, case sensitivity, extensionless và unsupported-path classification.

### M1 — Suggestion — Fresh/carried resolver-index registration lặp

- Location: `src/extract/resolve.rs#L68-L128`, `#L189-L251`.
- Correction/acceptance: share registration sau characterization; giữ fresh/carried IDs, parent-kind lookup, insertion order và module-path handling. Không coi refactor này là tự sửa `A2` hoặc `A3`.

## Supplementary historical-harness review

Các finding bổ sung chỉ liên quan scorer/reporting lịch sử, không phải runtime `aw`. Scope và corrected source locations được controller kiểm tra độc lập.

### H1 — Important — Historical search scorer chấp nhận missing/null hops

- Location: `docs/baselines/utility-run-2026-11-02/search_check.py#L4-L7`, `#L35-L49`.
- Evidence: `CONTROLLED`, case `historical-search-hops`: missing, null và explicit zero đều pass cùng hàm thật; chỉ ROOT/EXP được chuyển tới fixture.
- Scenario/impact: answer thiếu required field vẫn được coi correct; không chứng minh run lịch sử đã tận dụng lỗi này.
- Correction/acceptance: require `hops == 0`; missing/null fail, explicit zero pass. Extra route entries không tự động là defect: docstring chỉ yêu cầu expected pointer xuất hiện trong route.

### H2 — Suggestion đề xuất — Pointer shape không chứng minh route adjacency

- Location: `docs/baselines/utility-run-2026-11-02/path_check.py#L4-L9`, `#L63-L82`.
- Disposition: `CONTEST`, không tính là accepted defect. Source docstring đã công bố `route well-formed`, không promise exact-route/adjacency validation.
- Giữ thành evidence limitation: không được dùng kết quả scorer này như proof mọi hop có quan hệ gọi thật. Chưa có report clause cụ thể chứng minh đã overclaim.

### H3 — Important đề xuất — Economics dùng mọi row, gồm failed/invalid cells

- Location: `docs/baselines/utility-run-2026-11-02/summarize_hosts.py#L73-L100`, disclosure `#L101-L108`.
- Disposition: `CONTEST`, không tính là accepted defect. `docs/baselines/hosts-utility-2026-11-02.md#L81-L85` chủ ý giữ over-budget controls; `#L27` định nghĩa mean per cell, không successful-only.
- Giữ thành estimator limitation: cần planned/completed/invalid denominators và tách valid-protocol analysis. Không chứng minh historical delta là sai chỉ từ việc failed runs có mặt.
