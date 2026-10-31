#!/usr/bin/env bash
# P7-4e 钉子（W1.3，可移植性见 P7-4h/W2）：**非交互路径**的端到端固定断言。
#
# 为什么需要：P7-4d 曾把"docs 段路径不同"误判为回退，真因是环境里没设 `TS_PI_ENTRY`
# （`tools/difftest/run.mjs:24/164-167/342/586` 用它算 `PI_PACKAGE_DIR`；缺它时 pi-rs 的
# docs 段退化为 `/README.md`，与 golden 差 213 字符 ⇒ A/B/C 全判 different）。
#
# **不给静默默认值**（P7-4h/W2）：必须由环境提供 TS 侧入口，缺了就报错并打印设置说明。
#   方式一（推荐）：export TS_PI_ENTRY=<pi 包目录>/dist/bundle/cli.js
#   方式二：export PI_TS_PACKAGE_DIR=<pi 包目录>   （本脚本自行拼 /dist/bundle/cli.js）
# 断言内容：非交互路径（S01，print 模式）必须 A/B/C/fs/exit 全 same。
set -uo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

if [ -n "${TS_PI_ENTRY:-}" ]; then
  ts_entry="${TS_PI_ENTRY}"
elif [ -n "${PI_TS_PACKAGE_DIR:-}" ]; then
  ts_entry="${PI_TS_PACKAGE_DIR%/}/dist/bundle/cli.js"
else
  cat >&2 <<'MSG'
[P7-4e 钉子] 缺少 TS 侧入口，无法运行非交互路径断言。
  请二选一设置后再运行（本脚本**不提供**静默默认值）：
    export TS_PI_ENTRY=/path/to/pi-package/dist/bundle/cli.js
    export PI_TS_PACKAGE_DIR=/path/to/pi-package
  说明：该变量经 tools/difftest/run.mjs 换算成 PI_PACKAGE_DIR，决定 system 提示里 docs 段的路径；
        缺省会让 pi-rs 的 docs 段与 golden 不同（差 213 字符），从而出现"假回退"。
MSG
  exit 2
fi

out="tmp/precommit-pin-s01"
rm -rf "${out}"
DIFTEST_IMPL=pi-rs TS_PI_ENTRY="${ts_entry}" timeout 240 tools/difftest/run.sh scenarios/S01.json --out "${out}" >/dev/null 2>&1 || true
line="$(node tools/difftest/compare.mjs S01 --out "${out}" 2>&1 | grep -E '^\[compare\] S01:' | head -1 || true)"
expected="[compare] S01: A=same B=same C=same fs=same exit=same"
if [ "${line}" = "${expected}" ]; then
  printf '  非交互路径 S01：%s\n' "${line}"
  exit 0
fi
printf '  非交互路径 S01 断言失败\n  期望: %s\n  实得: %s\n' "${expected}" "${line:-（无输出）}"
exit 1
