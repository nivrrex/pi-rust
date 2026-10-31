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

if [ "$failed" -ne 0 ]; then
  echo "仓库约定检查失败" >&2
  exit 1
fi
echo "仓库约定检查通过（.ps1 BOM、索引可执行位）"
