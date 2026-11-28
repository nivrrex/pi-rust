#!/usr/bin/env bash
# 在独立网络命名空间里跑 truncate oracle（不联网；保持一致的做法）。
set -euo pipefail
cd "$(dirname "$0")/../.."
: "${PI_CODING_AGENT_INDEX:?必须设置 PI_CODING_AGENT_INDEX 指向包的 dist/index.js}"
if [ "$(id -u)" = "0" ]; then UN_SHARE=(unshare -n); else UN_SHARE=(unshare -rn); fi
exec "${UN_SHARE[@]}" bash -c '
  set -euo pipefail
  ip link set lo up
  for d in "$HOME"/.local/share/fnm/node-versions/*/installation/bin; do
    [ -x "$d/node" ] && PATH="$d:$PATH" && break
  done
  export PATH
  exec node tools/oracle/edit-oracle.mjs
'
