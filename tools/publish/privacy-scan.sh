#!/bin/sh
# 公开树隐私扫描（G''c）：密钥/路径/个人信息 + **用户全局提示词特征短语**。
# 只读扫描，不修改任何文件；命中即打印文件清单并以非零退出。
set -eu
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
cd "$ROOT"

PATTERNS='sk-[A-Za-z0-9]{16,}|ghp_[A-Za-z0-9]{20,}|github_pat_|-----BEGIN [A-Z ]*PRIVATE KEY-----|C:\\\\Users|E:\\\\dev|/root/|vdisksvr|系统盘|[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.(com|cn|org|net)'
PROMPT_PATTERNS='N先生|以上回答已遵循所有设定规则|全局提示词|末尾固定|逐条检查'

hits=0
scan() {
  label=$1; pattern=$2
  files=$(git ls-files -z | xargs -0 grep -lE "$pattern" 2>/dev/null || true)
  if [ -n "$files" ]; then
    printf '✗ %s 命中：\n' "$label"
    printf '%s\n' "$files" | sed 's/^/    /' | head -20
    hits=$((hits + 1))
  else
    printf '✓ %s 无命中\n' "$label"
  fi
  # golden 是二进制快照，单独按纯文本扫
  ghits=$(grep -rlE "$pattern" golden/ golden-tui/ golden-tui-light/ 2>/dev/null | head -20 || true)
  if [ -n "$ghits" ]; then
    printf '✗ %s（golden）命中：\n' "$label"
    printf '%s\n' "$ghits" | sed 's/^/    /'
    hits=$((hits + 1))
  else
    printf '✓ %s（golden）无命中\n' "$label"
  fi
}

scan "密钥/本机路径/邮箱" "$PATTERNS"
scan "用户全局提示词特征短语" "$PROMPT_PATTERNS"

if [ "$hits" -gt 0 ]; then
  echo "privacy-scan: 有命中，发布前必须逐项处置" >&2
  exit 1
fi
echo "privacy-scan: 全部干净"
