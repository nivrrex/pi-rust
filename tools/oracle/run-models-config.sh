#!/usr/bin/env bash
# 在独立网络命名空间里跑 models.json 解析 oracle（命名空间内只有回环）。
set -euo pipefail
cd "$(dirname "$0")/../.."
if [ "$(id -u)" = "0" ]; then UN_SHARE=(unshare -n); else UN_SHARE=(unshare -rn); fi
exec "${UN_SHARE[@]}" bash -c '
  set -euo pipefail
  ip link set lo up
  for d in "$HOME"/.local/share/fnm/node-versions/*/installation/bin; do
    [ -x "$d/node" ] && PATH="$d:$PATH" && break
  done
  export PATH
  export PI_OFFLINE=1 PI_TELEMETRY=0
  exec node tools/oracle/models-config-oracle.mjs
'
