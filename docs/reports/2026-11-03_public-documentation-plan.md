# Kế hoạch hoàn thiện tài liệu public cho aw

## Tóm tắt

Chuyển tài liệu từ lịch sử triển khai sang hướng dẫn theo nhu cầu người dùng, có bằng chứng và giới hạn rõ ràng.
Kiểm kê nguồn gồm 127 tài liệu Markdown; phạm vi cập nhật chỉ gồm tài liệu dự kiến public.
Nghiệm thu trên tập tài liệu đã chọn và bản phân phối public, không phải toàn bộ tài liệu nội bộ.

**Phạm vi đã chốt:** `docs/reports/**` không public, không thuộc backlog biên tập public.
Kế hoạch này nằm trong thư mục nội bộ đó. Không xuất bản hoặc liên kết đến nó từ tài liệu public.

Trạng thái cập nhật 2026-11-03: chủ dự án đã yêu cầu triển khai và commit theo Conventional Commits.
Đã hoàn thành biên tập và Goldfish review từng tài liệu public; xem `2026-11-03_public-documentation-review.md` cùng thư mục.
Chưa kiểm chứng cài đặt trên máy sạch, host integration hoặc quyền publish. Không sửa dữ liệu benchmark hoặc publish.
Ngôn ngữ tài liệu sản phẩm giữ tiếng Anh theo repository; kế hoạch và trao đổi dùng tiếng Việt.

## 1. Phạm vi và mốc đối chiếu

- Revision: `c3f907c973b2288f8ee2a37466a244406f2ffe8a`, ngày 2026-11-03.
- Working tree trước khi viết kế hoạch: `AGENTS.md` và `docs/reports/2026-11-03_aw-build-force-handoff.md` chưa tracked.
- Không coi handoff chưa tracked là tài liệu đã được duyệt hoặc được phép xuất bản.
- Kiểm kê: 127 tài liệu Markdown, 12.483 dòng; không tính chính kế hoạch này.
- Có thêm 235 file `.txt` thuộc prompt, transcript và artifact benchmark; đây không phải 235 trang hướng dẫn người dùng.
- Các con số trên là kết quả kiểm kê, không phải điểm chất lượng hoặc số trang cần viết lại.
- Phát hiện bên dưới đến từ đọc tĩnh. Chưa chạy lại benchmark, cài đặt từ checkout sạch hoặc kiểm chứng bản export.

Lệnh tái lập kiểm kê tại snapshot trước khi thêm kế hoạch:

```sh
git rev-parse HEAD
git status --short
python3 - <<'PY'
from pathlib import Path
import subprocess
paths = subprocess.check_output(
    ['git', 'ls-files', '--cached', '--others', '--exclude-standard'], text=True
).splitlines()
documents = [Path(p) for p in paths if Path(p).suffix.lower()
             in {'.md', '.rst', '.adoc', '.html'}
             and Path(p).name not in {'CLAUDE.md', 'GEMINI.md'}]
print('documents:', len(documents))
print('lines:', sum(len(p.read_text().splitlines()) for p in documents))
print('txt artifacts:', sum(Path(p).suffix == '.txt' for p in paths))
PY
```

| Nhóm hiện có | Số tài liệu | Cách xử lý |
|---|---:|---|
| Root, gồm AGENTS | 8 | Rà toàn bộ; ưu tiên README, changelog, đóng góp và hỗ trợ |
| GitHub templates | 3 | Giảm yêu cầu giải mã quy trình nội bộ |
| Skills | 2 | Làm rõ tích hợp, cách chọn lệnh và giới hạn |
| Thiết kế | 5 | Phân biệt hợp đồng hiện hành và đề xuất lịch sử |
| Baseline ngoài review | 30 | Lập bản đồ bằng chứng; tóm tắt kết quả theo tác vụ |
| Review và tài liệu con | 77 | Bảo toàn lịch sử; xác định trạng thái và khả năng công khai |
| Reports/handoff | 2 | Không public; chỉ tham khảo nội bộ, không viết lại trong đợt này |

