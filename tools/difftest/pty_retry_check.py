#!/usr/bin/env python3
"""TUI 的会话级 auto-retry（对齐上游 `agent-session.ts:978-990/1503` 的行为）。

mock 的流在第 2 块后断开（可重试错误），断言：
- 状态行出现 `Retrying (1/3) in 2s... (escape to cancel)`（ref `status-indicator.ts:54-63` 的文案）；
- 三次都失败后出现 `Retry failed after 3 attempts: …`（`interactive-mode.ts:3631`）；
- mock 至少收到 3 个请求（说明整轮被重跑，而不是只报一次错）。

注：当前 mock 没有"只失败一次随后成功"的能力（`disconnectAfterChunks` 对该步持续生效），
所以这里断言的是**失败重试路径**；成功路径由 `T01` 等正常场景覆盖。
"""
import fcntl, json, os, pty, re, select, shutil, struct, subprocess, sys, termios, time

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BINARY = os.environ.get("PI_RS_BIN", f"{REPO}/target/debug/pi-rs")
BASE = "/tmp/p8b-retry"
CFG, SESS, HOME, WORK = (os.path.join(BASE, n) for n in ("cfg", "sess", "home", "work"))


def start_mock():
    scenario = os.path.join(BASE, "scenario.json")
    json.dump({"id": "RETRY", "provider": "openai-completions", "model": "mock-model",
               "responses": [{"file": "fixtures/sse/openai/text-basic.sse", "disconnectAfterChunks": 2},
                              {"file": "fixtures/sse/openai/text-basic.sse"}],
               "files": {}, "prompt": "hi", "description": "retry",
               "tui": {"cols": 100, "rows": 30, "steps": []}}, open(scenario, "w"))
    log = os.path.join(BASE, "mock.out")
    handle = open(log, "w+")
    process = subprocess.Popen([f"{REPO}/target/debug/mock-provider", scenario, "--port", "0",
                                "--request-log", os.path.join(BASE, "requests.jsonl")],
                               stdout=handle, stderr=subprocess.STDOUT)
    for _ in range(200):
        time.sleep(0.05)
        text = open(log).read()
        m = re.search(r":(\d+)\s*$", text.strip().splitlines()[-1] if text.strip() else "")
        if m:
            return process, int(m.group(1))
    process.kill()
    raise SystemExit("mock 未启动")


def main() -> int:
    shutil.rmtree(BASE, ignore_errors=True)
    os.makedirs(BASE, exist_ok=True)
    mock, port = start_mock()
    try:
        for path in (CFG, SESS, HOME, WORK):
            os.makedirs(path, exist_ok=True)
        json.dump({"providers": {"mock": {"baseUrl": f"http://127.0.0.1:{port}/v1",
                                          "api": "openai-completions", "apiKey": "k",
                                          "models": [{"id": "mock-model"}]}}},
                  open(os.path.join(CFG, "models.json"), "w"))
        json.dump({"defaultProvider": "mock", "defaultModel": "mock-model"},
                  open(os.path.join(CFG, "settings.json"), "w"))
        json.dump({"mock": {"type": "api_key", "key": "k"}}, open(os.path.join(CFG, "auth.json"), "w"))
        env = dict(os.environ, HOME=HOME, PI_CODING_AGENT_DIR=CFG, PI_CODING_AGENT_SESSION_DIR=SESS)
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(WORK)
            os.execvpe(BINARY, [BINARY], env)
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
        buf = b""

        def drain(sec: float) -> None:
            nonlocal buf
            end = time.time() + sec
            while time.time() < end:
                if select.select([fd], [], [], 0.03)[0]:
                    try:
                        chunk = os.read(fd, 1 << 16)
                    except OSError:
                        return
                    if not chunk:
                        return
                    buf += chunk
                    end = time.time() + 0.2

        drain(6.0)
        os.write(fd, b"hello\r")
        drain(20.0)          # 重试退避 2s + 第二轮
        text = re.sub(rb"\x1b\[[0-9;?]*[ -/]*[@-~]", b"", buf).decode("utf-8", "replace")
        os.write(fd, b"\x15\x04")
        drain(0.6)
        try:
            os.close(fd)
        except OSError:
            pass
        os.waitpid(pid, 0)
        requests = 0
        log = os.path.join(BASE, "requests.jsonl")
        if os.path.exists(log):
            requests = sum(1 for _ in open(log))
        lines = [line.strip() for line in text.splitlines() if line.strip()]
        result = {
            "tail": lines[-6:],
            "retry_status": "Retrying (1/3)" in text,
            "retry_failed_notice": "Retry failed after 3 attempts" in text,
            "request_count": requests,
        }
        result["ok"] = (
            result["retry_status"] and result["retry_failed_notice"] and requests >= 3
        )
        print(json.dumps(result, ensure_ascii=False))
        return 0 if result["ok"] else 1
    finally:
        mock.kill()


sys.exit(main())
