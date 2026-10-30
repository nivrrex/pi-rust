#!/bin/sh
# 生成 MANIFEST（G''d）：记录构建 HEAD、发布提交、tree 与四份产物哈希。
# 用法：write-manifest.sh <包目录> <发布提交> "<产物名列表>"
set -eu
OUT=${1:?包目录}
COMMIT=${2:?发布提交}
ARTIFACTS=${3:?产物列表}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)

HEAD_SHA=$(git -C "$ROOT" rev-parse HEAD)
PUBLISH_TREE=$(git -C "$ROOT" rev-parse "$COMMIT^{tree}")
VERSION=$("$ROOT"/target/release/pi-rs --version 2>/dev/null | head -1 || echo unknown)

{
  printf 'schema: 1\n'
  printf 'buildHead: %s\n' "$HEAD_SHA"
  printf 'publishCommit: %s\n' "$(git -C "$ROOT" rev-parse "$COMMIT")"
  printf 'publishTree: %s\n' "$PUBLISH_TREE"
  printf 'sourceTreeOfBuildHead: %s\n' "$(git -C "$ROOT" rev-parse "$HEAD_SHA^{tree}")"
  printf 'version: %s\n' "$VERSION"
  printf 'artifacts:\n'
  for f in $ARTIFACTS; do
    printf '  %s %s\n' "$(sha256sum "$OUT/$f" | awk '{print $1}')" "$f"
  done
} > "$OUT/MANIFEST"
printf '✓ MANIFEST 已写入（buildHead=%s）\n' "$HEAD_SHA"