Rà thêm các bề mặt liên quan: help CLI, mô tả MCP, metadata package, LICENSE, NOTICE, CI và release workflow.
Không dịch hàng loạt, tạo website tài liệu, thêm framework hoặc thay đổi hành vi sản phẩm trong đợt biên tập này.

### Ranh giới tài liệu public

- Phạm vi ứng viên: tài liệu root, GitHub templates, skills, các trang hướng dẫn mới và thiết kế được chọn để xuất bản.
- Giữ `docs/baselines/**` nội bộ theo luồng export hiện có; không biên tập lại toàn bộ baseline/review cho public.
- Loại toàn bộ `docs/reports/**` khỏi bản public, kể cả report, handoff và kế hoạch này.
- Chỉ rà nguồn nội bộ khi cần kiểm chứng claim hoặc xác định trạng thái chức năng trong tài liệu public.
- Lập manifest đường dẫn public trước triển khai. Số 127 là tổng nguồn kiểm kê, không phải số tài liệu sẽ public.
- Không mở rộng sang công khai lịch sử Git hoặc artifact nội bộ trong kế hoạch này.

## 2. Những vấn đề phải giải quyết

| Phát hiện tĩnh | Nguồn | Hướng xử lý |
|---|---|---|
| Cơ chế và benchmark dày đặc xuất hiện trước hành trình sử dụng | `README.md:5–29` | Mở bằng tác vụ, kết quả và ví dụ ngắn |
| P4/P8 và mã review đứng ở vị trí tên chức năng | `CHANGELOG.md:40–97`, `docs/designs/` | Tên dễ hiểu trước; mã tham chiếu sau |
| F1 không có cùng nghĩa giữa các amendment | Tài liệu thiết kế P4 và P8 | Không thay thế toàn cục theo mã; luôn xác định ngữ cảnh |
| Đề xuất P8 vẫn ghi chờ duyệt, nhưng spec chính đã ghi triển khai | Preamble thiết kế P8; Amendment 19 trong spec chính | Gắn trạng thái lịch sử và liên kết quyết định hiện hành |
| Nhiều phép đo so sánh hai cách dùng aw, không phải aw với không dùng aw | `docs/baselines/agent-utility-2026-11-02.md:15–17`; `stage5-context-2026-11-02.md:7–8` | Ghi rõ công cụ được phép ở từng nhóm đối chứng |
| Evaluation trộn số đo từ nhiều revision dưới lời giới thiệu “binary being shipped” | `docs/EVALUATION.md:3–7`, các mục lịch sử và schema v5/v7 | Gắn revision/ngày cho từng nhóm bằng chứng; bỏ lời khẳng định bao trùm |
| Hướng dẫn hỗ trợ yêu cầu nguồn không truy cập trực tiếp | `SUPPORT.md:9` | Đưa giới hạn cần biết vào trang public có link thật |
| Export xóa baseline và đổi tham chiếu thành `evidence/` | `tests/scripts/public_export.py:89,100–107` | Kiểm tra cây export; không coi đổi chuỗi là đã cung cấp bằng chứng public |
| Báo cáo build mới có kết quả khác headline hiệu năng cũ | `docs/reports/2026-11-03_aw-build-force-optimization.md:20–43` | Ghi rõ so sánh xen kẽ, host đang tải cao và đánh đổi bộ nhớ |
| Help chứa thuật ngữ triển khai và mã amendment | `src/cli.rs` | Rà riêng; thay help phải tuân thủ hợp đồng stdout/stderr |

## 3. Chuẩn nội dung public

### Người đọc và câu hỏi phải trả lời

| Người đọc | Câu hỏi | Trang dẫn đường |
|---|---|---|
| Người đánh giá công cụ | aw giúp gì, khác grep/LSP ở đâu, khi nào không cần aw? | README |
| Người dùng lần đầu | Cài thế nào, chạy gì đầu tiên, đầu ra có ý nghĩa gì? | Getting started |
| Người dùng thường xuyên | Chọn lệnh nào, diễn giải kết quả rỗng/cũ/bị giới hạn thế nào? | Usage và CLI reference |
| Người tích hợp agent | Cài skill hoặc MCP thế nào, thay đổi quy trình gì? | Agent integration |
| Người kiểm chứng | Số liệu đo thế nào, có tái lập được không? | Evaluation |
| Người đóng góp | Code ở đâu, hợp đồng nào phải giữ, kiểm tra và gửi PR thế nào? | Contributing và spec |

