#!/usr/bin/env python3
"""P7-8c/L：长会话曲线（N 轮提交 → 首字符延迟、请求体大小）。

判据：N=150 与 N=1 **同量级**（不随轮次增长），请求体**线性**增长。
用法：unshare -n sh -c 'ip link set lo up; python3 tools/difftest/turn_latency_curve.py'
"""
import fcntl, json, os, pty, re, select, shutil, struct, subprocess, sys, termios, time

REPO = str(__import__("pathlib").Path(__file__).resolve().parents[2])
BASE = "/tmp/p78c-curve"
CFG, SESS, HOME, WORK = (os.path.join(BASE, name) for name in ("cfg", "sess", "home", "work"))
POINTS = (1, 20, 60, 150)


def prepare_scenario() -> str:
    scenario = {"id": "P78C-CURVE", "provider": "openai-completions", "model": "mock-model",
                "responses": [{"file": "fixtures/sse/openai/text-basic.sse"} for _ in range(200)],
                "files": {}, "prompt": "hi", "description": "L 曲线", "tui": {"cols": 100, "rows": 30, "steps": []}}
    path = os.path.join(BASE, "scenario.json")
    with open(path, "w") as handle:
        json.dump(scenario, handle)
    return path


def start_mock(scenario: str):
    request_log = os.path.join(BASE, "requests.jsonl")
    log = os.path.join(BASE, "mock.out")
    handle = open(log, "w+")
    process = subprocess.Popen(
        [f"{REPO}/target/debug/mock-provider", scenario, "--port", "0", "--request-log", request_log],
        stdout=handle, stderr=subprocess.STDOUT)
    for _ in range(200):
        time.sleep(0.05)
        with open(log) as reader:
            text = reader.read()
        match = re.search(r":(\d+)\s*$", text.strip().splitlines()[-1] if text.strip() else "")
        if match:
            return process, int(match.group(1)), request_log
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
    mock, port, request_log = start_mock(prepare_scenario())
    try:
        prepare_config(port)
        env = dict(os.environ, HOME=HOME, PI_CODING_AGENT_DIR=CFG, PI_CODING_AGENT_SESSION_DIR=SESS)
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(WORK)
            os.execvpe(f"{REPO}/target/debug/pi-rs", ["pi-rs"], env)
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))

        def drain(seconds: float) -> bytes:
            data = b""
            end = time.time() + seconds
            while time.time() < end:
                if select.select([fd], [], [], 0.01)[0]:
                    try:
                        chunk = os.read(fd, 1 << 16)
                    except OSError:
                        return data
                    if not chunk:
                        return data
                    data += chunk
                    end = time.time() + 0.05
            return data

        drain(6.0)
        first_char, sizes = {}, {}
        sizes_before = 0
        for turn in range(1, max(POINTS) + 1):
            os.write(fd, b"hello")
            start = time.time()
            os.write(fd, b"\r")
            latency = None
            # 一轮结束 = 首个输出块之后**静默 250ms**（不能等回复文本：第二轮的增量输出里
            # 已经不含 `Hello world`，按文本判会一直等到超时）。
            while time.time() - start < 15:
                chunk = drain(0.05)
                if not chunk:
                    if latency is not None:
                        break
                    continue
                if latency is None:
                    latency = (time.time() - start) * 1000
            os.write(fd, b"\x15")
            drain(0.15)
            if turn in POINTS:
                first_char[turn] = round(latency or -1, 1)
                with open(request_log) as handle:
                    lines = handle.readlines()
                sizes[turn] = len(lines[-1]) if lines else 0
        os.write(fd, b"\x04")
        drain(0.8)
        try:
            os.close(fd)
        except OSError:
            pass
        os.waitpid(pid, 0)
        print(json.dumps({"turns": POINTS,
                          "first_char_ms": [first_char[t] for t in POINTS],
                          "last_request_bytes": [sizes[t] for t in POINTS]}))
        base = first_char[1]
        worst = max(first_char[t] for t in POINTS)
        return 0 if base > 0 and worst < max(5 * base, 500) else 1
    finally:
        mock.kill()


if __name__ == "__main__":
    sys.exit(main())
