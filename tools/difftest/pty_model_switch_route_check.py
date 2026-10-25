#!/usr/bin/env python3
"""P7-9：跨 provider 切换后，请求必须打到**被选中 provider** 的 baseUrl。

两个本地 mock（各自独立端口与请求日志）：
- provider `alpha`：mock-model
- provider `beta`：model-b
`/model` 选 beta 的 model-b → 发一句话 → 断言 beta 的日志有 1 条、alpha 仍是 0。
"""
import fcntl, json, os, pty, re, select, shutil, struct, subprocess, termios, time, sys

REPO = str(__import__("pathlib").Path(__file__).resolve().parents[2])
# 允许用 `PI_RS_BIN` 指定被测二进制（P8a 用它跑 musl 静态构建）。
BINARY = os.environ.get("PI_RS_BIN", f"{REPO}/target/debug/pi-rs")
BASE = "/tmp/p79-route"
CFG, SESS, HOME, WORK = (os.path.join(BASE, n) for n in ("cfg", "sess", "home", "work"))


def start_mock(name: str) -> tuple:
    log = os.path.join(BASE, f"{name}.out")
    request_log = os.path.join(BASE, f"{name}.jsonl")
    handle = open(log, "w+")
    process = subprocess.Popen(
        [f"{REPO}/target/debug/mock-provider", f"{REPO}/scenarios-tui/T01.json",
         "--port", "0", "--request-log", request_log],
        stdout=handle, stderr=subprocess.STDOUT)
    port = None
    for _ in range(200):
        time.sleep(0.05)
        text = open(log).read()
        match = re.search(r":(\d+)\s*$", text.strip().splitlines()[-1] if text.strip() else "")
        if match:
            port = int(match.group(1))
            break
    assert port, f"{name} mock 未启动"
    return process, port, request_log


def main() -> int:
    shutil.rmtree(BASE, ignore_errors=True)
    os.makedirs(BASE, exist_ok=True)
    alpha, alpha_port, alpha_log = start_mock("alpha")
    beta, beta_port, beta_log = start_mock("beta")
    try:
        for path in (CFG, SESS, HOME, WORK):
            os.makedirs(path, exist_ok=True)
        with open(os.path.join(CFG, "models.json"), "w") as handle:
            json.dump({"providers": {
                "alpha": {"baseUrl": f"http://127.0.0.1:{alpha_port}/v1", "api": "openai-completions",
                          "apiKey": "k", "models": [{"id": "mock-model"}]},
                "beta": {"baseUrl": f"http://127.0.0.1:{beta_port}/v1", "api": "openai-completions",
                         "apiKey": "k", "models": [{"id": "model-b"}]},
            }}, handle)
        with open(os.path.join(CFG, "settings.json"), "w") as handle:
            json.dump({"defaultProvider": "alpha", "defaultModel": "mock-model",
                       "enabledModels": ["alpha/mock-model", "beta/model-b"]}, handle)
        with open(os.path.join(CFG, "auth.json"), "w") as handle:
            json.dump({"alpha": {"type": "api_key", "key": "ka"},
                       "beta": {"type": "api_key", "key": "kb"}}, handle)
        env = dict(os.environ, HOME=HOME, PI_CODING_AGENT_DIR=CFG, PI_CODING_AGENT_SESSION_DIR=SESS)
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(WORK)
            os.execvpe(BINARY, ["pi-rs"], env)
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
                    end = time.time() + 0.15

        drain(6.0)
        # Ctrl+P 切到 beta/model-b
        os.write(fd, b"\x10")
        drain(1.2)
        text = re.sub(rb"\x1b\[[0-9;?]*[ -/]*[@-~]", b"", buf).decode("utf-8", "replace")
        switched = "Switched to model-b" in text
        # 发一句话，看请求落到谁身上
        os.write(fd, b"hello\r")
        drain(4.0)
        os.write(fd, b"\x15\x04")
        drain(0.8)
        try:
            os.close(fd)
        except OSError:
            pass
        os.waitpid(pid, 0)
        counts = {
            "alpha": sum(1 for _ in open(alpha_log)) if os.path.exists(alpha_log) else 0,
            "beta": sum(1 for _ in open(beta_log)) if os.path.exists(beta_log) else 0,
        }
        print(json.dumps({"switched": switched, "requests": counts}))
        return 0 if switched and counts["beta"] == 1 and counts["alpha"] == 0 else 1
    finally:
        alpha.kill()
        beta.kill()


sys.exit(main())
