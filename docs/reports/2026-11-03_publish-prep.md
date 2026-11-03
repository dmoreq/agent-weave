# Publish preparation — kg library (2026-11-03)

Trạng thái: **chuẩn bị, chưa publish**. Không push, không tag, không
`cargo publish`, không rewrite history, không xóa worktree.

## 1. Quyết định đã chốt

| Hạng mục | Quyết định |
|---|---|
| Kênh | GitHub public repo + crates.io |
| Crate name | `agent-kg` (crates.io trả 404, còn trống); fallback `kg-code-index` (cũng trống) |
| Binary | Giữ tên `kg` qua `[[bin]] name = "kg"` |
| Library crate | Giữ `[lib] name = "kg"` để tests hiện tại dùng `kg::...` |
| Version | `0.3.0`, `publish = false` đến khi review xong |
| Public tree | Giữ scope hiện tại: user guides, `docs/designs/`, `skills/`, scripts cần CI |
| Private evidence | `docs/baselines/` và `docs/reports/` không public |
| History | `git filter-repo` trên clone public; repo private gốc giữ nguyên |
| Worktree | Lưu patch/bundle rồi xóa; chưa chạy trong đợt prep |

## 2. Owner inputs còn thiếu

- GitHub public owner/repo URL; hiện `repository` trỏ `quy-doan/kg` và trả 404.
- crates.io API token và quyền sở hữu `agent-kg`.
- Xác nhận license/legal, NOTICE và kênh báo lỗi.
- Review chấp thuận bật `publish = true`.

## 3. Đã chuẩn bị trong đợt này

- `Cargo.toml`: package name `agent-kg` (crates.io 404 confirmed), `[lib] name = "kg"`,
  `[[bin]] name = "kg"`, `documentation`, anchored `include` allowlist.
- `publish = false` vẫn giữ.
- `tests/scripts/public_export.py`: prune recall scripts nội bộ; gate kiểm tra
  `include` allowlist; exporter tự xóa khỏi bản export.
- `tests/scripts/test_public_export.py`: 4 test pass, gồm gate private script.
- `.gitignore`: thêm `.kg/`, `dist/`, `*.tar.gz`.
- `cargo package --list`: 33 files, 685.7 KiB unpacked / 161.2 KiB compressed;
  no docs/baselines, docs/reports, skills, exporter scripts, or recall scripts.
- `cargo package --allow-dirty`: build verification pass.

## 4. Phase B — chưa chạy

### 4.1 Worktree và branch

```sh
mkdir -p /tmp/kg-lab-archive-2026-11-03/{map,sourcefirst}
git -C /tmp/kg-lab/map diff --binary > /tmp/kg-lab-archive-2026-11-03/map/tracked.patch
git -C /tmp/kg-lab/map status --porcelain > /tmp/kg-lab-archive-2026-11-03/map/status.txt
git -C /tmp/kg-lab/map ls-files --others --exclude-standard -z \
  | tar -C /tmp/kg-lab/map --null -T - -czf /tmp/kg-lab-archive-2026-11-03/map/untracked.tar.gz
# lặp lại cho sourcefirst; verify tar/patch trước khi xóa
git worktree remove /tmp/kg-lab/base
git worktree remove --force /tmp/kg-lab/map
git worktree remove --force /tmp/kg-lab/sourcefirst
git branch -d lab/proactive-map lab/search-source-first
git worktree prune
rm -rf /tmp/kg-lab/target-base /tmp/kg-lab/target-map /tmp/kg-lab/target-sourcefirst
```

Chỉ chạy sau khi owner xác nhận patch/bundle mở được và không cần giữ
worktree.

### 4.2 Public history

```sh
git clone --no-local /Users/quy.doan/Workspace/kg /tmp/kg-public-2026-11-03
cd /tmp/kg-public-2026-11-03
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
  --src /tmp/kg-public-2026-11-03 \
  --out /tmp/kg-public-export-2026-11-03
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
python3 -I tests/scripts/public_export.py --src . --out /tmp/kg-public-prep-2026-11-03
```

`cargo package --list` phải có source/schema/build/Cargo.lock/tests; không có
`docs/`, `skills/`, exporter scripts, recall scripts.

## 6. Rollback

- Repo private gốc và worktree không bị sửa trong đợt prep.
- Patch/bundle worktree nằm ngoài repo.
- Public clone/export nằm trong `/tmp`, có thể xóa.
