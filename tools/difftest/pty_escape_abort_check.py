#!/usr/bin/env python3
"""Escape 中止回合（P8c/E）：kitty 开/关 × 单 ESC / CSI-u ESC，各 N=20 轮。

断言（每轮）：
1. 回合进行中（mock 卡在闸门，永不完成）发送 ESC 后 **≤200 ms** 屏幕出现中止提示；
2. 回合确实结束：`Working` 从最后一帧消失、编辑器可再次输入（下一轮能起来）。

判据来自 ref：`interactive-mode.ts:3607-3613`（escape → `session.abort()`）、
`assistant-message.ts` 中止提示文案（实测 TS 为 `Operation aborted`）。

用法（必须在本机 netns 内）：
    unshare -n sh -c 'ip link set lo up; python3 tools/difftest/pty_escape_abort_check.py'
环境变量：PI_RS_BIN（默认 target/debug/pi-rs）、ESC_N（默认 20）。
"""
import fcntl
import json
import os
import pty
import re
import select
import shutil
import struct
import subprocess
import sys
import termios
import time

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BINARY = os.environ.get("PI_RS_BIN", os.path.join(REPO, "target", "debug", "pi-rs"))
N = int(os.environ.get("ESC_N", "20"))
BASE = "/tmp/p8c-esc-abort"
CSI_U = b"\x1b[27u"
ABORTED = "Operation aborted"


def setup_mock():
    shutil.rmtree(BASE, ignore_errors=True)
    for name in ("cfg", "sess", "home", "work"):
        os.makedirs(os.path.join(BASE, name), exist_ok=True)
    # mock 的 `root` = **场景文件的祖父目录**（`main.rs:463-467`）⇒ 场景要放在
    # 仓库内的临时子目录，`fixtures/...` 才能解析到真实夹具（放 /tmp 会读成空文件、
    # 表现为"服务端 0 字节 + 客户端 Stream ended without finish_reason"，假故障）。
    scenario_dir = os.path.join(REPO, "tmp-difftest-" + os.path.basename(BASE))
    shutil.rmtree(scenario_dir, ignore_errors=True)
    os.makedirs(scenario_dir, exist_ok=True)
    scenario = os.path.join(scenario_dir, "scenario.json")
    # 闸门：每段响应发 1 个 chunk 后停住，直到显式放行 ⇒ 回合永远不会自然结束。
    json.dump(
        {
            "id": "ESC",
            "provider": "openai-completions",
            "model": "mock-model",
            "responses": [{"file": "fixtures/sse/openai/text-basic.sse", "gateAfterChunks": 1}],
            "files": {},
            "prompt": "hi",
            "description": "slow turn for escape",
            "tui": {"cols": 100, "rows": 30, "steps": []},
        },
        open(scenario, "w"),
    )
    log = os.path.join(BASE, "mock.out")
    handle = open(log, "w+")
    mock = subprocess.Popen(
        [
            os.path.join(REPO, "target", "debug", "mock-provider"),
            scenario,
            "--port",
            "0",
            "--request-log",
            os.path.join(BASE, "requests.jsonl"),
        ],
        stdout=handle,
        stderr=subprocess.STDOUT,
    )
    port = None
    for _ in range(400):
        time.sleep(0.05)
        text = open(log).read()
        tail = text.strip().splitlines()[-1] if text.strip() else ""
        match = re.search(r":(\d+)\s*$", tail)
        if match:
            port = int(match.group(1))
            break
    if port is None:
        mock.kill()
        raise SystemExit("mock-provider 未在 20s 内启动")
    cfg = os.path.join(BASE, "cfg")
    json.dump(
        {
            "providers": {
                "mock": {
                    "baseUrl": f"http://127.0.0.1:{port}/v1",
                    "api": "openai-completions",
                    "apiKey": "k",
                    "models": [{"id": "mock-model"}],
                }
            }
        },
        open(os.path.join(cfg, "models.json"), "w"),
    )
    json.dump({"defaultProvider": "mock", "defaultModel": "mock-model"},
              open(os.path.join(cfg, "settings.json"), "w"))
    json.dump({"mock": {"type": "api_key", "key": "k"}},
              open(os.path.join(cfg, "auth.json"), "w"))
    return mock, port


