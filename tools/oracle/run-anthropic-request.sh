#!/usr/bin/env bash
# 在独立网络命名空间里跑 Anthropic 请求体 oracle（命名空间内只有回环，mock 也在回环上）。
set -euo pipefail
cd "$(dirname "$0")/../.."
: "${PI_AI_PKG:?必须设置 PI_AI_PKG 指向 npm 包里的 pi-ai 目录}"
if [ "$(id -u)" = "0" ]; then UN_SHARE=(unshare -n); else UN_SHARE=(unshare -rn); fi
exec "${UN_SHARE[@]}" bash -c '
  set -euo pipefail
  ip link set lo up
  for d in "$HOME"/.local/share/fnm/node-versions/*/installation/bin; do
    [ -x "$d/node" ] && PATH="$d:$PATH" && break
  done
  export PATH
  export PI_OFFLINE=1 PI_TELEMETRY=0
  exec node tools/oracle/anthropic-request-oracle.mjs
'
