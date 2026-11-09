# Finding register: agent-weave re-review

Snapshot: `c8eecb3b36f851c78e8a144c08809cb96de674d3`, ngày 2026-11-04. Register này không thay đổi kết luận lịch sử của snapshot cũ.

Evidence: `EXECUTED` là CLI/MCP thật; `CONTROLLED` là thư viện/hàm thật với dữ liệu tổng hợp; `STATIC` là source trace. Không coi hướng sửa là patch đã kiểm chứng.

Mỗi accepted finding có disposition `PROPOSE FIX`, trừ Suggestions là backlog tùy chọn. `SNEW4` là `ESCALATE` riêng; `RNEW5` trùng finding cũ `R6`, không đếm lần nữa. Các đề xuất `ENEW1`–`ENEW3` đã được reviewer rút lại sau đối chiếu contract.

Bằng chứng thực thi, fixtures và kết quả replay nằm trong [evidence.json](assets/2026-11-04_agent-weave-codebase-review/evidence.json). Một số lỗi chỉ xuất hiện trên điều kiện cục bộ được nêu rõ, không phải remote exploit.

## Critical

### ANEW1 — Gap healing làm mất inbound edges của carried files

- Location: `src/build.rs#L76-L107`, `#L191-L227`; `src/db.rs#L757-L765`.
- Evidence: `EXECUTED`, case `gap-inbound`; peer xác nhận.
- Scenario: `b.invoke` gọi `a.entry`; `a` có unresolved dependency. Thêm module còn thiếu làm `a` được gap-heal sau dirty closure, nhưng `b` vẫn carried. Incremental trả `0 callees`; forced build trả lại `a.entry`.
- Impact: index báo thành công nhưng mất dependency có thật; source cuối cùng giống nhau mà graph khác nhau.
- Correction: áp dụng lại inbound dirty closure cho files được chuyển từ carried sang fresh bởi gap/candidate healing; xử lý tới khi tập ổn định.
- Acceptance: so normalized edge sets của chuỗi ba file giữa incremental và forced build; giữ positive caller và unrelated-file carry control.

### ANEW2 — Xóa candidate không heal ambiguous sites

- Location: `src/build.rs#L81-L107`, `#L170-L200`; `src/db.rs#L469-L485`, `#L532-L569`.
- Evidence: `EXECUTED`, case `removed-candidate`; peer xác nhận.
- Scenario: hai definitions cùng qualname làm call ambiguous. Xóa một definition; null-destination edge không tham gia resolved-edge dirty closure. Incremental vẫn `[ambiguous] foo`; force resolve `pkg.py:1 pkg.foo`.
- Impact: candidate-removal không được phản ánh vào graph; không thể tin rebuild thường tương đương rebuild đầy đủ.
- Correction: candidate-set invalidation phải bao phủ removal/rename, không chỉ addition.
- Acceptance: delete/rename một ambiguous candidate; incremental và fresh có cùng resolution và normalized edges.

### ANEW3 — Symlink ở ancestor của tracked path dẫn đọc ra ngoài root

- Location: `src/walk.rs#L86-L96`, `#L109-L130`; `src/build.rs#L597-L601`.
- Evidence: `EXECUTED`, case `ancestor-symlink`; peer xác nhận với điều kiện dưới đây.
- Scenario: Git index có `pkg/a.py`; working tree thay `pkg` bằng symlink tới external directory. Final component là regular file nên qua kiểm tra; index chứa `pkg.a.external_marker` từ external source.
- Capability: cần working-tree/local namespace modification. Không chứng minh ordinary Git checkout tự materialize được tổ hợp này; không chứng minh network exfiltration.
- Correction: containment phải xét ancestor và file được mở, không chỉ leaf metadata.
- Acceptance: tracked-path fixture có ancestor symlink không được index external content; kiểm tra phương án xử lý concurrent replacement nếu thuộc threat model.

## Important — incremental và resource bounds

### QNEW-A3 — Added-candidate fix chưa làm import graph deterministic

