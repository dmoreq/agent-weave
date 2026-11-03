# Publish preparation — aw library (2026-11-03)

Trạng thái: **chuẩn bị, chưa publish**. Không push, không tag, không
`cargo publish`, không rewrite history, không xóa worktree.

## 1. Quyết định đã chốt

| Hạng mục | Quyết định |
|---|---|
| Kênh | GitHub public repo + crates.io |
| Crate name | `agent-weave` (crates.io trả 404, còn trống); fallback `aw-code-index` (cũng trống) |
| Binary | Giữ tên `aw` qua `[[bin]] name = "aw"` |
| Library crate | Giữ `[lib] name = "aw"` để tests hiện tại dùng `aw::...` |
| Version | `0.3.0`, `publish = false` đến khi review xong |
| Public tree | Giữ scope hiện tại: user guides, `docs/designs/`, `skills/`, scripts cần CI |
| Private evidence | `docs/baselines/` và `docs/reports/` không public |
| History | `git filter-repo` trên clone public; repo private gốc giữ nguyên |
| Worktree | Lưu patch/bundle rồi xóa; chưa chạy trong đợt prep |

## 2. Owner inputs còn thiếu

- GitHub public owner/repo URL; hiện `repository` trỏ `quy-doan/aw` và trả 404.
- crates.io API token và quyền sở hữu `agent-weave`.
- Xác nhận license/legal, NOTICE và kênh báo lỗi.
- README hiện dùng link tương đối `docs/...`; crates.io không phục vụ các file
  này. Trước publish cần link tuyệt đối tới GitHub hoặc `README.crate.md`
  riêng cho crate.
- Review chấp thuận bật `publish = true`.

## 3. Đã chuẩn bị trong đợt này

- `Cargo.toml`: package name `agent-weave` (crates.io 404 confirmed), `[lib] name = "aw"`,
  `[[bin]] name = "aw"`, `documentation`, anchored `include` allowlist.
- `publish = false` vẫn giữ.
- `tests/scripts/public_export.py`: prune recall scripts nội bộ; gate kiểm tra
  `include` allowlist; exporter tự xóa khỏi bản export.
- `tests/scripts/test_public_export.py`: 4 test pass, gồm gate private script.
- `.gitignore`: thêm `.aw/`, `dist/`, `*.tar.gz`.
- `cargo package --list`: 33 files, 685.7 KiB unpacked / 161.2 KiB compressed;
  no docs/baselines, docs/reports, skills, exporter scripts, or recall scripts.
- `cargo package --allow-dirty`: build verification pass.
- `public_export.py` to `/tmp/aw-public-prep-2026-11-03`: PASS; 62 exported
  files; no private names/symbols/paths/emails/secrets; `cargo test --release`
  in export PASS.
- Packaged crate `target/package/agent-weave-0.3.0`: 32 files; `cargo test
  --locked --release` PASS (104 CLI + 26 fixture + 7 MCP).

## 4. Phase B — chưa chạy

### 4.1 Worktree và branch

```sh
mkdir -p /tmp/aw-lab-archive-2026-11-03/{map,sourcefirst}
git -C /tmp/aw-lab/map diff --binary > /tmp/aw-lab-archive-2026-11-03/map/tracked.patch
git -C /tmp/aw-lab/map status --porcelain > /tmp/aw-lab-archive-2026-11-03/map/status.txt
git -C /tmp/aw-lab/map ls-files --others --exclude-standard -z \
  | tar -C /tmp/aw-lab/map --null -T - -czf /tmp/aw-lab-archive-2026-11-03/map/untracked.tar.gz
# lặp lại cho sourcefirst; verify tar/patch trước khi xóa
git worktree remove /tmp/aw-lab/base
git worktree remove --force /tmp/aw-lab/map
git worktree remove --force /tmp/aw-lab/sourcefirst
git branch -d lab/proactive-map lab/search-source-first
git worktree prune
rm -rf /tmp/aw-lab/target-base /tmp/aw-lab/target-map /tmp/aw-lab/target-sourcefirst
```

Chỉ chạy sau khi owner xác nhận patch/bundle mở được và không cần giữ
worktree.

### 4.2 Public history

```sh
git clone --no-local /Users/quy.doan/Workspace/aw /tmp/aw-public-2026-11-03
cd /tmp/aw-public-2026-11-03
git filter-repo --invert-paths --path docs/baselines --path docs/reports \
  --prune-empty=always
git log --all --oneline -- docs/baselines docs/reports   # phải rỗng
git count-objects -vH
```

- Thay tên/corpus/email/path nội bộ trong commit message bằng
  `git filter-repo --replace-text <file>` trước khi push.
- Tag cũ `v0.2.0`/`v0.2.1` không mang sang public; tạo `v0.3.0` sau review.
- Không chạy filter-repo trên repo gốc.

### 4.3 Public export và publish

```sh
python3 -I tests/scripts/public_export.py \
  --src /tmp/aw-public-2026-11-03 \
  --out /tmp/aw-public-export-2026-11-03
# kiểm tra export: không docs/baselines, docs/reports, recall scripts
cargo package --list
cargo publish --dry-run
```

Sau review: đổi `publish = true`, cập nhật `repository`/`homepage` URL thật,
commit, tag `v0.3.0`, push public repo, `cargo publish`.

## 5. Verification prep

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo metadata --no-deps --format-version 1
cargo package --list
python3 -m unittest discover -s tests/scripts -p test_public_export.py -v
python3 -I tests/scripts/public_export.py --src . --out /tmp/aw-public-prep-2026-11-03
```

`cargo package --list` phải có source/schema/build/Cargo.lock/tests; không có
`docs/`, `skills/`, exporter scripts, recall scripts.

## 6. Rollback

- Repo private gốc và worktree không bị sửa trong đợt prep.
- Patch/bundle worktree nằm ngoài repo.
- Public clone/export nằm trong `/tmp`, có thể xóa.