class Screen:
    """把 pty 输出累积成"当前可见文本"（粗粒度：只取可打印字符行）。"""

    def __init__(self):
        self.buf = b""

    def feed(self, chunk):
        self.buf += chunk

    def text(self):
        plain = re.sub(rb"\x1b\[[0-9;?]*[ -/]*[@-~]", b"", self.buf)
        plain = re.sub(rb"\x1b\][^\x07]*\x07", b"", plain)
        return plain.decode("utf-8", "replace")

    def fresh(self):
        self.buf = b""


def run_config(kitty: bool, esc: bytes, label: str):
    mock, _port = setup_mock()
    env = dict(
        os.environ,
        HOME=os.path.join(BASE, "home"),
        PI_CODING_AGENT_DIR=os.path.join(BASE, "cfg"),
        PI_CODING_AGENT_SESSION_DIR=os.path.join(BASE, "sess"),
    )
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(os.path.join(BASE, "work"))
        os.execvpe(BINARY, [BINARY], env)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
    screen = Screen()

    def drain(sec):
        end = time.time() + sec
        while time.time() < end:
            if select.select([fd], [], [], 0.02)[0]:
                try:
                    chunk = os.read(fd, 1 << 16)
                except OSError:
                    return False
                if not chunk:
                    return False
                screen.feed(chunk)
        return True

    if kitty:
        # 模拟"终端已激活 kitty 键盘协议"：让应用进入 kitty 模式（`\x1b[>1u`）。
        os.write(fd, b"\x1b[?1u")  # 应答 kitty 查询（`CSI ? u`）
    drain(6.0)

    results = []
    for index in range(N):
        screen.fresh()
        os.write(fd, b"hi\r")
        # 等回合真的起来（`Working` 出现）
        started = time.time()
        up = False
        while time.time() - started < 8.0:
            drain(0.05)
            if "Working" in screen.text():
                up = True
                break
        if not up:
            results.append({"turn": index + 1, "ok": False, "why": "回合未启动(无 Working)"})
            break
        sent_at = time.time()
        os.write(fd, esc)
        # 等中止提示，最多 1s
        seen_at = None
        while time.time() - sent_at < 1.0:
            drain(0.02)
            if ABORTED in screen.text():
                seen_at = time.time()
                break
        latency = None if seen_at is None else (seen_at - sent_at) * 1000.0
        drain(0.4)
        tail = screen.text()
        ended = "Working" not in tail.splitlines()[-1] if tail.splitlines() else False
        results.append(
            {
                "turn": index + 1,
                "ok": seen_at is not None and latency <= 200.0 and ended,
                "latency_ms": None if latency is None else round(latency, 1),
                "aborted": ABORTED in tail,
                "ended": ended,
            }
        )
    os.kill(pid, 9)
    try:
        os.close(fd)
    except OSError:
        pass
    mock.kill()
    ok = sum(1 for item in results if item["ok"])
    return {"config": label, "ok": ok, "total": N, "failures": [r for r in results if not r["ok"]][:3]}


def main():
    if not os.path.exists(BINARY):
        raise SystemExit(f"找不到二进制：{BINARY}")
    reports = []
    for kitty in (False, True):
        for esc, name in ((b"\x1b", "esc"), (CSI_U, "csi-u")):
            label = f"{'kitty' if kitty else 'plain'}/{name}"
            report = run_config(kitty, esc, label)
            reports.append(report)
            print(json.dumps(report, ensure_ascii=False))
    bad = [report for report in reports if report["ok"] != report["total"]]
    print(json.dumps({"configs": len(reports), "all_ok": not bad}, ensure_ascii=False))
    return 0 if not bad else 1


if __name__ == "__main__":
    sys.exit(main())
