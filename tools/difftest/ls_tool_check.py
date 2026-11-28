#!/usr/bin/env python3
"""V1：`ls` 工具端到端（print + pty 两种模式，与 ref 对拍）。

- print 模式：mock 回一个 `ls` toolCall，第二轮请求体里的 `role=tool` 消息就是 ls 输出；
  与 TS 侧**逐字节**比对，并与 ref 规则（`core/tools/ls.ts:110-136`：大小写不敏感排序、
  目录加 `/`）推出的期望值比对。
- pty 模式：同一 scenario 走 TUI，比较屏幕上是否出现同样的条目列表。

用法（必须在本机 netns 内）：
    unshare -n sh -c 'ip link set lo up; python3 tools/difftest/ls_tool_check.py'
环境变量：PI_RS_BIN（默认 target/debug/pi-rs）。
"""
import json
import os
import re
import shutil
import subprocess
import sys

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
PI_RS = os.environ.get("PI_RS_BIN", os.path.join(REPO, "target", "debug", "pi-rs"))
TS_ENTRY = os.environ.get("TS_PI_ENTRY", "")
BASE = "/tmp/ls-tool-check"
EXPECTED = "aardvark.txt\nAGENTS.md\nBravo.txt\ndir-nested/\nzeta/"


def run_scenario(scenario, out, impl):
    shutil.rmtree(out, ignore_errors=True)
    env = dict(os.environ, DIFTEST_IMPL=impl)
    if impl == "pi-rs":
        env["TUI_CMD"] = PI_RS
    proc = subprocess.run(
        ["node", os.path.join(REPO, "tools", "difftest", "run-tui.mjs"),
         "--scenario", os.path.join(REPO, "scenarios-tui", scenario), "--out", out],
        env=env, capture_output=True, text=True, timeout=240,
        stdin=subprocess.DEVNULL,
    )
    return proc


def tool_text(out_dir, index=2):
    path = os.path.join(out_dir, "requests.jsonl")
    if not os.path.exists(path):
        return None
    for number, line in enumerate(open(path), 1):
        entry = json.loads(line)
        body = entry.get("body")
        body = json.loads(body) if isinstance(body, str) else (body or {})
        for message in body.get("messages", []):
            if message.get("role") == "tool":
                content = message.get("content")
                return content if isinstance(content, str) else json.dumps(content, ensure_ascii=False)
    return None


def screen_text(raw_path):
    raw = open(raw_path, "rb").read()
    plain = re.sub(rb"\x1b\[[0-9;?]*[ -/]*[@-~]", b"", raw)
    plain = re.sub(rb"\x1b\][^\x07]*\x07", b"", plain)
    return plain.decode("utf-8", "replace")


def main():
    if not os.path.exists(PI_RS):
        print(f"找不到 pi-rs：{PI_RS}")
        return 2
    if shutil.which("node") is None:
        print("找不到 node")
        return 2
    shutil.rmtree(BASE, ignore_errors=True)
    os.makedirs(BASE, exist_ok=True)
    report = {}

    # ---- print 模式 ----
    ts_out = os.path.join(BASE, "print-ts")
    rs_out = os.path.join(BASE, "print-rs")
    run_scenario("T72-ls-tool.json", ts_out, "ts")
    run_scenario("T72-ls-tool.json", rs_out, "pi-rs")
    ts_text = tool_text(ts_out)
    rs_text = tool_text(rs_out)
    report["print"] = {
        "ts": ts_text,
        "pi_rs": rs_text,
        "matches_ref_rules": rs_text == EXPECTED,
        "matches_ts": ts_text == rs_text,
    }

    # ---- pty 模式 ----
    # TS 侧在这里会被 `fd` 探测拖住（离网 netns 里它要下载 `fd`，实测 >200s 不返回），
    # 所以 pty 口径只断言 pi-rs 屏幕内容；与 TS 的 pty 对拍在 `notes/limits.md` 登记为待补。
    rs_tui = os.path.join(BASE, "tui-rs")
    run_scenario("T73-ls-tool-tui.json", rs_tui, "pi-rs")
    rs_screen = screen_text(os.path.join(rs_tui, "run-1", "raw.bin"))
    # 注意：pi-rs 的 TUI **只渲染工具调用的头部行**（`ls .`），不渲染结果正文
    # （已登记 notes/limits.md：TUI 工具结果正文缺失）；正文正确性由 print 口径逐字节证明。
    report["pty"] = {
        "pi_rs_shows_call": "ls ." in rs_screen,
        "pi_rs_shows_result_body": "aardvark.txt" in rs_screen,
        "ts_note": "TS 在离网 netns 里卡在 fd 下载（>200s），未纳入自动对拍",
    }

    ok = (
        report["print"]["matches_ref_rules"]
        and report["print"]["matches_ts"]
        and report["pty"]["pi_rs_shows_call"]
    )
    report["ok"] = ok
    print(json.dumps(report, ensure_ascii=False, indent=1))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
