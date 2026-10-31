#!/usr/bin/env python3
"""A：Esc 中止的延迟测量（P8d）。

状态：
- `stream`：SSE 卡在闸门（回合一直在流式输出）；
- `bash`：模型调用 `bash` 执行 `sleep 30`（工具执行中）。
每个状态注入 ESC，测两段延迟：
  1) 注入 → 屏幕出现中止提示（`Operation aborted`）；
  2) 注入 → 回合真正结束（末行不再有 `Working`，可再次输入）。

用法（netns 内）：
    unshare -n sh -c 'ip link set lo up; python3 tools/difftest/pty_esc_latency_check.py'
环境变量：PI_RS_BIN、ESC_N（默认 10）、ESC_LAT_ONLY（stream|bash）。
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
N = int(os.environ.get("ESC_N", "10"))
BASE = "/tmp/p8d-esc-latency"
ONLY = os.environ.get("ESC_LAT_ONLY", "")
# 中止文案按状态不同：
# - 回合流式中被中止 ⇒ `Operation aborted`（ref assistant 侧的 abort 文案）；
# - 工具执行中被中止 ⇒ `This operation was aborted`（工具自己的 abort 文案）。
ABORTED_BY_STATE = {
    "stream": "Operation aborted",
    "bash": "This operation was aborted",
}


def write_scenario(state: str) -> str:
    shutil.rmtree(BASE, ignore_errors=True)
    for name in ("cfg", "sess", "home", "work"):
        os.makedirs(os.path.join(BASE, name), exist_ok=True)
    scenario = os.path.join(REPO, "tmp-difftest-esc-latency", "scenario.json")
    shutil.rmtree(os.path.dirname(scenario), ignore_errors=True)
    os.makedirs(os.path.dirname(scenario), exist_ok=True)
    if state == "stream":
        responses = [{"file": "fixtures/sse/openai/text-basic.sse", "gateAfterChunks": 1}]
    else:
        # 每轮消耗两段响应（toolCall + 正文）⇒ 准备 N 轮的份数。
        responses = []
        for _ in range(N + 2):
            responses.append({"file": "fixtures/sse/openai/tool-bash-sleep.sse"})
            responses.append({"file": "fixtures/sse/openai/text-basic.sse"})
    with open(scenario, "w") as handle:
        json.dump({"id": "LAT", "provider": "openai-completions", "model": "mock-model",
                   "responses": responses, "files": {}, "prompt": "hi",
                   "description": "esc latency", "tui": {"cols": 100, "rows": 30, "steps": []}},
                  handle)
    log = os.path.join(BASE, "mock.out")
    handle = open(log, "w+")
    mock = subprocess.Popen(
        [os.path.join(REPO, "target", "debug", "mock-provider"), scenario, "--port", "0",
         "--request-log", os.path.join(BASE, "requests.jsonl")],
        stdout=handle, stderr=subprocess.STDOUT)
    port = None
    for _ in range(400):
        time.sleep(0.05)
        lines = open(log).read().strip().splitlines()
        match = re.search(r":(\d+)\s*$", lines[-1]) if lines else None
        if match:
            port = int(match.group(1))
            break
    if port is None:
        mock.kill()
        raise SystemExit("mock 未启动")
    cfg = os.path.join(BASE, "cfg")
    json.dump({"providers": {"mock": {"baseUrl": f"http://127.0.0.1:{port}/v1",
                                     "api": "openai-completions", "apiKey": "k",
                                     "models": [{"id": "mock-model"}]}}},
              open(os.path.join(cfg, "models.json"), "w"))
    json.dump({"defaultProvider": "mock", "defaultModel": "mock-model"},
              open(os.path.join(cfg, "settings.json"), "w"))
    json.dump({"mock": {"type": "api_key", "key": "k"}}, open(os.path.join(cfg, "auth.json"), "w"))
    return mock, port


def measure(state: str) -> dict:
    samples = [one_sample(state) for _ in range(N)]
    ok = [s for s in samples if s.get("ok")]
    aborts = sorted(s["abort_ms"] for s in ok if s["abort_ms"] is not None)
    ends = sorted(s["end_ms"] for s in ok if s["end_ms"] is not None)

    def percentile(values, ratio):
        if not values:
            return None
        index = min(len(values) - 1, int(round(ratio * (len(values) - 1))))
        return values[index]

    return {
        "state": state,
        "n": len(samples),
        "ok": len(ok),
        "abort_p50": percentile(aborts, 0.5),
        "abort_p99": percentile(aborts, 0.99),
        "end_p50": percentile(ends, 0.5),
        "end_p99": percentile(ends, 0.99),
    }


def one_sample(state: str) -> dict:
    mock, _port = write_scenario(state)
    env = dict(os.environ, HOME=os.path.join(BASE, "home"),
               PI_CODING_AGENT_DIR=os.path.join(BASE, "cfg"),
               PI_CODING_AGENT_SESSION_DIR=os.path.join(BASE, "sess"))
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(os.path.join(BASE, "work"))
        os.execvpe(BINARY, [BINARY], env)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
    buf = bytearray()

    def drain(sec: float) -> None:
        end = time.time() + sec
        while time.time() < end:
            if select.select([fd], [], [], 0.02)[0]:
                try:
                    chunk = os.read(fd, 1 << 16)
                except OSError:
                    return
                if not chunk:
                    return
                buf.extend(chunk)

    drain(6.0)
    if True:
        buf.clear()
        os.write(fd, b"hi\r")
        started = time.time()
        up = False
        while time.time() - started < 10:
            drain(0.05)
            if "Working" in buf.decode("utf-8", "replace"):
                up = True
                break
        if not up:
            mock.kill()
            return {"ok": False, "why": "回合未启动"}
        injected = time.time()
        os.write(fd, b"\x1b")
        aborted_at = None
        ended_at = None
        while time.time() - injected < 5:
            drain(0.02)
            text = buf.decode("utf-8", "replace")
            if aborted_at is None and ABORTED_BY_STATE[state] in text:
                aborted_at = time.time()
            if aborted_at is not None:
                lines = [line for line in text.splitlines() if line.strip()]
                if lines and "Working" not in lines[-1]:
                    ended_at = time.time()
                    break
        if os.environ.get("ESC_LAT_DUMP"):
            open(os.environ["ESC_LAT_DUMP"], "w").write(buf.decode("utf-8", "replace"))
        drain(0.4)
    os.kill(pid, 9)
    try:
        os.close(fd)
    except OSError:
        pass
    try:
        os.waitpid(pid, 0)
    except ChildProcessError:
        pass
    mock.kill()
    try:
        mock.wait(timeout=5)
    except Exception:
        pass
    return {
        "ok": aborted_at is not None and ended_at is not None,
        "abort_ms": None if aborted_at is None else round((aborted_at - injected) * 1000, 1),
        "end_ms": None if ended_at is None else round((ended_at - injected) * 1000, 1),
    }


def main() -> int:
    if not os.path.exists(BINARY):
        print(f"找不到二进制：{BINARY}")
        return 2
    states = ["stream", "bash"] if not ONLY else [ONLY]
    reports = [measure(state) for state in states]
    for report in reports:
        print(json.dumps(report, ensure_ascii=False))
    ok = all(r["ok"] == r["n"] and r["abort_p99"] is not None and r["abort_p99"] <= 200 for r in reports)
    print(json.dumps({"all_ok": ok}, ensure_ascii=False))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
