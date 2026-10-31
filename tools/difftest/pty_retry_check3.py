#!/usr/bin/env python3
"""会话级 auto-retry 三条路径（P8c/R）。

1. `cancel`：退避期按 Escape ⇒ 不再发第二次请求，回合结束；
2. `success`：首次连接中断、第二次成功 ⇒ 最终正文出现、请求数 2；
3. `exhausted`：连续中断 ⇒ `Retry failed after 3 attempts`、请求数 4。

判据：`status-indicator.ts:54-63`（`Retrying (n/3) in 2s... (escape to cancel)`）、
`interactive-mode.ts:3631`（`Retry failed after N attempts: …`）。

用法：unshare -n sh -c 'ip link set lo up; python3 tools/difftest/pty_retry_check3.py'
环境变量：PI_RS_BIN、RETRY_ONLY（cancel|success|exhausted，默认全跑）。
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
BASE = "/tmp/p8c-retry"
ONLY = os.environ.get("RETRY_ONLY", "")
FIXTURE = "fixtures/sse/openai/text-basic.sse"


_MOCK = []


def stop_mock():
    """杀掉本次起的 mock：探针异常退出时也要收干净（precommit 会查残留进程）。"""
    while _MOCK:
        proc = _MOCK.pop()
        try:
            proc.kill()
            proc.wait(timeout=5)
        except Exception:
            pass


def setup(response_specs):
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
    json.dump(
        {
            "id": "RT",
            "provider": "openai-completions",
            "model": "mock-model",
            "responses": response_specs,
            "files": {},
            "prompt": "hi",
            "description": "retry paths",
            "tui": {"cols": 100, "rows": 30, "steps": []},
        },
        open(scenario, "w"),
    )
    log = os.path.join(BASE, "mock.out")
    handle = open(log, "w+")
    mock = subprocess.Popen(
        [os.path.join(REPO, "target", "debug", "mock-provider"), scenario,
         "--port", "0", "--request-log", os.path.join(BASE, "requests.jsonl")],
        stdout=handle, stderr=subprocess.STDOUT,
    )
    port = None
    for _ in range(400):
        time.sleep(0.05)
        lines = open(log).read().strip().splitlines()
        match = re.search(r":(\d+)\s*$", lines[-1]) if lines else None
        if match:
            port = int(match.group(1))
            break
    _MOCK.append(mock)
    if port is None:
        stop_mock()
        raise SystemExit("mock 未启动")
    cfg = os.path.join(BASE, "cfg")
    json.dump({"providers": {"mock": {"baseUrl": f"http://127.0.0.1:{port}/v1",
                                     "api": "openai-completions", "apiKey": "k",
                                     "models": [{"id": "mock-model"}]}}},
              open(os.path.join(cfg, "models.json"), "w"))
    json.dump({"defaultProvider": "mock", "defaultModel": "mock-model"},
              open(os.path.join(cfg, "settings.json"), "w"))
    json.dump({"mock": {"type": "api_key", "key": "k"}}, open(os.path.join(cfg, "auth.json"), "w"))


def run(path, response_specs, action):
    setup(response_specs)
    env = dict(os.environ, HOME=os.path.join(BASE, "home"),
               PI_CODING_AGENT_DIR=os.path.join(BASE, "cfg"),
               PI_CODING_AGENT_SESSION_DIR=os.path.join(BASE, "sess"))
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(os.path.join(BASE, "work"))
        os.execvpe(BINARY, [BINARY], env)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
    buf = bytearray()

    def drain(sec):
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
    buf.clear()
    os.write(fd, b"hi\r")
    if action == "cancel":
        deadline = time.time() + 6.0
        while time.time() < deadline:
            drain(0.05)
            if "Retrying (" in buf.decode("utf-8", "replace"):
                break
        os.write(fd, b"\x1b")
    drain(20.0)
    os.kill(pid, 9)
    try:
        os.close(fd)
    except OSError:
        pass
    try:
        os.waitpid(pid, 0)
    except ChildProcessError:
        pass
    stop_mock()
    text = re.sub(rb"\x1b\[[0-9;?]*[ -/]*[@-~]", b"", bytes(buf)).decode("utf-8", "replace")
    log_path = os.path.join(BASE, "requests.jsonl")
    # 日志里每条请求一行 + 每段响应完成再加一行 ⇒ 只数带 `served` 的请求行。
    requests = 0
    if os.path.exists(log_path):
        for line in open(log_path):
            try:
                entry = json.loads(line)
                # 默认日志一请求一行（含 `body`）；`MOCK_VERBOSE=1` 时另有 `served`。
                if "body" in entry or "served" in entry:
                    requests += 1
            except json.JSONDecodeError:
                continue
    if os.environ.get("RETRY_DUMP"):
        open(os.environ["RETRY_DUMP"], "w").write(text)
    # 诊断用：把 `Retry failed after N attempts: <原文>` 一并带回
    err = [line.strip() for line in text.splitlines() if "Retry failed" in line]
    # 只看**最后一帧**（累积文本里含历史帧，断言会假阳性）：取最后 12 行非空行。
    tail_lines = [line.strip() for line in text.splitlines() if line.strip()][-12:]
    tail_text = "\n".join(tail_lines)
    return {"path": path, "requests": requests,
            "retrying": "Retrying (" in tail_text,
            "failed": "Retry failed after" in tail_text,
            "aborted": "Operation aborted" in tail_text,
            "answer": "Mock assistant reply" in tail_text or "Hello" in tail_text,
            "retry_error": err[-1][:120] if err else None,
            "tail": [line.strip() for line in text.splitlines() if line.strip()][-3:]}


def main():
    cases = {
        "cancel": ([{"file": FIXTURE, "chunkByEvent": True, "disconnectAfterChunks": 2},
                    {"file": FIXTURE}], "cancel",
                   {"requests": 1, "failed": False}),
        "success": ([{"file": FIXTURE, "chunkByEvent": True, "disconnectAfterChunks": 2},
                     {"file": FIXTURE}, {"file": FIXTURE}, {"file": FIXTURE}], "none",
                    {"requests": 2, "answer": True, "failed": False}),
        "exhausted": ([{"file": FIXTURE, "chunkByEvent": True, "disconnectAfterChunks": 2}] * 4, "none",
                      {"requests": 4, "failed": True}),
    }
    reports = []
    for name, (specs, action, expect) in cases.items():
        if ONLY and ONLY != name:
            continue
        got = run(name, specs, action)
        ok = all(got.get(key) == value for key, value in expect.items())
        got["ok"] = ok
        got["expect"] = expect
        reports.append(got)
        print(json.dumps(got, ensure_ascii=False))
    print(json.dumps({"all_ok": all(report["ok"] for report in reports)}, ensure_ascii=False))
    return 0 if all(report["ok"] for report in reports) else 1


if __name__ == "__main__":
    sys.exit(main())
