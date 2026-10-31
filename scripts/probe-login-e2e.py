#!/usr/bin/env python3
"""`/login` 端到端探针：pty 起 pi-rs，本地 SSE mock provider，走完登录并让模型回一句话。

用法：`python3 scripts/probe-login-e2e.py [--impl path/to/pi-rs]`
输出一行 JSON：登录成功、auth.json 权限/内容、回合是否拿到助手回复。
不使用任何真实凭据（key 为 `sk-probe-<随机>`），mock 只监听 127.0.0.1。
"""
import argparse, http.server, json, os, pty, re, select, shutil, struct, tempfile, threading, time

SSE = (
    'data: {"id":"chunk-1","object":"chat.completion.chunk","created":1,'
    '"model":"mock-model","choices":[{"index":0,"delta":{"role":"assistant","content":"pong"},'
    '"finish_reason":null}]}\n\n'
    'data: {"id":"chunk-2","object":"chat.completion.chunk","created":1,'
    '"model":"mock-model","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],'
    '"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}\n\n'
    'data: [DONE]\n\n'
)


class Handler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("content-length", "0"))
        self.rfile.read(length)
        body = SSE.encode()
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--impl", default="./target/debug/pi-rs")
    args = parser.parse_args()

    server = http.server.HTTPServer(("127.0.0.1", 0), Handler)
    port = server.server_address[1]
    threading.Thread(target=server.serve_forever, daemon=True).start()

    cfg = tempfile.mkdtemp(prefix="pi-login-e2e-")
    with open(os.path.join(cfg, "models.json"), "w") as handle:
        json.dump({"providers": {"mockp": {
            "api": "openai-completions",
            "baseUrl": f"http://127.0.0.1:{port}/v1",
            "models": [{"id": "mock-model"}],
        }}}, handle)
    with open(os.path.join(cfg, "settings.json"), "w") as handle:
        handle.write("{}")

    pid, fd = pty.fork()
    if pid == 0:
        os.environ.update({"PI_CODING_AGENT_DIR": cfg, "TERM": "xterm-256color"})
        os.execv(args.impl, [args.impl])
    import fcntl, termios
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
    buf = bytearray()

    def pump(seconds):
        end = time.time() + seconds
        while time.time() < end:
            ready, _, _ = select.select([fd], [], [], 0.2)
            if ready:
                try:
                    buf.extend(os.read(fd, 65536))
                except OSError:
                    return
            else:
                time.sleep(0.02)

    def wait(marker, timeout=25.0):
        end = time.time() + timeout
        while time.time() < end:
            if marker.encode() in buf:
                return True
            pump(0.2)
        return False

    def send(text):
        os.write(fd, text.encode())

    result = {"startup": wait("no-model"), "login_prompt": False, "logged_in": False,
              "auth_mode": None, "auth_entry": None, "reply": False}
    send("/login mockp\r")
    result["login_prompt"] = wait("Logging in to mockp")
    result["logged_in"] = wait("Logged in to mockp") if False else False
    send("sk-probe-123\r")
    result["logged_in"] = wait("Logged in to mockp")
    send("ping\r")
    result["reply"] = wait("pong", timeout=30.0)
    auth = os.path.join(cfg, "auth.json")
    if os.path.exists(auth):
        result["auth_mode"] = oct(os.stat(auth).st_mode & 0o777)
        text = open(auth).read()
        result["auth_entry"] = "api_key" in text and "sk-probe-123" in text
    send("/quit\r")
    pump(0.5)
    try:
        os.kill(pid, 9)
    except ProcessLookupError:
        pass
    shutil.rmtree(cfg, ignore_errors=True)
    server.shutdown()
    print(json.dumps(result))


if __name__ == "__main__":
    main()