### Mẫu giải thích tính năng

1. Câu hỏi thực tế người dùng muốn giải quyết.
2. Cách giải quyết khi không có tính năng đó; nêu công cụ đối chứng thực tế.
3. Lệnh aw và ví dụ đầu ra ngắn, có thể đối chiếu.
4. Người dùng có thể quyết định hoặc làm gì tiếp từ kết quả.
5. Giới hạn: kết quả thiếu, tên mơ hồ, index cũ, quan hệ động hoặc loại file chưa hỗ trợ.
6. Bằng chứng định lượng nếu có; nếu chưa đo, chỉ mô tả chức năng.

Ví dụ: “Tìm các file phụ thuộc vào file sắp sửa để chọn nơi cần kiểm tra tiếp.”
Không viết: “Biết đầy đủ mọi nơi sẽ hỏng khi sửa file.”

### Quy tắc từ ngữ và bằng chứng

- Dùng tên chức năng trước mã P/F/G/R; giải thích thuật ngữ ở lần đầu xuất hiện.
- Giữ nguyên tên lệnh, flag, output literal và ID bằng chứng.
- Tách “có sẵn”, “được khuyến nghị”, “thử nghiệm”, “chưa có bằng chứng tiết kiệm”.
- Không diễn giải KEEP là mọi người nên dùng; không diễn giải NO VALUE là vô dụng trong mọi tình huống.
- Mỗi tuyên bố số liệu phải có nguồn, revision, tác vụ/corpus, host/model, cỡ mẫu, metric và ngoại lệ liên quan.
- Phân biệt recall trung bình theo truy vấn với tỷ lệ tổng số symbol tìm được; không trình bày như cùng mẫu số.
- Phân biệt kết quả đúng trên bộ đáp án với độ bao phủ toàn repository.
- Không đổi giảm tool calls thành giảm tokens, thời gian, tiền hoặc lỗi production nếu chưa đo.
- Không gọi kết quả vài host là “host-independent”. Ghi rõ những host/model đã đo.
- Nếu dữ liệu gốc không công khai, ghi “được báo cáo từ phép đo nội bộ; không thể tái lập đầy đủ từ bản public”.
- Bổ sung phép đo tái lập bằng self-repository hoặc fixture public nếu cần bằng chứng public mạnh hơn; không giả lập lại số cũ.

## 4. Cấu trúc tài liệu đề xuất

Giữ Markdown trong repository, không cần thêm hệ thống build tài liệu.
Các trang mới dưới đây là đề xuất để chủ dự án duyệt trước khi triển khai.

| Đường dẫn | Trách nhiệm duy nhất |
|---|---|
| `README.md` | Giá trị, phạm vi, quickstart ngắn, bằng chứng chính và bản đồ tài liệu |
| `docs/README.md` | Chỉ mục theo nhu cầu; đường vào tài liệu người dùng, kỹ thuật và bằng chứng |
| `docs/getting-started.md` | Cài từ clone, điều kiện môi trường, build index, truy vấn đầu tiên, rebuild và gỡ dữ liệu index |
| `docs/usage.md` | Quy trình sửa code, theo luồng gọi, đọc cấu trúc, điều tra ứng viên test/duplicate |
| `docs/cli-reference.md` | Lệnh/flag/default, ví dụ, đầu ra, exit code, giới hạn và journal |
| `docs/agent-integration.md` | Skill và MCP: cách thiết lập, kiểm tra hoạt động, fallback và gỡ tích hợp |
| `docs/EVALUATION.md` | Kết quả, đối chứng, phạm vi áp dụng, tái lập và bằng chứng âm |
| `SUPPORT.md` | Triệu chứng → nguyên nhân → bước xử lý; nơi báo lỗi và phạm vi hỗ trợ |
| `CONTRIBUTING.md` | Thiết lập dev, bản đồ code, hợp đồng, kiểm chứng và quy trình PR |
| `CHANGELOG.md` | Thay đổi theo phiên bản, tác động và hành động người dùng |
| `SECURITY.md`, `CODE_OF_CONDUCT.md`, `LICENSE*`, `NOTICE` | Chính sách và giấy phép; giữ đúng nội dung có thẩm quyền |
| `docs/designs/` | Hợp đồng kỹ thuật, amendment và lịch sử quyết định có trạng thái rõ |
| `docs/baselines/`, `docs/reports/` | Nguồn nội bộ, không nằm trong tập tài liệu public |
| `skills/README.md`, `skills/aw-first/SKILL.md`, `AGENTS.md` | Hướng dẫn agent; dẫn tới tài liệu chính thay vì nhân bản số liệu |

