#!/usr/bin/env python3
"""P7-6am/E：**中文输入态下的硬件光标位置** —— pi-rs 与 TS 的对比。

做法：两侧都在 pty 里键入同一串中英混排文本，抓 `\\x1b[<n>G`（绝对列）与 `\\x1b[<n>A/B`（相对行）序列，
比较**序列列表**（列号是 1-based 显示列）。
"""
import json, os, re, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_harness import Harness

TS = os.environ.get("TS_PI_ENTRY")
TS_CMD = ["node", TS] if TS else None
SEQ = re.compile(r"\x1b\[(\d+)([AGB])")


def collect(cmd, text, kitty=False):
    H = Harness(cols=100, rows=30, respond=True, cmd=cmd, kitty=kitty).start()
    try:
        H.wait_for("Pi can explain", 25)
        H.clean_buffer()
        H.type(text)
        H.pump(0.8)
        raw = bytes(H.buf).decode("utf-8", "replace")
        seqs = [f"{m.group(1)}{m.group(2)}" for m in SEQ.finditer(raw)]
        return {"sequences": seqs[-12:], "last_col": next((int(m.group(1)) for m in reversed(list(SEQ.finditer(raw))) if m.group(2) == "G"), None)}
    finally:
        H.close()


def main() -> int:
    cases = [("ascii", "abc"), ("cjk", "中文"), ("mixed", "a中b")]
    rows = []
    for name, text in cases:
        pi = collect(None, text)
        ts = collect(TS_CMD, text) if TS_CMD else None
        rows.append({"case": name, "text": text, "pi_rs": pi, "ts": ts,
                     "same_last_col": bool(ts) and pi["last_col"] == ts["last_col"]})
    print(json.dumps({"rows": rows}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