- Location: `src/extract/resolve.rs#L70-L84`, `#L187-L214`; `src/build.rs#L165-L200`.
- Evidence: `EXECUTED`, case `added-candidate-detail`; peer xác nhận.
- Scenario: `pkg.py` và `pkg/__init__.py` cùng khai báo `foo`. Call đã ambiguous trong cả hai build, nhưng incremental import destination là initializer; forced destination là `pkg.py`.
- Impact: old A3 đã sửa call edge, chưa đạt acceptance toàn bộ edge set; import/impact có thể thay đổi theo build mode.
- Correction: duplicate module/symbol candidate handling phải explicit và deterministic, không lấy candidate theo fresh/carried insertion order.
- Acceptance: so normalized import edges và call edges cùng final source; không chỉ assert `[ambiguous]` trong callees.

### ANEW4 — Carried-path query vượt SQLite bind limit

- Location: `src/db.rs#L697-L712`.
- Evidence: `CONTROLLED`, case `carried-sql-limit`: thư viện thật pass với 1.000 virtual paths, lỗi `too many SQL variables` với 32.767. Không tạo repository 32.767 files.
- Scenario/impact: `load_carried` tạo một placeholder cho mỗi path, không chunk; incremental build lớn không prepare được query.
- Correction: chunk query hoặc lọc bằng cơ chế có giới hạn tương đương.
- Acceptance: vượt configured SQLite variable limit vẫn load đúng inherited relations; giữ small-set control và không tạo test corpus lớn không cần thiết.

### ANEW5 — Walk-time size cap không bound read-time allocation

- Location: `src/walk.rs#L123-L136`; `src/build.rs#L55-L56`, `#L597-L601`.
- Evidence: `STATIC`; peer xác nhận; không chạy growing-file hoặc OOM probe.
- Scenario: file nhỏ lúc discovery nhưng lớn lên/thay thế trước `std::fs::read`; toàn bộ bytes vẫn được đọc. Metadata lưu có thể thuộc trạng thái trước read.
- Impact: giới hạn 1 MiB không phải hard allocation bound khi có concurrent changes.
- Correction: bounded read từ opened file; validate hoặc lấy metadata nhất quán với nội dung đã đọc.
- Acceptance: growth/replacement giữa discovery và read phải skip/fail có giới hạn; không allocate theo file không giới hạn hoặc publish metadata sai.

## Important — extraction và resolution

`CNEW1`–`CNEW10` đều có valid parsed fixture và kết quả CLI trong evidence. Không dùng các fixture này để ước lượng tỷ lệ lỗi trên corpus thực tế.

### CNEW1 — Assignment không shadow bare callable name

- Location: `src/extract/walk.rs#L785-L831`; `src/extract/resolve.rs#L683-L722`, `#L1053-L1068`.
- Scenario/actual: `target = None; target()` trong `go` vẫn thành caller của module function `target`. Case `assignment-shadow`.
- Correction/acceptance: assignment bindings phải chặn outer bare-name lookup; tách binding existence khỏi receiver type. Assert unresolved cho shadowed call, resolved cho unshadowed control; thêm tuple/augmented assignments.

### CNEW2 — Scoped import không áp dụng nhất quán cho qualified receiver

- Location: `src/extract/resolve.rs#L705-L709`, `#L1203-L1230`, `#L1333-L1340`.
- Scenario/actual: local `import helpers as h; h.run()` không có callee. Local constructor `C()` có thể resolve, nhưng `value.run()` không resolve. Case `local-qualified-import`.
- Correction/acceptance: module/class/receiver lookup phải dùng lexical scope tương ứng, không chỉ bare calls. Resolve local alias và imported-class method; không leak sang sibling scope; giữ parameter-shadow negative.

### CNEW3 — TS receiver types và ordinary constructor bindings bị mất

- Location: `src/extract/walk.rs#L213-L217`, `#L445-L448`, `#L699-L714`.
- Scenario/actual: `value: C` và `const value = new C()` đều không tạo caller của `C.run`; chỉ constructor edge còn. Case `typescript-receivers`.
- Correction/acceptance: xử lý `type_annotation` wrapper và variable constructor binding. TS/TSX typed/local receiver positives phải resolve; factory và unknown reassignment phải không tạo certainty sai.

