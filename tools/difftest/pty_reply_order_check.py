#!/usr/bin/env python3
"""P7-6ak/B：**应答顺序竞态与 TS 对照**。

对 (DA1 先 / kitty 先) × (间隔 0 / 50 ms) 四组，分别用 pi-rs 与 TS 启动同一场景，
在第 1.5 s 窗口内统计启动阶段写出的关键序列（`\\x1b[>4;2m` / `\\x1b[<u` 等），给出差异表。
"""
import json, os, sys, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_harness import Harness, REPO

TS_ENTRY = os.environ.get("TS_PI_ENTRY")
TS_CMD = ["node", TS_ENTRY] if TS_ENTRY else None
DA1 = b"\x1b[?62;c"
KITTY = b"\x1b[>1u"          # flags=1 ⇒ 激活 kitty
MOK = "\x1b[>4;2m"


def run_once(cmd, order, gap_ms):
    H = Harness(cols=100, rows=30, respond=False, cmd=cmd).start()
    try:
        # 等程序把两个查询都写出来（查询本身出现在输出流里）
        deadline = time.time() + 6
        while time.time() < deadline:
            H.pump(0.05)
            txt = H.text()
            if ("\x1b[>7u" in txt or "[>7u" in txt) and ("\x1b[c" in txt or "[c" in txt):
                break
        replies = [DA1, KITTY] if order == "da1-first" else [KITTY, DA1]
        for r in replies:
            H.write_raw(r)
            if gap_ms:
                time.sleep(gap_ms / 1000)
            H.pump(0.05)
        H.pump(0.6)
        raw = bytes(H.buf).decode("utf-8", "replace")
        return {
            "order": order,
            "gap_ms": gap_ms,
            "mok_count": raw.count(MOK),
            "kitty_pop": raw.count("\x1b[<u"),
            "started": "Pi can explain" in H.text() or "pi v" in H.text(),
        }
    finally:
        H.close()


def main() -> int:
    impls = [("pi-rs", None)]
    if TS_CMD:
        impls.append(("ts", TS_CMD))
    rows = []
    for name, cmd in impls:
        for order in ("da1-first", "kitty-first"):
            for gap in (0, 50):
                r = run_once(cmd, order, gap)
                r["impl"] = name
                rows.append(r)
    # 差异表：同 (order,gap) 下 pi-rs 与 ts 的 mok_count 是否一致
    diffs = []
    for order in ("da1-first", "kitty-first"):
        for gap in (0, 50):
            a = next(r for r in rows if r["impl"] == "pi-rs" and r["order"] == order and r["gap_ms"] == gap)
            b = next((r for r in rows if r["impl"] == "ts" and r["order"] == order and r["gap_ms"] == gap), None)
            if b is None:
                continue
            if a["mok_count"] != b["mok_count"]:
                diffs.append({"order": order, "gap_ms": gap, "pi_rs": a["mok_count"], "ts": b["mok_count"]})
    print(json.dumps({"rows": rows, "mok_count_diffs": diffs}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
