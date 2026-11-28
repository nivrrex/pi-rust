#!/bin/sh
# `check-no-stray.sh` 的判定自检：确认它能把 **pty-helper 拉起的 pi-rs 残留**
# 判成"自动化残留"（退出码 1），而不是当成"人工会话"放过（退出码 0）。
#
# 为什么需要：F118 之前，那条命令行里既没有 `scenarios-tui/` 也没有 `--scenario`
# （场景路径在 helper 自己的参数里），于是测试孤儿被当成人工会话、检查照样报通过。
# 修完后本脚本钉住这个行为——用 `exec -a` 伪造一个同名 argv 的 `sleep` 进程，
# 不依赖真实 pi-rs，也不会跑任何会话。
#
# 用法：`sh scripts/check-no-stray-selftest.sh`（只读，不改仓库、不杀别人的进程）。
set -eu

REPO=$(cd "$(dirname "$0")/.." && pwd)
FAKE_ARGV="$REPO/target/debug/pi-rs --session /tmp/no-stray-selftest tools/difftest/pty-helper.py"

cleanup() {
  if [ -n "${FAKE_PID:-}" ] && [ -d "/proc/$FAKE_PID" ]; then
    kill -TERM "$FAKE_PID" 2>/dev/null || true
    sleep 0.3
    [ -d "/proc/$FAKE_PID" ] && kill -KILL "$FAKE_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

# 造一个**真可执行**的假 pi-rs：路径写成 `…/target/debug/pi-rs`（脚本的 PATTERN 认这个），
# 参数里带 `tools/difftest/pty-helper.py`。用临时目录，不碰仓库。
FAKE_ROOT=$(mktemp -d)
mkdir -p "$FAKE_ROOT/target/debug"
cat > "$FAKE_ROOT/target/debug/pi-rs" <<'FAKE'
#!/bin/sh
sleep 30
FAKE
chmod +x "$FAKE_ROOT/target/debug/pi-rs"
# 脚本的 PATTERN 匹配的是**路径子串** `target/debug/pi-rs`，所以临时目录下的也命中。
"$FAKE_ROOT/target/debug/pi-rs" --session /tmp/no-stray-selftest "$REPO/tools/difftest/pty-helper.py" >/dev/null 2>&1 &
FAKE_PID=$!
cleanup_extra() {
  rm -rf "$FAKE_ROOT"
}
trap 'cleanup; cleanup_extra' EXIT INT TERM
sleep 0.5

if [ ! -d "/proc/$FAKE_PID" ]; then
  echo "check-no-stray-selftest: 假进程没能启动" >&2
  exit 2
fi

status=0
sh "$REPO/scripts/check-no-stray.sh" >/dev/null 2>&1 || status=$?
if [ "$status" -eq 0 ]; then
  echo "check-no-stray-selftest: FAILED —— pty-helper 残留被当成了人工会话（退出码 0）" >&2
  exit 1
fi

echo "check-no-stray-selftest: OK（pty-helper 残留被正确判为自动化，退出码 $status）"