### CNEW4 — Named default export không bind default import

- Location: `src/extract/walk.rs#L1666-L1675`, `#L1700-L1709`; `src/extract/resolve.rs#L1104-L1121`.
- Scenario/actual: `export default function work()` và `import execute` không tạo `go -> worker.work`. Case `typescript-default`.
- Correction/acceptance: map default export tới identity thật; unrelated local import name vẫn resolve. Giữ named imports và nonexistent-default negative control.

### CNEW5 — Constructor guard loại struct literal và `Self::new()`

- Location: `src/extract/walk.rs#L885-L920`.
- Scenario/actual: `A { value: 1 }` và `Self::new()` không type receiver; với hai `render` methods để chặn fallback, không có caller của `A.render`. Case `rust-constructors`.
- Correction/acceptance: struct expression tách khỏi associated-constructor guard; normalize `Self` theo impl trước khi bỏ method segment. Giữ arbitrary factory returning-other-type negative.

### CNEW6 — Later assignment thay đổi attribution của earlier call

- Location: `src/extract/resolve.rs#L154-L157`, `#L1156-L1169`.
- Scenario/actual: call sau `A()` và call sau `B()` đều được gán tới `B.run`; sites 9 và 11 cùng trỏ B. Case `ordered-bindings`.
- Correction/acceptance: ordered binding events và latest preceding binding theo call site. Assert destination theo `site_line`; later unknown assignment không được xóa earlier known call.

### CNEW7 — Annotation override explicit reassignment

- Location: `src/extract/resolve.rs#L1154-L1169`.
- Scenario/actual: `go(value: A)` rồi `value = B(); value.run()` vẫn có `A.run`, không có `B.run`. Case `annotation-reassign`.
- Reconciliation: peer có một nhận xét `CONTEST` nhưng đồng thời xác nhận annotation-first mechanism. Controller contest nhận xét đó bằng exact output `sample.py:2 sample.A.run` và source branch; giữ severity Important, không coi phản biện mâu thuẫn là unanimous approval.
- Correction/acceptance: annotation là initial binding, assignment sau đó có precedence theo vị trí. Untouched A, reassigned B và reassigned unknown phải phân biệt đúng.

### CNEW8 — Arrow parameter không shadow global function

- Location: `src/extract/walk.rs#L1599-L1607`, `#L1657-L1659`; `src/extract/resolve.rs#L711-L722`.
- Scenario/actual: `(target) => target()` trở thành caller của global `target`. Case `arrow-parameter`.
- Correction/acceptance: collect arrow parameters và lexical boundaries của nested callbacks. Giữ genuine unshadowed arrow calls; test single-parameter form không parentheses.

### CNEW9 — Receiver spelling thắng nested parameter identity

- Location: `src/extract/walk.rs#L669-L675`; `src/extract/resolve.rs#L768-L786`.
- Scenario/actual: `inner(self)` trong `C.outer` tạo edge tới `C.run` dù `self` là parameter mới chưa biết type. Case `nested-self-shadow`.
- Correction/acceptance: implicit receiver gắn với owning method binding, không chỉ tên `self`. Giữ ordinary method và captured closure positives; nested shadow phải unresolved.

### CNEW10 — Rust inline-module aliases leak trong layout `src/lib.rs`

- Location: `src/extract/walk.rs#L1963-L2008`; `src/extract/resolve.rs#L723-L749`.
- Scenario/actual: left module import `a::target`, right import `b::target`; cả hai `go` gọi `src.b.target`. Case `rust-inline-import-src`.
- Reconciliation: root `lib.rs` variation có cả hai calls unresolved, nên rejection của peer đúng cho variation đó. Layout `src/lib.rs` đã tái hiện leakage; không đồng nhất hai outcomes hoặc che negative evidence.
- Correction/acceptance: use bindings giữ inline module/function scope; cả hai destination đúng và không cross-edge. Characterize cả root và `src/` layouts, cùng bare inline-module calls.

## Important — filesystem, journals và scripts

### SNEW1 — Hardlinked lock làm thay đổi external sentinel

