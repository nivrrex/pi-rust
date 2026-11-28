#!/bin/sh
# 生成发布包（本地执行；不联网、不使用令牌）。
#
# 用法：sh tools/publish/build-package.sh <输出目录> [产物目录] [发布提交]
#   产物目录默认 ${HOME}/archives；发布提交默认 refs/heads/publish-v0.87.1
set -eu
OUT=${1:?用法: build-package.sh <输出目录> [产物目录] [发布提交]}
SRC=${2:-${HOME}/archives}
COMMIT=${3:-publish-v0.87.1}  # shellcheck disable=SC2034
BACKUP_BUNDLE=${BACKUP_BUNDLE:-${HOME}/archives/pi-rust-remote-backup-20261029.bundle}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)

if [ -d "$OUT" ] && [ -n "$(ls -A "$OUT" 2>/dev/null)" ]; then
  echo "输出目录非空：$OUT" >&2
  exit 1
fi
mkdir -p "$OUT"

ARTIFACTS="pi-rs-linux-amd64 pi-rs-linux-amd64-musl pi-rs-linux-arm64 pi-rs-windows-amd64.exe"
for f in $ARTIFACTS; do cp "$SRC/$f" "$OUT/$f"; done
( cd "$OUT" && sha256sum $ARTIFACTS > SHA256SUMS )  # shellcheck disable=SC2086

# 发布用的单一提交（含 tag），打成 bundle 供脚本 push
git -C "$ROOT" bundle create "$OUT/pi-rs.git.bundle" "$COMMIT"
git -C "$ROOT" clone --quiet --no-hardlinks "$ROOT" "$OUT/repo"
git -C "$OUT/repo" fetch --quiet "$OUT/pi-rs.git.bundle" "$COMMIT:refs/heads/publish"
git -C "$OUT/repo" checkout --quiet refs/heads/publish

# MANIFEST：把包与构建产物绑死（G''d）。构建 HEAD、发布提交、tree 与产物哈希都记下来。
"$ROOT"/tools/publish/write-manifest.sh "$OUT" "$COMMIT" "$ARTIFACTS"

cp "$ROOT/tools/publish/publish.sh" "$OUT/publish.sh"
cp "$ROOT/tools/publish/release-body.md" "$OUT/发布正文.md"
chmod +x "$OUT/publish.sh"

cat > "$OUT/发布说明.md" <<MD
# pi-rs $COMMIT 发布说明

- 版本：pi 0.87.1 的 Rust 重写（TUI / print / json / rpc）
- 产物：linux amd64（glibc）、linux amd64（musl static）、linux arm64（musl static）、
  windows amd64（实验性）
- 校验：同目录 \`SHA256SUMS\`，\`sha256sum -c SHA256SUMS\`
- 已知限制：见仓库 \`LIMITATIONS.md\`

## 发布步骤（三行）
\`\`\`sh
cd $OUT
export GITHUB_TOKEN=<你的令牌>
sh publish.sh
\`\`\`

验证：脚本最后会重新下载每个附件并逐个比对 sha256，全部 ✓ 即成功；
失败会停下并打印回滚提示（指向同款备份 bundle：$(basename "$BACKUP_BUNDLE")）。
MD

printf '✓ 发布包已生成：%s\n' "$OUT"
ls -1 "$OUT" | sed 's/^/  /'
