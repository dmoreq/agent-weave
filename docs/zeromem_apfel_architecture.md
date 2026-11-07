# Kiến Trúc Tích Hợp ZeroMem & Apfel: Hệ Sinh Thái Agent Memory Cục Bộ ($0 Token Cost)

**Tài liệu đặc tả kiến trúc hệ thống (Architecture Specification)**  
**Phiên bản:** 1.0  
**Nền tảng mục tiêu:** macOS (Apple Silicon M-Series)  
**Thành phần cốt lõi:** `ZeroMem` (Rust/PyO3/SQLite) + `fastembed` (ONNX Bi-Encoder) + `apfel` (Apple Foundation Model ~3B via ANE) + `Hermes Agent` / `MCP`

---

## 1. Tầm Nhìn & Triết Lý Thiết Kế (Design Philosophy)

Kiến trúc kết hợp giữa **ZeroMem** (dựa trên bài báo *arXiv:2607.29377 - Zero-Mem: Token-Free Memory for LLM Agents*) và **apfel** (công cụ khai thác Apple Foundation Model có sẵn trên macOS) tạo ra một đường ống xử lý bộ nhớ và suy luận **100% On-Device, độ trễ cực thấp và hoàn toàn không phát sinh chi phí LLM token phụ trợ**.

### 1.1. Sự phân tách giữa Memory Pipeline và QA Reader
Các hệ thống bộ nhớ thế hệ cũ (như Mem0, Zep, LangMem) thường gọi LLM ngầm ở bước ghi (để tóm tắt, trích xuất thực thể, phân rã mệnh đề nguyên tử) và bước đọc (để viết lại câu hỏi hoặc định tuyến truy vấn). Điều này gây tốn kém token, tăng độ trễ và làm lệch ngữ nghĩa gốc (hallucination/drift) qua nhiều lần tóm tắt.

Kiến trúc **ZeroMem + apfel** tách biệt rạch ròi hai mặt phẳng:
1. **Memory Plane (Đảm nhiệm bởi ZeroMem + `fastembed`):** Thực hiện toàn bộ quá trình lưu trữ nguyên bản, trích xuất thực thể bằng thuật toán heuristic, nhúng vector hai chiều, dựng đồ thị thực thể - ngữ cảnh (Entity-Context Graph) và truy xuất bằng chứng qua Personalized PageRank (PPR) kết hợp BM25 với **chi phí 0 LLM token**.
2. **Reasoning & Synthesis Plane (Đảm nhiệm bởi `apfel`):** Chỉ tham gia ở chặng cuối cùng (QA Reader / Agent). `apfel` nhận câu hỏi của người dùng cùng với top-$k$ đoạn bằng chứng gốc đã được ZeroMem chắt lọc để tổng hợp câu trả lời trên Apple Neural Engine (ANE).

---

## 2. Sơ Đồ Kiến Trúc Tổng Thể (System Architecture Diagram)

```text
+-----------------------------------------------------------------------------------+
|                               USER / DEVELOPER CLI                                |
|                  (apfel --chat / Hermes Agent / Python Script)                    |
+-------------------+-------------------------------------------+-------------------+
                    | (1) Raw Turn (Ingest)                     | (2) User Query
                    v                                           v
+---------------------------------------+   +---------------------------------------+
|     ZEROMEM INGESTION PIPELINE        |   |      ZEROMEM RETRIEVAL PIPELINE       |
|       (0 LLM Tokens, ~1-3 ms)         |   |        (0 LLM Tokens, <10 ms)         |
+---------------------------------------+   +---------------------------------------+
| 1. Heuristic NER & Lexical Tokenizer  |   | 1. Query Embedding (`fastembed` ONNX) |
| 2. Local Bi-Encoder (`fastembed`)     |   | 2. Seed Node Activation (Cosine +     |
|    -> Dense Vector (384-d)            |   |    Lexical BM25 Matching)             |
| 3. SQLite Append (`mem.db`):          |   | 3. Dual-View Retrieval:               |
|    - Raw Conversation Turns           |   |    - Entity-Context Graph (PPR)       |
|    - Embedding Cache                  |   |    - Temporal Hierarchy Tree          |
+-------------------+-------------------+   | 4. Score Normalization & Fusion       |
                    |                       +-------------------+-------------------+
                    v                                           |
+---------------------------------------+                       |
|      SINGLE-FILE SOURCE OF TRUTH      |                       |
|             `mem.db` (SQLite)         |<----------------------+
|  (Zero Index Drift - Rebuilt on Boot) |   Read Graph / Temporal / BM25 Index
+---------------------------------------+
                                                                | (3) Top-k Evidence
                                                                |     (3-5 Raw Turns)
                                                                v
+-----------------------------------------------------------------------------------+
|                        APFEL INFERENCE ENGINE (QA READER)                         |
|                Apple Foundation Model (~3B Params on Apple Neural Engine)         |
+-----------------------------------------------------------------------------------+
| - Context Budget Guard: 4,096 tokens (M1/M2) or 8,192 tokens (M3+ / macOS 27)     |
| - Sliding Window Strategy: Giữ 4 turns gần nhất + 512 tokens Output Reserve       |
| - Giao tiếp qua: MCP (`zm mcp` -> `zeromem_recall`) hoặc OpenAI API (`:11434`)    |
+-----------------------------------------------------------------------------------+
```

