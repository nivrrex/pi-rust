#!/usr/bin/env bash
# 仓库约定检查的正负对照：合法部署目标名放行，普通文件里的本机主机名必须拦截。
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf -- "$tmp"' EXIT

mkdir -p "$tmp/scripts" "$tmp/tools/deploy"
cp "$root/scripts/check-repo-conventions.sh" "$tmp/scripts/check-repo-conventions.sh"
host_name="$(hostname -s 2>/dev/null || hostname 2>/dev/null || true)"
if [ -z "$host_name" ]; then
  printf '无法取得主机名，自测无法执行\n' >&2
  exit 1
fi

git -C "$tmp" init -q
git -C "$tmp" config user.email test@example.invalid
git -C "$tmp" config user.name test
printf '%s\n' "$host_name" >"$tmp/tools/deploy/install-remote.sh"
git -C "$tmp" add scripts/check-repo-conventions.sh tools/deploy/install-remote.sh
if ! bash "$tmp/scripts/check-repo-conventions.sh" >"$tmp/good.log" 2>&1; then
  printf 'known-good 失败：合法部署目标名被误判\n' >&2
  cat "$tmp/good.log" >&2
  exit 1
fi

printf '%s\n' "$host_name" >"$tmp/leak.txt"
git -C "$tmp" add leak.txt
if bash "$tmp/scripts/check-repo-conventions.sh" >"$tmp/bad.log" 2>&1; then
  printf 'known-bad 失败：普通文件里的本机主机名未被拦截\n' >&2
  exit 1
fi
grep -qF "$host_name" "$tmp/bad.log"
printf 'repo-conventions selftest: 2 项全部通过\n'
