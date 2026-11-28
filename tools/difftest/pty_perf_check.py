#!/usr/bin/env python3
"""P7-6ao/E：**可复现的多轮性能测量**（等本轮结算后再计时）。

每轮：输入 `next` → 等 `Working` 从末帧消失（结算）→ 键入 `x` 并计时到回显 → 记录
该窗口写出字节数。输出每轮的 击键延迟(ms) 与 每帧平均写出字节。
"""
import json, os, re, sys, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_harness import Harness

SC = os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))), "scenarios-tui/T24-perf200.json")
ANSI = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)")


def last_frame_text(H):
    """取**最后一帧**（r3 每帧以 `\x1b[?2026h` 开始）的纯文本。"""
    raw = bytes(H.buf)
    parts = raw.split(b"\x1b[?2026h")
    frame = parts[-1] if len(parts) > 1 else raw
    return ANSI.sub("", frame.decode("utf-8", "replace")).replace("\r", "")


def tail_text(H, n=8):
    return "\n".join(last_frame_text(H).split("\n")[-n:])


def settled(H, timeout=120.0) -> bool:
    end = time.time() + timeout
    while time.time() < end:
        H.pump(0.2)
        if "Working" not in tail_text(H):
            return True
    return False


def measure(cmd, label, turns):
    H = Harness(cols=100, rows=30, scenario=SC, respond=True, cmd=cmd).start()
    rows = []
    try:
        if not H.wait_for("Pi can explain", 40):
            return {"impl": label, "error": "startup timeout"}
        done = 0
        for turn in turns:
            while done < turn - 1:
                H.type("next\r")
                if not settled(H):
                    rows.append({"turn": done + 1, "error": "turn did not settle"})
                done += 1
            H.clean_buffer()
            t0 = time.time()
            H.type("x")
            ok = H.wait_for("x", 60)
            latency = int((time.time() - t0) * 1000)
            rows.append({"turn": turn, "settled": True, "key_latency_ms": latency,
                         "bytes_in_window": len(H.buf), "echo_seen": ok})
    finally:
        H.close()
    return {"impl": label, "rows": rows}


def main() -> int:
    turns = [1, 5, 10, 20]
    res = [measure(None, "pi-rs", turns)]
    if os.environ.get("TS_PI_ENTRY"):
        res.append(measure(["node", os.environ["TS_PI_ENTRY"]], "ts", turns))
    print(json.dumps(res, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
