#!/usr/bin/env bash
# tools/oracle/run.sh —— 在独立网络命名空间里跑 oracle。
# 命名空间内只有回环，mock 服务与 pi-ai 走 127.0.0.1，真实网络不可达。
set -euo pipefail
cd "$(dirname "$0")/../.."

: "${PI_AI_PKG:?必须设置 PI_AI_PKG 指向 npm 包里的 pi-ai 目录}"

# root 下用 -n，非 root 下用 -rn（user namespace）。
if [ "$(id -u)" = "0" ]; then
  UN_SHARE=(unshare -n)
else
  UN_SHARE=(unshare -rn)
fi

exec "${UN_SHARE[@]}" bash -c '
  set -euo pipefail
  ip link set lo up
  exec node tools/oracle/run-oracle.mjs tools/oracle/manifest.json
'