- Location: `src/db.rs#L148-L178`, `#L202-L205`; journal append `src/journal.rs#L203-L222`.
- Evidence: `EXECUTED`, case `hardlink-lock`; build exit 0 và sentinel thay đổi. Journal-hardlink variant chỉ static.
- Capability/impact: cần local hardlink setup và write permission; Git checkout không tạo hardlink. Đây là externally named inode corruption, không chứng minh privilege escalation.
- Correction/acceptance: validate opened descriptor/file type/link count theo platform trước truncate/append; lock và log hardlink sentinels không được đổi bytes.

### SNEW2 — Journal FIFO làm command không kết thúc

- Location: `src/journal.rs#L195-L222`; MCP finalization `src/mcp.rs#L239-L248`.
- Evidence: `EXECUTED`, case `fifo-writer`; query đã in output nhưng chưa thoát sau timeout 1 s, child bị kill và reap.
- Scenario/impact: readerless FIFO pass symlink guard rồi blocking append; best-effort journal ảnh hưởng availability. Cần local filesystem setup; không phải Git-only payload.
- Correction/acceptance: reject non-regular targets trước blocking open và handle replacement policy. CLI phải terminate; MCP phải trả response và xử lý ping tiếp theo.

### SNEW3 — Journal readers không bound special files hoặc bytes

- Location: `src/journal.rs#L324-L337`, `#L459-L496`; `src/doctor.rs#L83-L89`.
- Evidence: FIFO doctor read `EXECUTED`, case `fifo-reader`, timeout 1 s rồi kill/reap. Infinite-device/OOM path chỉ `STATIC`; không chạy `/dev/zero`.
- Scenario/impact: unrestricted `read_to_string` nhận FIFO, device-backed symlink hoặc oversized regular file; verification sample limit không bound việc load trước đó.
- Correction/acceptance: regular-file checks và bounded byte/record reads, kể cả rotated generations; tránh materialize toàn bộ history để sample. Device/FIFO/oversized inputs phải terminate predictably.

### TNEW1 — Export scrub follow tracked symlink ra ngoài destination

- Location: `tests/scripts/public_export.py#L99-L109`, `#L116-L131`.
- Evidence: `EXECUTED`, case `export-symlink`; external sentinel hash thay đổi sau archive và scrub.
- Scenario/impact: `.rs` hoặc `.md` tracked symlink được archive giữ nguyên; `is_file`, read và write follow target. Nonempty-destination fix không chặn đường này.
- Correction/acceptance: reject hoặc safely process symlink entries trước cả hai scrub loops. Tracked symlink fixture phải fail safely, external sentinel giữ nguyên.

### TNEW2 — Allowlist checker PASS khi Python input read lỗi

- Location: `tests/scripts/allowlist_check.sh#L7-L23`.
- Evidence: `EXECUTED`, case `allowlist-missing-input`: `FileNotFoundError` nhưng exit 0 và PASS.
- Scenario/impact: `set +e` bỏ qua Python failure, empty stdout bị hiểu là không có package lạ; gate không fail-closed.
- Correction/acceptance: propagate Python status trước khi xét stdout; missing/unreadable inputs và unlisted packages đều fail, valid listed input pass.

### TNEW3 — Network negative control thiếu positive reachability control

- Location: `.github/workflows/ci.yml#L80-L90`; `tests/scripts/seatbelt-build.sh#L18-L29`.
- Evidence: `STATIC`; peer xác nhận; không chạy network-escape probe.
- Scenario/impact: endpoint không reachable ngay ngoài sandbox hoặc client lỗi vẫn được coi là isolation thành công. Không chứng minh sandbox thực sự cho egress.
- Correction/acceptance: positive control với controlled reachable endpoint; phân biệt setup/probe error với denial. Tắt isolation phải làm negative control fail.

## Important — query contracts và accounting

### DNEW1 — Audit action không nhận required tool input

