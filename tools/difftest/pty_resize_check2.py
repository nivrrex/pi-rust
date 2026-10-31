#!/usr/bin/env python3
"""P7-6aj/M1：缩放（TIOCSWINSZ + SIGWINCH ⇒ ≤1 s 内按新 cols 渲染）。基于 pty_harness。"""
import json, os, sys, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_harness import Harness

COLS1, COLS2 = 100, 60


def main() -> int:
    H = Harness(cols=COLS1, rows=30, respond=True).start()
    try:
        started = H.wait_for("Pi can explain", 25)
        H.type("say hi\r")
        replied = H.wait_for("Hello", 30)
        H.clean_buffer()
        H.resize(COLS2, 30)
        wide_old = ("─" * COLS1).encode()
        wide_new = ("─" * COLS2).encode()
        deadline = time.time() + 1.0
        saw_new = False
        while time.time() < deadline:
            H.pump(0.1)
            text = H.text()
            if "─" * COLS2 in text:
                saw_new = True
                break
        payload = {
            "started": started,
            "replied": replied,
            "cols_before": COLS1,
            "cols_after": COLS2,
            "saw_60wide_rule_within_1s": saw_new,
            "bytes_after_resize": len(H.buf),
        }
        print(json.dumps(payload, ensure_ascii=False))
        return 0 if (started and saw_new) else 1
    finally:
        H.close()


if __name__ == "__main__":
    sys.exit(main())