Không bắt người đọc đi qua glossary để hiểu quickstart. Giải nghĩa tại chỗ trước khi cân nhắc tạo glossary riêng.
Khi tách nội dung khỏi README, giữ một nguồn chính cho mỗi thông tin; phần còn lại dùng liên kết.

## 5. Các gói thực hiện

Mỗi gói là một thay đổi logic có thể review độc lập. Tách diff lớn thành các phần khoảng 100–300 dòng khi khả thi.
Chỉ commit sau khi được yêu cầu; ranh giới gói dưới đây cũng là ranh giới commit dự kiến.

| Gói | File và công việc | Phụ thuộc | Bằng chứng nghiệm thu | Reviewer |
|---|---|---|---|---|
| Kiểm kê và xuất bản | Lập manifest tài liệu public hiện hành/lịch sử; loại `docs/baselines/**` và `docs/reports/**` | Phạm vi đã chốt ở mục 1 | Mỗi ứng viên có disposition; không tự xóa hoặc công khai artifact nội bộ | Maintainer phụ trách xuất bản |
| Thông điệp và nhập môn | `README.md`, `docs/README.md`, `docs/getting-started.md` | Cấu trúc được duyệt | Người mới đi từ clone đến kết quả thật; ví dụ không cần repo riêng | Technical writer và người dùng mới |
| Hướng dẫn và tham chiếu | `docs/usage.md`, `docs/cli-reference.md`, `SUPPORT.md` | Nhập môn | Đủ lệnh hiện có; defaults/flags/exit codes khớp CLI; có fallback | Reviewer hiểu CLI và static analysis |
| Bằng chứng giá trị | `docs/EVALUATION.md` và phần claim của README/skills | Phân loại public | Mỗi headline truy được đối chứng, số đo và giới hạn; không link tới file bị export loại bỏ | Reviewer benchmark độc lập |
| Tích hợp agent | `docs/agent-integration.md`, `skills/README.md`, `skills/aw-first/SKILL.md` | Hướng dẫn và bằng chứng | Từng host ghi phiên bản và trạng thái kiểm chứng; không thay routing chỉ để làm đẹp nội dung | Reviewer CLI/MCP và agent integration |
| Đóng góp và chính sách | `CONTRIBUTING.md`, `.github/*TEMPLATE*`, `SECURITY.md`, `CODE_OF_CONDUCT.md`, `NOTICE`, `LICENSE*`, `AGENTS.md` | Phạm vi public | Link công khai tồn tại; lệnh dev khớp CI; liên hệ và giấy phép được maintainer xác nhận | Maintainer |
| Lịch sử và thiết kế | `CHANGELOG.md` và các thiết kế trong manifest public; baseline/review/report chỉ để đối chiếu | Bản đồ trạng thái | Proposal public cũ dẫn tới quyết định hiện hành; không phụ thuộc link nội bộ | Reviewer hợp đồng độc lập |
| Help và export, nếu cần sửa | `src/cli.rs`, `src/mcp.rs`, `tests/scripts/public_export.py` và test liên quan | Phát hiện cụ thể từ các gói trên | Patch/test riêng; contract-bump nếu đổi output; export không bỏ sót trang mới hoặc phá link | Reviewer Rust/contract và export |
| Nghiệm thu public | Toàn bộ tập tài liệu đã chọn | Các gói được duyệt | Checkout/export sạch đọc được, hướng dẫn chạy được, không có blocker chưa xử lý | Người chưa tham gia viết + maintainer |

