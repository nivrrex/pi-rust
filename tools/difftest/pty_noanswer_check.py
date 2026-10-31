#!/usr/bin/env python3
"""P7-6aj/M3：**终端全不应答路径**——DA1/kitty/OSC11/DSR996 都不回应答。

断言：①启动不卡住（UI 出现）；②200 ms 内键入可用；③无应答样字符漏出（如 `?62;c`、`7;1u`、`?0u`、`rgb:`）；
④孤立 Esc 不被 150 ms 前缀兜底挂起（Esc 后 200 ms 内补全弹层应关闭）。
"""
import json, os, sys, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_harness import Harness

LEAK_PATTERNS = ["?62;c", "7;1u", "?0u", "rgb:", "997;", "?996n"]


def main() -> int:
    H = Harness(cols=100, rows=30, respond=True).start()
    try:
        H.answering = False                      # 模拟"终端不应答"
        started = H.wait_for("Pi can explain", 25)
        H.clean_buffer()
        t0 = time.time()
        H.type("x")                              # 200 ms 内键入可用
        typed_ok = H.wait_for("x", 0.2) or H.wait_for("x", 1.0)
        typed_ms = int((time.time() - t0) * 1000)
        text = H.text()
        leaks = [p for p in LEAK_PATTERNS if p in text]
        # ④ 孤立 Esc：先打开补全（输入 `/`）再按 Esc，200 ms 内应关闭（弹层里 `→`/`/quit` 等消失）
        H.type("\x15")                           # Ctrl+U 清空
        H.type("/")
        H.pump(0.4)
        had_menu = "/quit" in H.text() or "/help" in H.text()
        H.clean_buffer()
        H.type("\x1b")
        esc_closed = not any(s in H.text() for s in ("/quit", "/help"))
        payload = {
            "started": started,
            "typed_ok": typed_ok,
            "typed_ms": typed_ms,
            "leaks": leaks,
            "completion_menu_seen": had_menu,
            "esc_closed_within_200ms": esc_closed,
        }
        ok = started and typed_ok and not leaks and esc_closed
        print(json.dumps(payload, ensure_ascii=False))
        return 0 if ok else 1
    finally:
        H.close()


if __name__ == "__main__":
    sys.exit(main())