---

## 3. Phân Tích Chuyên Sâu Các Thành Phần Cốt Lõi

### 3.1. Tầng Lưu Trữ Gốc (Single-File Source of Truth)
* **Cơ chế lưu trữ:** Toàn bộ dữ liệu nằm gọn trong một file SQLite duy nhất (`mem.db`), chỉ lưu **lượt hội thoại nguyên bản (raw turns)** và **bộ nhớ đệm vector (embedding cache)**.
* **Chống trôi lệch chỉ mục (Zero Index Drift):** Đồ thị thực thể, cây phân cấp thời gian và chỉ mục từ vựng BM25 không bị ghi cứng thành các trạng thái rời rạc dễ lỗi thời. Khi khởi động, ZeroMem dựng lại (rebuild) các cấu trúc này trong bộ nhớ từ bảng raw turns của SQLite chỉ trong vài chục mili-giây.

### 3.2. Tầng Biểu Diễn Ngữ Nghĩa (`fastembed` vs. `apfel`)
Tại sao bắt buộc dùng `fastembed` bên trong ZeroMem thay vì dùng `apfel` để xử lý ngữ nghĩa?

| Tiêu chí kỹ thuật | `fastembed` (Trong ZeroMem) | `apfel` (Tầng Đọc & Trả lời) |
| :--- | :--- | :--- |
| **Kiến trúc mạng nơ-ron** | **Bi-Encoder** (BERT-based, ~33M–118M params) | **Causal Decoder LLM** (~3B params) |
| **Cơ chế Attention** | Hai chiều (Bidirectional) — gom trọn ngữ nghĩa câu vào 1 vector | Một chiều (Autoregressive) — dự đoán từ tiếp theo |
| **Đầu ra (Output)** | Dense Vector chuẩn hóa (384 chiều) | Chuỗi văn bản (Text stream) / Tool Call JSON |
| **Độ trễ xử lý 1 lượt** | **1 – 3 ms** (chạy qua ONNX Runtime) | **200 – 800 ms** (sinh tuần tự trên ANE) |
| **Hỗ trợ `/v1/embeddings`** | Có (Native trong Rust/Python) | **Không** (Framework Apple không mở cổng vector) |
| **Vai trò kiến trúc** | Tính độ tương đồng Cosine, kích hoạt hạt giống đồ thị (Seed Nodes) | Đọc hiểu bằng chứng gốc và sinh câu trả lời tự nhiên |

> **Lưu ý tối ưu cho Tiếng Việt:** Mô hình mặc định của `fastembed` trong ZeroMem là `BAAI/bge-small-en-v1.5` (tối ưu cho tiếng Anh). Nếu lịch sử hội thoại chứa nhiều tiếng Việt hoặc thuật ngữ pha trộn Anh-Việt, nên cấu hình sang mô hình đa ngôn ngữ **`sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2`** (vẫn giữ nguyên số chiều 384-d và tốc độ ONNX tương đương nhưng bắt ngữ nghĩa tiếng Việt chính xác hơn hẳn).