Các reviewer đánh giá từ diff, hợp đồng và bằng chứng; không chỉ dựa vào giải thích của tác giả.
Giới hạn fix/review theo `CONTRIBUTING.md`: tối đa hai chu kỳ; sau đó chuyển vấn đề chưa giải quyết cho chủ dự án.
Không cần sửa help hoặc export nếu biên tập Markdown đã đáp ứng yêu cầu và bản public kiểm tra đạt.

## 6. Thứ tự triển khai và điểm dừng

1. Chốt manifest theo phạm vi đã duyệt, cấu trúc tài liệu và trang đích cho kết luận dựa trên bằng chứng nội bộ.
2. Chốt bảng claim: giữ, diễn đạt hẹp hơn, đánh dấu lịch sử, hoặc cần đo thêm.
3. Hoàn thành README và getting started trước; review hành trình người mới.
4. Hoàn thành usage, reference, support và agent integration; kiểm tra tính nhất quán.
5. Hoàn thành changelog, contributing, thiết kế public và phần giải thích nguồn bằng chứng trong evaluation.
6. Xử lý riêng các thay đổi executable/help/export nếu thực sự cần.
7. Nghiệm thu bản public tại revision đã chốt; bàn giao danh sách kiểm tra đạt/chưa đạt.

Đợt này hoàn tất khi tài liệu đạt tiêu chí đã duyệt. Không tự mở rộng sang benchmark mới, thiết kế lại tính năng hoặc phát hành release.

## 7. Kiểm chứng và tiêu chí chấp nhận

### Kiểm tra tài liệu

- Inventory phải bao phủ mọi file thuộc phạm vi, kể cả file giữ nguyên; ghi lý do giữ nguyên.
- Mọi trang public có thể đi tới từ README hoặc chỉ mục tài liệu thích hợp.
- Không có link nội bộ hỏng, anchor hỏng, file đích bị export bỏ, hoặc placeholder trình bày như hướng dẫn hoàn chỉnh.
- Bản public không chứa `docs/reports/**` hoặc `docs/baselines/**`; không có link hay hướng dẫn yêu cầu mở hai thư mục này.
- Claim dựa trên nguồn nội bộ phải được giải thích tại trang public, ghi rõ giới hạn tái lập; không sao chép dữ liệu chưa được phép.
- Ví dụ chứa tên giả phải ghi rõ; quickstart dùng symbol thật trong fixture hoặc repository public.
- Quickstart nêu Rust/C compiler, hệ điều hành đã kiểm chứng và cách cài thực tế; không hứa có gói crates.io chưa tồn tại.
- Bảng ngôn ngữ hỗ trợ phản ánh parser thật; không suy từ build smoke trên repo Flutter thành hỗ trợ Dart.
- Kết quả rỗng, ambiguous, capped và stale đều có cách xử lý rõ ràng.
- Mọi headline định lượng đi qua checklist bằng chứng ở mục 3.
- Không có mã phase đứng một mình thay cho tên tính năng ở phần nhập môn.

Lệnh rà soát và phát hiện thủ công; kết quả grep là danh sách cần đánh giá, không phải yêu cầu xóa mọi mã:

```sh
git diff --check
git diff --stat
rg -n '\b(P[0-9]+|F[0-9]+|G[0-9]+|R[0-9]+)\b' README.md docs skills CHANGELOG.md --glob '!docs/reports/**' --glob '!docs/baselines/**'
rg -n 'private evidence vault|Owner approval pending|binary being shipped' README.md docs SUPPORT.md --glob '!docs/reports/**' --glob '!docs/baselines/**'
```

Kiểm tra link cả file và heading anchor bằng công cụ sẵn có; nếu không có, dùng kiểm tra stdlib nhỏ, không thêm dependency.
Trước triển khai, ghi lệnh kiểm tra link cụ thể và quy tắc xử lý link ngoài vào checklist của gói.
Link ngoài cần kiểm tra đích thật; khi mạng không khả dụng, ghi chưa kiểm chứng thay vì pass.

### Kiểm tra hành trình người dùng

