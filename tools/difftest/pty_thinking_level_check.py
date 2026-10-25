#!/usr/bin/env python3
"""P8b/B：思考档位切换（`shift+tab`）、保存（`ctrl+s`）与请求参数下发。

三个断言：
1. 推理模型：`shift+tab` 让状态行出现 `Thinking level: <level>`，且下一轮请求体带
   `reasoning_effort`（openai-completions）/`reasoning.effort`（openai-responses）；
2. 非推理模型：`shift+tab` 显示 `Current model does not support thinking`；
3. `ctrl+s`：settings.json 写入 `defaultThinkingLevel`，会话文件出现 `thinking_level_change`。
"""
import fcntl, json, os, pty, re, select, shutil, struct, subprocess, sys, termios, time

REPO = str(__import__("pathlib").Path(__file__).resolve().parents[2])
BINARY = os.environ.get("PI_RS_BIN", f"{REPO}/target/debug/pi-rs")
BASE = "/tmp/p8b-thinking"
CFG, SESS, HOME, WORK = (os.path.join(BASE, name) for name in ("cfg", "sess", "home", "work"))


def start_mock() -> tuple:
    log = os.path.join(BASE, "mock.out")
    handle = open(log, "w+")
    process = subprocess.Popen(
        [f"{REPO}/target/debug/mock-provider", f"{REPO}/scenarios-tui/T01.json",
         "--port", "0", "--request-log", os.path.join(BASE, "requests.jsonl")],
        stdout=handle, stderr=subprocess.STDOUT)
    for _ in range(200):
        time.sleep(0.05)
        text = open(log).read()
        match = re.search(r":(\d+)\s*$", text.strip().splitlines()[-1] if text.strip() else "")
        if match:
            return process, int(match.group(1))
    process.kill()
    raise SystemExit("mock 未启动")


def prepare(port: int, reasoning: bool) -> None:
    for path in (CFG, SESS, HOME, WORK):
        shutil.rmtree(path, ignore_errors=True)
        os.makedirs(path, exist_ok=True)
    with open(os.path.join(CFG, "models.json"), "w") as handle:
        json.dump({"providers": {"mock": {"baseUrl": f"http://127.0.0.1:{port}/v1",
                                          "api": "openai-completions", "apiKey": "k",
                                          "models": [{"id": "mock-model", "reasoning": reasoning}]}}}, handle)
    with open(os.path.join(CFG, "settings.json"), "w") as handle:
        json.dump({"defaultProvider": "mock", "defaultModel": "mock-model", "theme": "dark"}, handle)
    with open(os.path.join(CFG, "auth.json"), "w") as handle:
        json.dump({"mock": {"type": "api_key", "key": "k"}}, handle)


def session(binary: str, keys: list) -> bytes:
    env = dict(os.environ, HOME=HOME, PI_CODING_AGENT_DIR=CFG, PI_CODING_AGENT_SESSION_DIR=SESS)
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(WORK)
        os.execvpe(binary, [binary], env)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
    buf = b""

    def drain(seconds: float) -> None:
        nonlocal buf
        end = time.time() + seconds
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
    for key in keys:
        os.write(fd, key)
        drain(1.2)
    os.write(fd, b"\x15\x04")
    drain(0.8)
    try:
        os.close(fd)
    except OSError:
        pass
    try:
        os.waitpid(pid, 0)
    except ChildProcessError:
        pass
    return buf


def plain(raw: bytes) -> str:
    text = re.sub(rb"\x1b\[[0-9;?]*[ -/]*[@-~]", b"", raw).decode("utf-8", "replace")
    return re.sub(r"\x1b\][^\x07]*\x07", "", text)


def main() -> int:
    shutil.rmtree(BASE, ignore_errors=True)
    os.makedirs(BASE, exist_ok=True)
    mock, port = start_mock()
    result = {}
    try:
        # 1) 推理模型：shift+tab 切换 + 请求体带 reasoning_effort
        prepare(port, True)
        raw = session(BINARY, [b"\x1b[Z", b"hello\r"])
        text = plain(raw)
        result["reasoning_cycle_status"] = "Thinking level:" in text
        result["reasoning_cycle_level"] = (re.search(r"Thinking level: (\w+)", text) or [None, None])[1]
        request = ""
        if os.path.exists(os.path.join(BASE, "requests.jsonl")):
            first = json.loads(open(os.path.join(BASE, "requests.jsonl")).readline())
            request = first.get("body", "")
        result["request_has_reasoning_effort"] = "reasoning_effort" in request

        # 2) 非推理模型：提示不支持
        prepare(port, False)
        text = plain(session(BINARY, [b"\x1b[Z"]))
        result["non_reasoning_notice"] = "Current model does not support thinking" in text

        # 2b) `/thinking` 选择器：Enter 选中即生效（状态行 `Thinking level: X`），
        #     选择器内 ctrl+s = 设为默认（`Default thinking level: X`）
        prepare(port, True)
        raw = session(BINARY, [b"/thinking", b"\r", b"\x1b[B", b"\r"])
        text = plain(raw)
        result["selector_enter_status"] = "Thinking level:" in text
        prepare(port, True)
        raw = session(BINARY, [b"/thinking", b"\r", b"\x1b[B", b"\x13"])
        text = plain(raw)
        result["selector_ctrl_s_status"] = "Default thinking level:" in text
        settings = json.loads(open(os.path.join(CFG, "settings.json")).read())
        result["selector_saved_level"] = settings.get("defaultThinkingLevel")

        # 3) ctrl+s 保存
        prepare(port, True)
        session(BINARY, [b"\x1b[Z", b"\x13", b"hello\r"])
        settings = json.loads(open(os.path.join(CFG, "settings.json")).read())
        result["saved_level"] = settings.get("defaultThinkingLevel")
        result["unknown_key_kept"] = settings.get("theme") == "dark"
        entries = []
        for name in os.listdir(SESS):
            entries.extend(json.loads(line) for line in open(os.path.join(SESS, name)) if line.strip())
        result["session_entry"] = any(entry.get("type") == "thinking_level_change" for entry in entries)
    finally:
        mock.kill()
    ok = (
        result["reasoning_cycle_status"]
        and result["request_has_reasoning_effort"]
        and result["non_reasoning_notice"]
        and result["saved_level"] not in (None, "off")
        and result["unknown_key_kept"]
        and result["session_entry"]
    )
    result["ok"] = ok
    print(json.dumps(result, ensure_ascii=False))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
