#!/usr/bin/env bash
# 仓库约定检查（P8e-10/C-b）。
#
# 1) 含非 ASCII 字符的 `.ps1` **必须**带 UTF-8 BOM：Windows PowerShell 5.1 会把无 BOM 的
#    脚本按系统代码页（GBK）解码，中文引号/花括号被吃掉后 `-File` 直接语法报错
#    （真实踩过：tools/windows/probe-vt-input.ps1 报 3 处语法错）。
# 2) `scripts/*.sh` 与 `tools/` 下的可执行脚本在**索引**里必须是 100755，否则
#    `./scripts/xxx.sh` 在干净 clone 上不可执行（SMB/Windows 挂载会丢 755 位）。
set -uo pipefail
cd "$(dirname "$0")/.."

failed=0

# 1) .ps1 的 BOM
while IFS= read -r file; do
  [ -f "$file" ] || continue
  if LC_ALL=C grep -qP '[^\x00-\x7F]' "$file" 2>/dev/null; then
    head -c 3 "$file" | od -An -tx1 | tr -d ' \n' | grep -q '^efbbbf' || {
      echo "  缺少 UTF-8 BOM（含非 ASCII）：$file"
      failed=1
    }
  fi
done < <(git ls-files '*.ps1')

# 2) 索引可执行位
while IFS= read -r entry; do
  mode=${entry%% *}
  path=${entry#*$'\t'}
  case "$path" in
    scripts/*.sh|tools/*.sh|tools/windows/*.ps1|scripts/hooks/*)
      [ "$mode" = "100755" ] || {
        echo "  索引权限应为 100755，实际 $mode：$path"
        failed=1
      }
      ;;
  esac
done < <(git ls-files -s | awk '$4 ~ /^(scripts|tools)\// {print $1" "$4}')

# 3) 真实用户路径 / 主机名（P9-1-C）：暂存文件优先，没有暂存则扫全部索引文件。
#    目的：P9-1-A 曾把 /root/Desktop/colibri 写进测试与 golden，privacy-scan 在发布前才拦住；
#    这条规则让"提交时"就失败。`docs/evidence/` 与 privacy-scan 一样豁免（内部证据，不入公开树）。
user_name=$(id -un 2>/dev/null || true)
host_name=$(hostname -s 2>/dev/null || hostname 2>/dev/null || true)
scan_list=$(git diff --cached --name-only --diff-filter=ACMR 2>/dev/null || true)
if [ -z "$scan_list" ]; then
  scan_list=$(git ls-files)
fi
while IFS= read -r file; do
  [ -n "$file" ] || continue
  [ -f "$file" ] || continue
  case "$file" in
    # 证据目录与**本脚本自己**豁免（脚本里必然含被扫描的模式字面量）。
    docs/evidence/*) continue ;;
    scripts/check-repo-conventions.sh) continue ;;
  esac
  # 只扫文本文件（`grep -I` 跳过二进制）。
  grep -Iq . "$file" 2>/dev/null || continue
  if grep -qF "/root/Desktop/" "$file" 2>/dev/null; then
    echo "  含本机路径 /root/Desktop/：$file"
    failed=1
  fi
  if [ -n "$user_name" ] && grep -qF "/home/$user_name/" "$file" 2>/dev/null; then
    echo "  含本机路径 /home/$user_name/：$file"
    failed=1
  fi
  if [ -n "$host_name" ] && grep -qwF "$host_name" "$file" 2>/dev/null; then
    echo "  含本机主机名 $host_name：$file"
    failed=1
  fi
done <<< "$scan_list"

if [ "$failed" -ne 0 ]; then
  echo "仓库约定检查失败" >&2
  exit 1
fi
echo "仓库约定检查通过（.ps1 BOM、索引可执行位）"