### 3.3. Tầng Truy Xuất Kép (Dual-View Retrieval Engine)
Khi có truy vấn mới, ZeroMem không dùng LLM để đoán ý mà thực thi đồng thời hai góc nhìn toán học:
1. **Góc nhìn Đồ thị Thực thể - Ngữ cảnh (Entity-Context Graph View):**
   * Dùng vector từ `fastembed` và điểm từ vựng BM25 để tìm các nút hạt giống (seed nodes) trên đồ thị.
   * Chạy thuật toán **Personalized PageRank (PPR)** để lan truyền mức độ kích hoạt qua các cạnh đồng xuất hiện (co-occurrence edges), giúp tìm ra các đoạn hội thoại có liên kết bắc cầu (multi-hop reasoning) dù không trùng khớp từ khóa trực tiếp.
2. **Góc nhìn Phân cấp Thời gian (Temporal Hierarchy View):**
   * Tổ chức các lượt hội thoại theo cấu trúc cây thời gian (Phiên -> Ngày -> Tuần -> Giai đoạn).
   * Đảm bảo các truy vấn mang tính thời điểm hoặc diễn biến trạng thái (ví dụ: *"quyết định cấu hình database gần nhất"*) luôn ưu tiên đúng lát cắt thời gian mới nhất.
3. **Chuẩn hóa & Hợp nhất (Score Normalization):** Điểm từ nhánh Đồ thị (PPR) và nhánh Thời gian được chuẩn hóa về cùng thang phân phối trước khi xếp hạng lấy ra top-$k$ đoạn hội thoại nguyên bản.

---

## 4. Ba Mô Hình Triển Khai Thực Tế (Deployment Patterns)

### Mô hình A: Tích hợp trực tiếp qua MCP (`apfel` CLI Standalone)
Phù hợp khi sử dụng `apfel` trực tiếp trên terminal để hỏi đáp nhanh hoặc tra cứu lịch sử.

```bash
# Khởi chạy apfel ở chế độ chat và gắn ZeroMem làm MCP server
apfel --mcp "zm mcp" \
      --context-strategy sliding-window \
      --context-max-turns 4 \
      --context-output-reserve 512 \
      --chat
```
* **Cơ chế hoạt động:** Khi người dùng đặt câu hỏi cần nhớ lại thông tin cũ, `apfel` tự động gọi tool `zeromem_recall` qua giao thức MCP.
* **Ưu điểm:** Gọn nhẹ, không cần chạy daemon nền.
* **Hạn chế:** Giao thức `zm mcp` thuần túy chỉ phục vụ **Recall (Đọc)**. Để tự động **Ingest (Ghi)** các lượt chat mới vào `mem.db`, cần bọc qua wrapper script hoặc dùng Mô hình B.

### Mô hình B: Tích hợp 2 chiều qua Hermes Agent (Khuyến nghị)
Phù hợp cho luồng làm việc tự động hóa hoàn toàn: vừa tự động ghi nhận mọi lượt trao đổi ở luồng nền, vừa tự động nạp trước (prefetch) bằng chứng trước mỗi câu trả lời.

1. **Bước 1 — Khởi chạy `apfel` làm Local OpenAI-compatible Server:**
   ```bash
   apfel --serve --port 11434
   ```
2. **Bước 2 — Cấu hình `~/.hermes/config.yaml`:**
   ```yaml
   model:
     provider: openai
     base_url: "http://127.0.0.1:11434/v1"
     name: "apple-foundation-model"
     max_tokens: 512

   memory:
     provider: zeromem
     db_path: "~/.zeromem/mem.db"
     top_k: 4
     prefetch: true
     background_sync: true
   ```
* **Cơ chế hoạt động:**
  * Trước mỗi lượt gọi `apfel`, Hermes Agent tự động chạy `zeromem` prefetch (<10ms) để lấy 4 đoạn bằng chứng liên quan nhất ghép vào System Prompt.
  * Ngay khi `apfel` trả lời xong, một luồng nền (background thread) của Hermes đẩy cặp câu hỏi - trả lời nguyên bản vào `mem.db` thông qua `fastembed` mà không làm chậm giao diện chat.

### Mô hình C: Nhúng trực tiếp trong Pipeline Python (`PyO3` + `apfel --serve`)
Phù hợp khi xây dựng các kịch bản xử lý dữ liệu hoặc multi-agent tùy chỉnh bằng Python.

