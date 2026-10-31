#!/usr/bin/env python3
"""P7-6ak/C2：**80 档动态 hotkeys 的屏幕级比对**。

从 `fixtures-width.json` 取 cols=80 的行数组（真 TS 组件渲染），与 r3 端到端 `T15c80` 录制
（`/tmp/c5green`，或由参数指定）的去 ANSI 屏幕逐行比对，补上 C5 反证暴露的缺口。
"""
import json, os, re, sys

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ANSI = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)")


def main() -> int:
    raw_path = sys.argv[1] if len(sys.argv) > 1 else "/tmp/c5green/run-1/raw.bin"
    fx = json.load(open(os.path.join(REPO, "tools/difftest/tui-oracle/fixtures-width.json")))
    lines = [l.rstrip() for l in fx["panels"]["hotkeys"]["80"]["plain"]]
    nonempty = [l for l in lines if l.strip()]
    raw = open(raw_path, "rb").read().decode("utf-8", "replace")
    screen = set(l.rstrip() for l in re.split(r"[\r\n]", ANSI.sub("", raw)))
    missing = [l for l in nonempty if l not in screen]
    payload = {"raw": raw_path, "fixture_lines": len(lines), "fixture_nonempty": len(nonempty),
               "missing_in_screen": len(missing), "missing_sample": missing[:3]}
    print(json.dumps(payload, ensure_ascii=False))
    return 0 if not missing else 1


if __name__ == "__main__":
    sys.exit(main())
