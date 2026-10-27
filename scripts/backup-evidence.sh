#!/bin/sh
# 把本地证据（不入库的 notes/）打成归档，供换机/回溯用。
#
# 用法：scripts/backup-evidence.sh [输出目录]（默认 ~/archives）
# 归档内容：notes/（含 evidence/、progress、decisions、limits）与 golden/ 的新增基线清单。
set -eu
REPO=$(cd "$(dirname "$0")/.." && pwd)
OUT=${1:-$HOME/archives}
mkdir -p "$OUT"
STAMP=$(date +%Y%m%d-%H%M%S)
TARGET="$OUT/pi-rs-evidence-$STAMP.tar.gz"
cd "$REPO"
tar czf "$TARGET" notes
printf '%s\n' "$(sha256sum "$TARGET" | cut -d' ' -f1)  $(basename "$TARGET")" > "$TARGET.sha256"
echo "已归档：$TARGET"
cat "$TARGET.sha256"
