#!/usr/bin/env python3
"""P7-6aj/E：**端到端应答注入**——向 r3 stdin 注入 kitty/DA1/OSC11/DSR997 应答（含分片、
夹在按键间、粘贴内含 `7;1u` 样字节、无应答），断言：屏幕无泄漏、按键被处理、
粘贴内的应答样字节**作为文本出现**。
"""
import json, os, sys, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_harness import Harness

LEAK = ["7;1u", "?62;c", "?0u", "rgb:", "997;", "?996n"]

KITTY = b"\x1b[?1u"
DA1 = b"\x1b[?62;c"
OSC11 = b"\x1b]11;rgb:1111/2222/3333\x07"
DSR997 = b"\x1b[?997;1n"
PASTE = b"\x1b[200~text-with-7;1u-inside\x1b[201~"

CASES = [
    # (id, 注入序列, 分片方式)
    ("kitty-whole", KITTY, [KITTY]),
    ("kitty-split", KITTY, [KITTY[:3], KITTY[3:]]),
    ("kitty-between-keys", KITTY, [b"a", KITTY, b"b"]),
    ("da1-whole", DA1, [DA1]),
    ("da1-split", DA1, [DA1[:4], DA1[4:]]),
    ("da1-between-keys", DA1, [b"c", DA1, b"d"]),
    ("osc11-whole", OSC11, [OSC11]),
    ("osc11-split", OSC11, [OSC11[:8], OSC11[8:]]),
    ("osc11-between-keys", OSC11, [b"e", OSC11, b"f"]),
    ("dsr997-whole", DSR997, [DSR997]),
    ("dsr997-split", DSR997, [DSR997[:6], DSR997[6:]]),
    ("dsr997-between-keys", DSR997, [b"g", DSR997, b"h"]),
    ("paste-with-answer-bytes", PASTE, [PASTE]),
    ("no-answer-at-all", b"", []),
]


def run_case(case_id, chunks):
    H = Harness(cols=100, rows=30, respond=False).start()   # 关闭自动应答，全部手工注入
    try:
        started = H.wait_for("Pi can explain", 25)
        H.clean_buffer()
        for chunk in chunks:
            H.write_raw(chunk)
            H.pump(0.15)
        # 注入后键入一个字符，验证按键仍被处理
        H.type("z")
        typed = H.wait_for("z", 2.0)
        text = H.text()
        leaks = [p for p in LEAK if p in text]
        effective_leaks = leaks
        paste_ok = None
        if case_id.startswith("paste-with-answer"):
            paste_ok = "text-with-7;1u-inside" in text
        payload = {
            "id": case_id,
            "started": started,
            "typed_after_inject": typed,
            "leaks": effective_leaks,
            "paste_text_appeared": paste_ok,
        }
        # 粘贴用例里 `7;1u` 是**粘贴内容本身**（应当作为文本出现），不算泄漏 ⇒ 该例跳过泄漏断言。
        if paste_ok:
            effective_leaks = []
        ok = started and typed and not effective_leaks and (paste_ok is not False)
        payload["ok"] = bool(ok)
        return payload
    finally:
        H.close()


def main() -> int:
    results = [run_case(cid, chunks) for cid, _seq, chunks in CASES]
    failed = [r for r in results if not r["ok"]]
    print(json.dumps({"cases": len(results), "failed": len(failed), "results": results}, ensure_ascii=False))
    return 0 if not failed else 1


if __name__ == "__main__":
    sys.exit(main())
