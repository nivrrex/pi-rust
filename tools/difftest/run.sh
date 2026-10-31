#!/usr/bin/env bash
# tools/difftest/run.sh —— 差分测试的网络隔离包装。
#
# 强制使用 unshare 建立的独立网络命名空间：进去后只有回环，mock-provider 与被测 pi 在同一个
# 命名空间里通过 127.0.0.1 通信，真实请求在物理上发不出去。不提供绕过选项。
#
# 用法：tools/difftest/run.sh <scenario.json> [--runs N] [--out golden]

set -euo pipefail
cd "$(dirname "$0")/../.."

# 跑 pi-rs（DIFTEST_IMPL=pi-rs）时不需要 TS 入口；跑 TS 时必须给。
if [ "${DIFTEST_IMPL:-ts}" != "pi-rs" ]; then
  : "${TS_PI_ENTRY:?必须设置 TS_PI_ENTRY 指向 npm 包里的 dist/bundle/cli.js}"
fi

# node 由 fnm 管理，PATH 在不同 shell 里不一定带 fnm 的 bin 目录，这里兜底补上。
if ! command -v node >/dev/null 2>&1; then
  for d in "$HOME"/.local/share/fnm/node-versions/*/installation/bin; do
    [ -x "$d/node" ] && PATH="$d:$PATH" && break
  done
fi
: "${MOCK_PROVIDER_BIN:=target/debug/mock-provider}"

if [ ! -x "$MOCK_PROVIDER_BIN" ]; then
  export PATH="$HOME/.cargo/bin:$PATH"
  cargo build -q -p mock-provider
fi

if [ "$(id -u)" = "0" ]; then
  UN_SHARE=(unshare -n)
else
  UN_SHARE=(unshare -rn)
fi


exec "${UN_SHARE[@]}" bash -c '
  set -euo pipefail
  ip link set lo up
  exec node tools/difftest/run.mjs "$@"
' -- "$@"
