#!/usr/bin/env python3
"""切换到本版本不支持的 api（google-generative-ai；openai-responses 自 P8b 起已支持）时，状态行必须给出原因（不能静默）。"""
import fcntl, json, os, pty, re, select, shutil, struct, termios, time, sys

REPO = str(__import__("pathlib").Path(__file__).resolve().parents[2])
# 允许用 `PI_RS_BIN` 指定被测二进制（P8a 用它跑 musl 静态构建）。
BINARY = os.environ.get("PI_RS_BIN", f"{REPO}/target/debug/pi-rs")
BASE = "/tmp/p79-unsup"
CFG, SESS, HOME, WORK = (os.path.join(BASE, n) for n in ("cfg", "sess", "home", "work"))


def main() -> int:
    shutil.rmtree(BASE, ignore_errors=True)
    for path in (CFG, SESS, HOME, WORK):
        os.makedirs(path, exist_ok=True)
    with open(os.path.join(CFG, "models.json"), "w") as handle:
        json.dump({"providers": {"alpha": {"baseUrl": "http://127.0.0.1:9/v1",
                                           "api": "openai-completions", "apiKey": "k",
                                           "models": [{"id": "mock-model"},
                                                      {"id": "luna", "api": "google-generative-ai"}]}}}, handle)
    with open(os.path.join(CFG, "settings.json"), "w") as handle:
        json.dump({"defaultProvider": "alpha", "defaultModel": "mock-model",
                   "enabledModels": ["alpha/mock-model", "alpha/luna"]}, handle)
    with open(os.path.join(CFG, "auth.json"), "w") as handle:
        json.dump({"alpha": {"type": "api_key", "key": "k"}}, handle)
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
    os.write(fd, b"\x10")  # Ctrl+P → luna（不支持的 api）
    drain(1.5)
    text = re.sub(rb"\x1b\[[0-9;?]*[ -/]*[@-~]", b"", buf).decode("utf-8", "replace")
    os.write(fd, b"\x15\x04")
    drain(0.6)
    try:
        os.close(fd)
    except OSError:
        pass
    os.waitpid(pid, 0)
    ok = "无法切换模型" in text and "google-generative-ai" in text
    print(json.dumps({"error_status_shown": ok}))
    return 0 if ok else 1


sys.exit(main())