- Location: `.github/workflows/ci.yml#L55-L58`.
- Evidence: `STATIC` cộng pinned external metadata: [action.yml](https://raw.githubusercontent.com/taiki-e/install-action/1919c4a95903d2c33e4a221102edca87309fc14b/action.yml) khai báo `inputs.tool.required: true`, không có default. Excerpt được giữ trong evidence.
- Scenario/impact: clean audit runner không được chỉ định cài `cargo-audit` trước `cargo audit`. Chưa chạy workflow; không gọi đây là CI failure đã tái hiện.
- Correction/acceptance: truyền `tool: cargo-audit`; clean job phải tới được audit operation. Peer ban đầu `ESCALATE` vì chưa nhận metadata; final peer đã đọc excerpt và xác nhận static evidence, không phải reproduced CI failure.

### DNEW2 — Transitive callers mô tả thiếu shortest-symbol/seed exclusions

- Location: `src/query/callgraph.rs#L440-L449`, `#L475-L505`; `docs/cli-reference.md#L95-L101`, `docs/usage.md#L43`.
- Evidence: `EXECUTED`, case `callers-closure`: self-call có ở depth 1 biến mất ở depth 2; edge trên longer route từ symbol đã visited cũng không hiện.
- Impact: bounded reverse closure theo symbol không bằng mọi call site trên bounded routes, dù result không bị cap.
- Correction/acceptance: owner chốt semantics rồi đồng bộ docs/skill/examples. Test triangle, self-recursion và multi-route; không tự đổi traversal theo giả định của reviewer.

### RNEW1 — Valid transitive caller bị payload verifier đánh FAIL

- Location: `src/query/callgraph.rs#L397-L405`, `#L497`; `src/journal.rs#L553-L563`.
- Evidence: `EXECUTED`, case `transitive-verify`: `ok=1 changed=0 fail=1`; depth-two origin gọi middle, không nhắc leaf.
- Impact: verification báo false defect trên source/index không đổi. Exit code của command vẫn 0; finding là false FAIL, không phải exit handling.
- Correction/acceptance: phân biệt direct/transitive payload verification; giữ bounds/epoch/source checks và applicable direct-call checks. CLI/MCP chain phải verify không false FAIL.

### RNEW2 — Capped exact/FTS overlap làm total sai

- Location: `src/query/analysis.rs#L884-L936`.
- Evidence: `EXECUTED`, case `search-overcount`: ba `run` definitions nhưng `--limit 1` trả `showing 1 of 5`.
- Impact: full counts trừ capped-page overlap không phải cardinality của union; footer và journal totals sai.
- Correction/acceptance: đếm complete union/intersection theo symbol identity mà vẫn bound materialization. Cover limits 1/3, exact-only và prefix-only additions.

### RNEW3 — Impact explanation thay đổi theo row IDs

- Location: `src/query/impact.rs#L472-L477`, `#L527-L530`, `#L581-L596`.
- Evidence: `CONTROLLED`, case `impact-rowid-order`: cùng normalized edges, đổi module/function IDs làm explanation chuyển từ `[imports depth=1]` sang `[calls depth=1]`.
- Impact: giữ first record khi depth/line tie không đảm bảo semantic determinism; final path sort không sửa được representative đã chọn.
- Correction/acceptance: tie-break bằng stable semantic fields; normalized graphs khác row IDs/insertion order phải có cùng context/explanation.

### RNEW4 — Search vẫn gộp nhiều policy trong một function lớn

- Location: `src/query/analysis.rs#L772-L951`.
- Evidence: `STATIC`; 157 nonblank/non-comment physical lines, không phải cognitive complexity hoặc effective LOC. Peer xác nhận coupling và duplication của Pointer mapping.
- Impact: token/phrase/morphology/ranking/exact merge/count/render policy gắn nhau; overlap bug dễ bị che trong orchestration.
- Correction/acceptance: characterize rồi tách cohesive merge/count và mapper; sửa RNEW2 riêng khỏi mechanical refactor. Giữ exact-first order, fallback, cap, stale và payload bytes.

## Suggestions và threat-model decision

### DNEW3 — Trust wording mâu thuẫn recovery/verification rules

- Location: `skills/aw-first/SKILL.md#L31-L51`, `#L79-L82`, `#L102-L108`.
- Evidence: `STATIC`, peer xác nhận. Absolute “Do NOT re-verify” không chứa đầy đủ exceptions cho caps, stale và safety/review.
- Correction/acceptance: một policy nhất quán tránh duplicate navigation nhưng cho phép required verification và recovery; không biến static index thành authority về behavior.

### SNEW5 — JSON-RPC notifications/envelopes xử lý chưa đúng

- Location: `src/mcp.rs#L162-L188`, `#L226-L280`.
- Evidence: `EXECUTED` benign MCP frames: id-less ping nhận `id:null` response; invalid method type bị bỏ qua. Không chứng minh supported client bị lỗi thực tế.
- Correction/acceptance: validate envelope, suppress notification responses và trả proper invalid-request errors; exact frame tests phải giữ subsequent ping sequencing.

### RNEW6 — Payload metadata fallback lặp ở nhiều renderers

- Location: `src/query.rs#L347-L360`; `src/query/analysis.rs#L232-L244`, `#L615-L627`, `#L751-L763`; `src/query/callgraph.rs#L922-L926`, `#L1164-L1168`.
- Correction/acceptance: helper nhỏ ở query layer nếu giảm code; giữ empty-payload behavior, name, counts, stale flags, fallback và số lần recording. Không đưa journaling vào SQL loaders.

### SNEW4 — ESCALATE, Suggestion — Active namespace replacement threat model

- Location: `src/db.rs#L49-L78`, `#L147-L178`; `src/journal.rs#L195-L222`.
- Evidence: `STATIC` race window giữa path checks và opens; chưa chạy synchronized replacement probe.
- Owner decision: có yêu cầu chống actively hostile local actor rename `.aw` khi process đang chạy hay không? Không đồng nhất capability này với pre-positioned Git symlink.
- Owner decision (2026-11-04): **OUT OF SCOPE**. Keep the documented limitation in `SECURITY.md`; do not add descriptor-relative operations, new dependencies, or unsafe code for this threat model.
- Nếu thuộc scope: descriptor-relative/identity-safe operations và synchronized sentinel regression. Nếu ngoài scope: public safety wording phải nêu giới hạn; thêm pathname rechecks không chứng minh race immunity.

## Implementation status (working tree, 2026-11-04)

All accepted findings have working-tree fixes with targeted regression tests. This is implementation evidence, not an independent re-review.

| Finding | Fix | Test |
|---|---|---|
| ANEW1 | Fixed-point inbound closure after gap healing (`src/build.rs`) | `gap_healing_reapplies_inbound_closure` |
| ANEW2 | Candidate shrink/rename invalidation; `qualnames_for_files` (`src/build.rs`, `src/db.rs`) | `removed_duplicate_candidate_invalidates_ambiguous_sites` |
| ANEW3 | Canonical ancestor containment before metadata/read (`src/walk.rs`) | `walk_git_visible_skips_ancestor_symlink` |
| QNEW-A3 | Duplicate import candidates are `AMBIGUOUS`, not insertion-order picks (`src/extract/resolve.rs`) | `added_duplicate_candidate_invalidates_resolved_edges` (edge-set equality) |
| ANEW4 | Chunked `load_carried` inherits query (`src/db.rs`) | `load_carried_chunks_large_path_sets` |
| ANEW5 | Bounded read from the opened descriptor (`src/walk.rs::read_bounded`) | `read_bounded_rejects_over_cap` |
| CNEW1 | Assignments shadow bare callable names from their site line onward; tuple/augmented bindings included | `assignment_shadows_bare_callable_name`, `later_assignment_does_not_shadow_earlier_call`, `tuple_and_augmented_assignments_shadow_bare_names` |
| CNEW2 | Scope-aware alias lookup for qualified receivers and classes | `local_qualified_import_and_class_receiver` |
| CNEW3 | TS `type_annotation` and `new C()` receiver bindings; later `new` reassignment updates the type | `typescript_receiver_annotations_and_construction`, `reassignment_updates_receiver_type_ts_and_rust` |
| CNEW4 | Default export bound to its real identity | `typescript_default_export_binds_default_import` |
| CNEW5 | Rust struct literal and `Self::new()` receiver typing | `rust_struct_literal_and_self_new_type_receiver` |
| CNEW6 | Ordered binding events at each call site; TS/Rust reassignment updates the binding | `ordered_bindings_resolve_at_call_site`, `reassignment_updates_receiver_type_ts_and_rust`, `later_assignment_does_not_shadow_earlier_call` |
| CNEW7 | Annotation is an initial binding, not an override | `annotation_is_initial_binding_not_override` |
| CNEW8 | Arrow parameter collection with nested-callback line ranges (including nested named functions) | `arrow_parameter_shadows_global_function`, `nested_arrow_parameter_shadows_only_inside_callback`, `nested_function_inside_arrow_uses_arrow_binding` |
| CNEW9 | Explicit nested `self`/`cls`/`this` shadows the receiver | `nested_self_parameter_shadows_receiver` |
| CNEW10 | Rust use aliases scoped by module; root inline-module naming | `rust_inline_module_aliases_do_not_leak` |
| SNEW1 | Hardlink/non-regular checks for lock and journal appends | `hardlinked_build_lock_does_not_touch_external_file`, `hardlinked_journal_does_not_touch_external_file` |
| SNEW2 | FIFO/non-regular journal append is skipped before open | `fifo_journal_does_not_hang_build` |
| SNEW3 | Regular-file, bounded journal reads (`read_log_bounded`) | `fifo_journal_reader_does_not_hang` |
| TNEW1 | Export scrub rejects symlink entries | `test_scrub_rejects_symlink_entries` |
| TNEW2 | Allowlist checker propagates input read failures | `test_allowlist_checker_fails_closed` |
| TNEW3 | Controlled local positive control on macOS and Linux | `seatbelt-build.sh`, `linux-netns-build.sh` |
| DNEW1 | Audit job installs `cargo-audit` via `tool:` | workflow diff |
| DNEW2 | Shortest-depth and seed-exclusion semantics documented | `transitive_callers_use_shortest_depth_per_symbol` |
| RNEW1 | Transitive payload verification skips only depth ≥ 2 name checks; depth 1 still checks, and aliased call sites are accepted via the index edge | `usage_verify_transitive_callers_has_no_false_fail`, `usage_verify_depth_one_still_checks_name` |
| RNEW2 | Exact ∪ FTS cardinality by symbol identity | `search_limit_counts_union_by_identity` |
| RNEW3 | Stable semantic tie-break for impact rows | `impact_top_ties_are_deterministic` |
| RNEW4 | Search split into expression/page/merge helpers; exact-first order, union count, and payload shown/total characterized | `search_exact_first_merge_and_union_count_characterization` |
| RNEW6 | Shared `record_payload_meta` helper | `search_exact_first_merge_and_union_count_characterization`, `usage_verify_transitive_callers_has_no_false_fail`, full suite |
| DNEW3 | Trust Contract decision table with recovery/verification exceptions | `SKILL.md`, `agent-integration.md` |
| SNEW5 | Envelope validation, notification suppression, invalid-request errors | `notification_ping_has_no_response`, `invalid_request_envelopes_return_minus_32600`, `client_response_envelope_is_ignored` |
| SNEW4 | Owner confirmed out-of-scope (2026-11-04); limitation documented | `SECURITY.md` |

## Peer review (2026-11-04)

Two independent review cycles covered the working-tree diff against this register. The second cycle reported one blocking issue: `usage --verify` false-FAILed a valid depth-1 caller reached through an import alias. The fix resolves the site's index edge before applying the name heuristic and adds the aliased fixture. Re-verified: `usage_verify_transitive_callers_has_no_false_fail` passes, and a manual alias probe reports `ok=2 changed=0 fail=0`. The second cycle re-checked call-site binding order, TS/Rust reassignment, tuple/augmented assignments, nested arrow scopes, the depth-1 verifier check, TNEW1/TNEW2 tests, the Linux netns positive control, and the HTML model labels as resolved. Disclosed shape-by-shape residuals remain in the codebase review; they are not register blockers.