Trên checkout sạch, reviewer làm theo hướng dẫn bằng các lệnh đúng như tài liệu:

```sh
cargo install --path . --locked
aw --version
aw --help
aw build
aw symbols src/cli.rs --outline
aw doctor
```

Tiêu chí: cài thành công trên môi trường được công bố; truy vấn trả vị trí có thể mở; không cần đường dẫn riêng của tác giả.
Kiểm tra các ví dụ bổ sung và hướng dẫn MCP theo đúng phạm vi được duyệt; không tự cài vào cấu hình host của người dùng.
Đây là kiểm chứng hướng dẫn cài/sử dụng, không phải chạy test suite cho mỗi diff Markdown.

### Kiểm tra code và artifact xuất bản

- Prose-only: không chạy build/test trong từng vòng biên tập; tái sử dụng kết quả kiểm chứng còn hợp lệ.
- Nếu thay CLI/MCP/export: chạy test đúng phần sửa; thực hiện các kiểm tra contribution/CI liên quan theo `AGENTS.md`.
- Export hiện dùng `git archive HEAD`: chưa kiểm tra được file chưa commit. Không tuyên bố export bao phủ working tree.
- Chỉ chạy export sau khi có revision đã được chủ dự án cho phép chốt; dùng thư mục đầu ra mới, dành riêng cho kiểm chứng.
- Script hiện xóa thư mục `--out` nếu tồn tại; tuyệt đối không trỏ tới source, worktree hoặc thư mục người dùng.
- Đọc tĩnh cho thấy script hiện chỉ loại `docs/baselines/`, chưa loại `docs/reports/`.
  Trước nghiệm thu public, phải bổ sung loại `docs/reports/` và assertion thư mục này không tồn tại trong export.
  Đây là hạng mục sửa export riêng, chưa được thực thi bởi bản cập nhật kế hoạch này.

Lệnh dự kiến sau khi xác nhận revision và biến đường dẫn an toàn:

```sh
python3 tests/scripts/public_export.py --src "$SOURCE_REPO" --out "$NEW_EXPORT_DIR"
```

Script có chạy test suite release trong export. Ghi đây là kiểm chứng artifact riêng, không lặp lại như kiểm tra Markdown.
Kiểm tra tiếp link và nội dung trong chính cây export; PASS của script hiện tại không chứng minh tất cả link hợp lệ.
Không được coi kiểm tra working tree sạch thông tin là chứng minh lịch sử Git sạch; nếu công khai lịch sử, cần đánh giá riêng.

### Quy tắc đóng việc

Bàn giao danh sách file, claim đã sửa, lệnh đã chạy, kết quả và phần chưa kiểm chứng.
Nếu bất kỳ tiêu chí nào thất bại, báo lỗi; không đổi ngưỡng hoặc thu hẹp phạm vi rồi vẫn ghi hoàn tất.
“Tài liệu đạt nghiệm thu” không đồng nghĩa đã được phép publish repository hoặc release.

## 8. Quyết định cần chủ dự án chốt

| Quyết định | Lựa chọn và đánh đổi | Xử lý nếu chưa có câu trả lời |
|---|---|---|
| Cấu trúc tài liệu | Đã triển khai các trang ở mục 4 theo yêu cầu tiến hành kế hoạch | Không cần quyết định bổ sung cho đợt biên tập này |
| Bằng chứng nội bộ | Chỉ công bố tóm tắt có giới hạn: nhanh hơn nhưng không tái lập đầy đủ. Hoặc bổ sung benchmark trên corpus public: mạnh hơn nhưng là công việc đo riêng | Không nhận benchmark nội bộ là tái lập công khai; chưa chạy phép đo mới |
| Giấy phép, liên hệ và kênh hỗ trợ | Maintainer xác nhận nội dung hiện có và kênh GitHub thực sự hoạt động; không suy ra từ tên file hoặc báo cáo lịch sử | Giữ nguyên nội dung pháp lý; ghi trạng thái chưa xác nhận |

Kế hoạch này là artifact điều phối nội bộ. Toàn bộ `docs/reports/**` phải được loại khỏi export; không có ngoại lệ mặc định.
