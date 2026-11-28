#!/usr/bin/env python3
"""R2：网络类故障的重试行为（P8d）。

两种故障各跑一遍（都在 netns 内，`lo` 起来但没有外网）：
- `unreachable`：baseUrl 指向不可路由地址 ⇒ 连接失败/网络不可达；
- `dns`：baseUrl 主机名不存在 ⇒ DNS 解析失败。

断言（每例）：
1. 屏幕出现 `Retrying (1/3)`（说明被判为**可重试**，而不是直接报错停下）；
2. 最终出现 `Retry failed after 3 attempts`；
3. 从首次请求到最终失败的时间 ≥ 2+4+8=14s（退避确实是 2s/4s/8s）。

用法：unshare -n sh -c 'ip link set lo up; python3 tools/difftest/pty_retry_network_check.py'
"""
import fcntl
import json
import os
import pty
import re
import select
import shutil
import struct
import sys
import termios
import time

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BINARY = os.environ.get("PI_RS_BIN", os.path.join(REPO, "target", "debug", "pi-rs"))
BASE = "/tmp/p8d-retry-net"
CASES = {
    "unreachable": "http://10.255.255.1:9/v1",
    "dns": "http://no-such-host.invalid/v1",
}


def write_config(base_url: str) -> None:
    shutil.rmtree(BASE, ignore_errors=True)
    for name in ("cfg", "sess", "home", "work"):
        os.makedirs(os.path.join(BASE, name), exist_ok=True)
    cfg = os.path.join(BASE, "cfg")
    json.dump(
        {"providers": {"mock": {"baseUrl": base_url, "api": "openai-completions", "apiKey": "k",
                                "models": [{"id": "mock-model"}]}}},
        open(os.path.join(cfg, "models.json"), "w"),
    )
    json.dump({"defaultProvider": "mock", "defaultModel": "mock-model"},
              open(os.path.join(cfg, "settings.json"), "w"))
    json.dump({"mock": {"type": "api_key", "key": "k"}}, open(os.path.join(cfg, "auth.json"), "w"))


def run_case(name: str, base_url: str) -> dict:
    write_config(base_url)
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
            if select.select([fd], [], [], 0.05)[0]:
                try:
                    chunk = os.read(fd, 1 << 16)
                except OSError:
                    return
                if not chunk:
                    return
                buf.extend(chunk)

    drain(6.0)
    started = time.time()
    os.write(fd, b"hi\r")
    while time.time() - started < 40:
        drain(0.5)
        text = buf.decode("utf-8", "replace")
        if "Retry failed after" in text:
            break
    elapsed = time.time() - started
    os.kill(pid, 9)
    try:
        os.close(fd)
    except OSError:
        pass
    try:
        os.waitpid(pid, 0)
    except ChildProcessError:
        pass
    plain = re.sub(r"\x1b\[[0-9;?]*[ -/]*[@-~]", "", buf.decode("utf-8", "replace"))
    if os.environ.get("RETRY_DUMP"):
        open(os.environ["RETRY_DUMP"], "w").write(plain)
    return {
        "case": name,
        "retrying_seen": "Retrying (1/3)" in plain,
        "failed_after_3": "Retry failed after 3 attempts" in plain,
        "elapsed_s": round(elapsed, 1),
        "backoff_ok": elapsed >= 14.0,
        "error_line": next((line.strip()[:90] for line in plain.splitlines() if "Error:" in line), None),
        "error_blocks": plain.count("Error: transport error"),
        "retry_lines": [line.strip()[:48] for line in plain.splitlines() if "Retrying (" in line][-3:],
    }


def main() -> int:
    if not os.path.exists(BINARY):
        print(f"找不到二进制：{BINARY}")
        return 2
    reports = []
    for name, url in CASES.items():
        reports.append(run_case(name, url))
    for report in reports:
        print(json.dumps(report, ensure_ascii=False))
    ok = all(r["retrying_seen"] and r["failed_after_3"] and r["backoff_ok"] for r in reports)
    print(json.dumps({"all_ok": ok}, ensure_ascii=False))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
