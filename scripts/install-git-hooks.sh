#!/usr/bin/env bash
# 把版本化的钩子装进 .git/hooks（幂等）。Linux/Windows(Git Bash) 均可运行。
set -euo pipefail
root="$(git rev-parse --show-toplevel)"
mkdir -p "$root/.git/hooks"
cp "$root/scripts/hooks/pre-commit" "$root/.git/hooks/pre-commit"
chmod +x "$root/.git/hooks/pre-commit"
echo "已安装 .git/hooks/pre-commit（来源 scripts/hooks/pre-commit）"
