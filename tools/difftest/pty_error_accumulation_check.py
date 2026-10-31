#!/usr/bin/env python3
"""P7-8c/E：provider 每次失败时，**每轮只应显示一条错误**（不得累计拼接）。

真机现象（openrouter 401）：第 2 轮屏幕出现 2 份 `Error: …`、第 3 轮 3 份、第 4 轮 4 份。
根因：每轮新建的 `AgentLoop` 默认 `announce_from = 0`，会把**整段历史**重新宣告一遍，
TUI 对每条 assistant 的 `message_end` 都渲染一次错误提示 ⇒ 第 k 轮 k 份。

用法：unshare -n sh -c 'ip link set lo up; python3 tools/difftest/pty_error_accumulation_check.py'
"""
import fcntl, json, os, pty, re, select, shutil, struct, subprocess, sys, tempfile, termios, time

REPO = str(__import__("pathlib").Path(__file__).resolve().parents[2])
# 允许用 `PI_RS_BIN` 指定被测二进制（P8a 用它跑 musl 静态构建）。
BINARY = os.environ.get("PI_RS_BIN", f"{REPO}/target/debug/pi-rs")
BASE = "/tmp/p78c-error-check"
CFG, SESS, HOME, WORK = (os.path.join(BASE, name) for name in ("cfg", "sess", "home", "work"))
TURNS = 4


def prepare_scenario() -> str:
    body = os.path.join(BASE, "error-body.txt")
    with open(body, "w") as handle:
        handle.write('{"error":{"message":"No cookie auth credentials found"}}')
    scenario = {
        "id": "P78C-401",
        "provider": "openai-completions",
        "model": "mock-model",
        "responses": [{"file": body, "status": 401} for _ in range(TURNS + 4)],
        "files": {},
        "prompt": "hi",
        "description": "每次请求都 401",
        "tui": {"cols": 100, "rows": 30, "steps": []},
    }
    path = os.path.join(BASE, "scenario.json")
    with open(path, "w") as handle:
        json.dump(scenario, handle)
    return path


def start_mock(scenario: str) -> tuple:
    log = os.path.join(BASE, "mock.out")
    handle = open(log, "w+")
    process = subprocess.Popen(
        [f"{REPO}/target/debug/mock-provider", scenario, "--port", "0",
         "--request-log", os.path.join(BASE, "requests.jsonl")],
        stdout=handle, stderr=subprocess.STDOUT,
    )
    for _ in range(200):
        time.sleep(0.05)
        with open(log) as reader:
            text = reader.read()
        match = re.search(r":(\d+)\s*$", text.strip().splitlines()[-1] if text.strip() else "")
        if match:
            return process, int(match.group(1))
    process.kill()
    raise SystemExit("mock-provider 没起来")


def prepare_config(port: int) -> None:
    for path in (CFG, SESS, HOME, WORK):
        shutil.rmtree(path, ignore_errors=True)
        os.makedirs(path, exist_ok=True)
    with open(os.path.join(CFG, "models.json"), "w") as handle:
        json.dump({"providers": {"mock": {"baseUrl": f"http://127.0.0.1:{port}/v1",
                                          "api": "openai-completions", "apiKey": "k",
                                          "models": [{"id": "mock-model"}]}}}, handle)
    for name, payload in (("settings.json", {"defaultProvider": "mock", "defaultModel": "mock-model"}),
                          ("auth.json", {"mock": {"type": "api_key", "key": "k"}})):
        with open(os.path.join(CFG, name), "w") as handle:
            json.dump(payload, handle)


def main() -> int:
    shutil.rmtree(BASE, ignore_errors=True)
    os.makedirs(BASE, exist_ok=True)
    scenario = prepare_scenario()
    mock, port = start_mock(scenario)
    try:
        prepare_config(port)
        env = dict(os.environ, HOME=HOME, PI_CODING_AGENT_DIR=CFG, PI_CODING_AGENT_SESSION_DIR=SESS)
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(WORK)
            os.execvpe(BINARY, ["pi-rs"], env)
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
        screen = b""

        def drain(seconds: float) -> None:
            nonlocal screen
            end = time.time() + seconds
            while time.time() < end:
                if select.select([fd], [], [], 0.03)[0]:
                    try:
                        chunk = os.read(fd, 1 << 16)
                    except OSError:
                        return
                    if not chunk:
                        return
                    screen += chunk
                    end = time.time() + 0.15

        drain(6.0)
        counts = []
        for index in range(TURNS):
            os.write(fd, chr(ord("a") + index).encode())
            os.write(fd, b"\r")
            drain(3.0)
            rendered = re.sub(rb"\x1b\[[0-9;?]*[ -/]*[@-~]", b"", screen).decode("utf-8", "replace")
            counts.append(rendered.count("Error:"))
            os.write(fd, b"\x15")
            drain(0.2)
        os.write(fd, b"\x04")
        drain(0.8)
        try:
            os.close(fd)
        except OSError:
            pass
        os.waitpid(pid, 0)
        expected = list(range(1, TURNS + 1))
        print(json.dumps({"cumulative_error_counts": counts, "expected": expected}))
        return 0 if counts == expected else 1
    finally:
        mock.kill()


if __name__ == "__main__":
    sys.exit(main())
