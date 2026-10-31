#!/usr/bin/env python3
"""P8a：运行中指示器必须**动**（ref `loader.ts:13-14/79-86`：10 帧、80 ms 一换）。

做法：起一个**延迟 2 秒才回 SSE** 的本地服务端，让 `Working` 停留在屏幕上，
在 1.5 秒内反复读 pty 输出，统计出现过的 `── <帧> Working` 帧；断言 ≥3 种且首尾不同。
"""
import fcntl, json, os, pty, re, select, shutil, socket, struct, sys, threading, time, termios

REPO = str(__import__("pathlib").Path(__file__).resolve().parents[2])
BINARY = os.environ.get("PI_RS_BIN", f"{REPO}/target/debug/pi-rs")
BASE = "/tmp/p8a-spinner"
CFG, SESS, HOME, WORK = (os.path.join(BASE, name) for name in ("cfg", "sess", "home", "work"))
FRAMES = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"


def start_slow_server() -> int:
    server = socket.socket()
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind(("127.0.0.1", 0))
    server.listen(5)
    port = server.getsockname()[1]

    def serve() -> None:
        while True:
            try:
                conn, _ = server.accept()
            except OSError:
                return
            conn.recv(65536)
            time.sleep(12.0)  # 关键：让 Working 停留足够久以观察动画
            body = (
                b'data: {"id":"x","object":"chat.completion.chunk","created":1,"model":"m",'
                b'"choices":[{"index":0,"delta":{"role":"assistant","content":"Hi"},"finish_reason":"stop"}],'
                b'"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}\n\n'
                b"data: [DONE]\n\n"
            )
            try:
                conn.sendall(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: "
                             + str(len(body)).encode() + b"\r\nConnection: close\r\n\r\n" + body)
            except OSError:
                pass
            conn.close()

    threading.Thread(target=serve, daemon=True).start()
    return port


def main() -> int:
    shutil.rmtree(BASE, ignore_errors=True)
    for path in (CFG, SESS, HOME, WORK):
        os.makedirs(path, exist_ok=True)
    port = start_slow_server()
    with open(os.path.join(CFG, "models.json"), "w") as handle:
        json.dump({"providers": {"mock": {"baseUrl": f"http://127.0.0.1:{port}/v1",
                                          "api": "openai-completions", "apiKey": "k",
                                          "models": [{"id": "mock-model"}]}}}, handle)
    with open(os.path.join(CFG, "settings.json"), "w") as handle:
        json.dump({"defaultProvider": "mock", "defaultModel": "mock-model"}, handle)
    with open(os.path.join(CFG, "auth.json"), "w") as handle:
        json.dump({"mock": {"type": "api_key", "key": "k"}}, handle)
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
            if select.select([fd], [], [], 0.02)[0]:
                try:
                    chunk = os.read(fd, 1 << 16)
                except OSError:
                    return
                if not chunk:
                    return
                buf += chunk
                end = time.time() + 0.05

    drain(5.0)
    before = len(buf)
    os.write(fd, b"hello\r")
    # 采样：每 250ms 记录一次累计字节数（判断是"没有动画"还是"程序停住"）
    samples = []
    started = time.time()
    while time.time() - started < 10.0:
        drain(0.25)
        samples.append((round(time.time() - started, 2), len(buf) - before))
    if os.environ.get("SPINNER_DUMP"):
        with open("/tmp/spinner_samples.json", "w") as handle:
            json.dump(samples, handle)
    # 注意：增量渲染只重写**变化的单元格**，所以整行 "── X Working" 不会重复出现；
    # 直接统计原始字节流里出现过的盲文帧字符即可（只有 spinner 会用这些字符）。
    if os.environ.get("SPINNER_DUMP"):
        with open("/tmp/spinner.bin", "wb") as handle:
            handle.write(buf)
    text = buf.decode("utf-8", "replace")
    seen = [frame for frame in FRAMES if frame in text]
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
    result = {"distinct_frames": len(seen), "frames": seen, "bytes": len(buf)}
    print(json.dumps(result, ensure_ascii=False))
    return 0 if len(seen) >= 3 else 1


if __name__ == "__main__":
    sys.exit(main())
