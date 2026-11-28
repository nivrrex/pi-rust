#!/bin/sh
# F238：TUI 运行期不得直接写 stdout/stderr。
#
# 为什么：TUI 是 inline 渲染，stderr 与画面挂在**同一个 PTY** 上。在主循环画完一帧
# 之后写 stderr，那段字节落在**当前光标位置**——也就是编辑器/输入框那一行，
# 用户看到的就是"错误信息出现在输入框里"（真机反馈）。
#
# 约定：运行期告警走 `pi_cli::tui_notices::warn()`（TUI 入队→转录区渲染；
# 其它模式回落 stderr）。因此这几个文件里不该再有裸写：
#   - crates/pi-cli/src/tui.rs          仅 TUI 主循环
#   - crates/pi-cli/src/ext_tools.rs    扩展宿主回调（运行期高频）
#   - crates/pi-ext-host/src/client.rs  宿主客户端，跑在主进程内
#
# 唯一的例外是 `pi-ext-host/src/client.rs` 里 `fn warn()` 的 `None =>` 回落分支。
set -eu
root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$root"

fail=0
check() { # check <file>
    file="$1"
    hits=$(grep -n 'eprintln!\|eprint!\|println!\|print!' "$file" 2>/dev/null \
        | grep -v '^[0-9]*: *//' \
        | grep -v '`eprintln!\|`println!' || true)
    # 允许两类例外：
    # ① client.rs 没有 sink 时的 stderr 回落（非 TUI 模式）与子进程转发的非 TUI 分支；
    # ② ext_tools 的两处：启动期致命参数错误（随即 `exit(1)`，首帧之前）
    #    与 `flush_extension_log`（TUI 退出、`guard.restore()` 之后一次性输出日志）。
    if [ "$file" = "crates/pi-ext-host/src/client.rs" ]; then
        hits=$(printf '%s\n' "$hits" | grep -v 'None => eprintln!' | grep -v 'None => eprint!' || true)
    fi
    if [ "$file" = "crates/pi-cli/src/ext_tools.rs" ]; then
        hits=$(printf '%s\n' "$hits" | grep -v 'requires a value' | grep -v 'Unknown option' \
            | grep -v 'eprint!("{text}")' | grep -v 'eprintln!();' || true)
    fi
    if [ -n "$(printf '%s' "$hits" | tr -d '[:space:]')" ]; then
        echo "check-tui-stderr: $file 里有裸写 stdout/stderr（应改用 tui_notices::warn）：" >&2
        printf '%s\n' "$hits" >&2
        fail=1
    fi
}

check crates/pi-cli/src/tui.rs
check crates/pi-cli/src/ext_tools.rs
check crates/pi-ext-host/src/client.rs

[ "$fail" -eq 0 ] && echo "check-tui-stderr: OK"
exit "$fail"