```python
import requests
import zeromem_py  # Rust bindings qua PyO3

# 1. Khởi tạo engine bộ nhớ cục bộ
mem = zeromem_py.ZeroMem(db_path="project_memory.db")

# 2. Ghi nhận sự kiện mới (0 LLM token, ~2ms)
mem.add_turn(role="user", content="Chốt dùng DuckDB và Polars cho pipeline geocoding.")

# 3. Truy xuất bằng chứng khi có câu hỏi (PPR + BM25 + Temporal)
query = "Pipeline geocoding đang dùng công cụ gì?"
evidences = mem.recall(query=query, top_k=3)
context_block = "\n".join([f"- [{e.timestamp}] {e.content}" for e in evidences])

# 4. Gửi sang apfel local server để tổng hợp câu trả lời
resp = requests.post(
    "http://127.0.0.1:11434/v1/chat/completions",
    json={
        "model": "default",
        "messages": [
            {"role": "system", "content": f"Dựa vào bằng chứng sau để trả lời ngắn gọn:\n{context_block}"},
            {"role": "user", "content": query}
        ],
        "temperature": 0.2
    }
)
print(resp.json()["choices"][0]["message"]["content"])
```

---

## 5. Quản Trị Ngân Sách Ngữ Cảnh (Context Budget Management)

Điểm nghẽn lớn nhất của Apple Foundation Model bên dưới `apfel` là kích thước cửa sổ ngữ cảnh chặt chẽ (**4,096 tokens** trên chip M1/M2 và **8,192 tokens** trên chip M3/M4 chạy macOS 27). Để hệ thống hoạt động ổn định tuyệt đối, cần tuân thủ phân bổ ngân sách token như sau:

| Thành phần trong Prompt | Ngân sách Token (Mức 4K Window) | Quy tắc kiểm soát trong Kiến trúc |
| :--- | :--- | :--- |
| **System Prompt & MCP Tool Schema** | ~400 tokens | Chỉ bật đúng 2 tool của ZeroMem (`zeromem_recall`, `zeromem_stats`), tắt các MCP tool dư thừa. |
| **ZeroMem Retrieved Evidence** | ~800 – 1,200 tokens | Giới hạn cứng `top_k = 3` đến `5`. Mỗi đoạn bằng chứng là một raw turn ngắn gọn, không nhồi toàn bộ lịch sử. |
| **Active Sliding Window (Recent Chat)** | ~1,500 tokens | Đặt `APFEL_CONTEXT_STRATEGY=sliding-window` và `APFEL_CONTEXT_MAX_TURNS=4`. Các lượt cũ hơn tự động rơi khỏi RAM nhưng đã nằm trong SQLite của ZeroMem. |
| **Current User Query** | ~300 tokens | Câu hỏi hoặc đoạn code ngắn hiện tại. |
| **Output Reserve (Phần trả lời)** | 512 tokens | Cố định qua cờ `--context-output-reserve 512` để tránh lỗi cắt cụt câu trả lời giữa chừng. |
| **Tổng cộng** | **~3,512 – 3,912 / 4,096 tokens** | **An toàn tuyệt đối, không bao giờ xảy ra lỗi Context Overflow.** |

---

## 6. Nguyên Tắc Vàng Khi Vận Hành (Anti-Patterns Cần Tránh)

1. **KHÔNG dùng `apfel` để tóm tắt hội thoại trước khi lưu vào ZeroMem:**
   * *Lý do:* Làm mất thông tin chi tiết (tên biến, đường dẫn file, con số cụ thể), gây tốn thời gian suy luận của ANE và đi ngược lại triết lý lưu trữ nguyên bản (raw turns) của ZeroMem.
2. **KHÔNG ép `apfel` chấm điểm độ liên quan (Reranking) trên tập dữ liệu lớn:**
   * *Lý do:* Tổ hợp `fastembed` (Cosine) + `BM25` + `Personalized PageRank` trong Rust đã giải quyết bài toán xếp hạng trong `<10ms`. Đưa hàng chục đoạn văn cho LLM 3B đọc để lọc lại sẽ làm độ trễ tăng gấp 100 lần.
3. **KHÔNG bật chiến lược `summarize` của `apfel` khi đã có ZeroMem:**
   * *Lý do:* Khi đầy context, chế độ `summarize` của `apfel` sẽ gọi LLM để tóm tắt lịch sử chat. Hãy luôn dùng `sliding-window` để cắt bỏ thẳng các lượt cũ khỏi bộ nhớ đệm của `apfel`, vì ZeroMem đã giữ bản gốc an toàn trong `mem.db`.
