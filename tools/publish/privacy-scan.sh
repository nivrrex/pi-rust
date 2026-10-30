#!/bin/sh
# 公开树隐私扫描（G''c）：密钥/路径/个人信息 + **用户全局提示词特征短语**。
# 只读扫描，不修改任何文件；命中即打印文件清单并以非零退出。
set -eu
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
cd "$ROOT"

PATTERNS='sk-[A-Za-z0-9]{16,}|ghp_[A-Za-z0-9]{20,}|github_pat_|-----BEGIN [A-Z ]*PRIVATE KEY-----|C:\\\\Users|E:\\\\dev|/root/|vdisksvr|系统盘|[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.(com|cn|org|net)'
# 只放**用户全局提示词独有**的短语。`逐条检查` 之类的通用词会把 fixture 文本
# （场景自带的 skill/prompt 示例）误判为用户数据——P8e-14 就踩过这个坑。
PROMPT_PATTERNS='N先生|以上回答已遵循所有设定规则|全局提示词|用户的偏好|不允许编造来源'

ALLOW="$ROOT/tools/publish/privacy-allowlist.txt"
hits=0

# 命中是否已登记豁免（按路径前缀）
allowed() {
  [ -f "$ALLOW" ] || return 1
  while IFS=$(printf '\t') read -r prefix _reason; do
    case "$prefix" in ''|'#'*) continue ;; esac
    case "$1" in "$prefix"*) return 0 ;; esac
  done < "$ALLOW"
  return 1
}
scan() {
  label=$1; pattern=$2
  # 扫描脚本自身必然含模式串 ⇒ 排除它自己
  raw=$(git ls-files -z | grep -zv -e '^tools/publish/privacy-scan.sh$' -e '^tools/publish/privacy-allowlist.txt$' | xargs -0 grep -lE "$pattern" 2>/dev/null || true)
  files=""
  for hit in $raw; do
    if allowed "$hit"; then printf '  （豁免）%s\n' "$hit"; else files="$files $hit"; fi
  done
  if [ -n "$files" ]; then
    printf '✗ %s 命中：\n' "$label"
    for f in $files; do printf '    %s\n' "$f"; done | head -20
    hits=$((hits + 1))
  else
    printf '✓ %s 无命中\n' "$label"
  fi
  # golden 是二进制快照，单独按纯文本扫
  ghits=""
  for hit in $(grep -rlE "$pattern" golden/ golden-tui/ golden-tui-light/ 2>/dev/null | head -20 || true); do
    if allowed "$hit"; then printf '  （豁免）%s\n' "$hit"; else ghits="$ghits $hit"; fi
  done
  if [ -n "$ghits" ]; then
    printf '✗ %s（golden）命中：\n' "$label"
    for f in $ghits; do printf '    %s\n' "$f"; done
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
