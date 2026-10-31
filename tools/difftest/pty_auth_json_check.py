#!/usr/bin/env python3
"""P7-9/401：`auth.json` 里的 key 必须变成 `Authorization: Bearer …`。

真机现象（openrouter）：`Error: 401 No cookie auth credentials found` —— 该文案说明请求
**完全没带 Authorization 头**。根因：`CredentialSources.stored` 恒传 None，`auth.json` 从未被读取。

本脚本用一个**裸 TCP 服务**记录请求头（mock-provider 的请求日志不含 header），
config 只把 key 放在 `auth.json`（`models.json` 不带 `apiKey`），断言：
- 收到且仅收到 1 个请求；
- 请求头里有 `authorization: Bearer <key>`；
- 回复正常渲染（不出现 401）。

用法：unshare -n sh -c 'ip link set lo up; python3 tools/difftest/pty_auth_json_check.py'
"""
import fcntl, json, os, pty, re, select, shutil, socket, struct, sys, threading, time, termios

REPO = str(__import__("pathlib").Path(__file__).resolve().parents[2])
# 允许用 `PI_RS_BIN` 指定被测二进制（P8a 用它跑 musl 静态构建）。
BINARY = os.environ.get("PI_RS_BIN", f"{REPO}/target/debug/pi-rs")
BASE = "/tmp/p79-auth-check"
CFG, SESS, HOME, WORK = (os.path.join(BASE, name) for name in ("cfg", "sess", "home", "work"))
KEY = "sk-or-selftest-0123456789"


def start_server() -> tuple:
    server = socket.socket()
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind(("127.0.0.1", 0))
    server.listen(5)
    headers: list = []

    def serve() -> None:
        while True:
            try:
                conn, _ = server.accept()
            except OSError:
                return
            data = conn.recv(65536).decode("utf-8", "replace")
            headers.append(data.split("\r\n\r\n")[0])
            body = (
                b'data: {"id":"x","object":"chat.completion.chunk","created":1,"model":"m",'
                b'"choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"},"finish_reason":null}]}\n\n'
                b'data: {"id":"x","object":"chat.completion.chunk","created":1,"model":"m",'
                b'"choices":[{"index":0,"delta":{"content":" world"},"finish_reason":"stop"}],'
                b'"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}\n\n'
                b"data: [DONE]\n\n"
            )
            conn.sendall(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: "
                + str(len(body)).encode()
                + b"\r\nConnection: close\r\n\r\n"
                + body
            )
            conn.close()

    threading.Thread(target=serve, daemon=True).start()
    return server, server.getsockname()[1], headers


def prepare(port: int) -> None:
    for path in (CFG, SESS, HOME, WORK):
        shutil.rmtree(path, ignore_errors=True)
        os.makedirs(path, exist_ok=True)
    with open(os.path.join(CFG, "models.json"), "w") as handle:
        json.dump({"providers": {"openrouter": {"models": [
            {"id": "openrouter/free", "name": "Free", "api": "openai-completions",
             "baseUrl": f"http://127.0.0.1:{port}/v1", "contextWindow": 200000}]}}}, handle)
    with open(os.path.join(CFG, "settings.json"), "w") as handle:
        json.dump({"defaultProvider": "openrouter", "defaultModel": "openrouter/free"}, handle)
    # **只**在 auth.json 里放 key（models.json 没有 apiKey）
    with open(os.path.join(CFG, "auth.json"), "w") as handle:
        json.dump({"openrouter": {"type": "api_key", "key": KEY}}, handle)


def main() -> int:
    shutil.rmtree(BASE, ignore_errors=True)
    os.makedirs(BASE, exist_ok=True)
    server, port, headers = start_server()
    try:
        prepare(port)
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
        os.write(fd, b"hello\r")
        drain(4.0)
        os.write(fd, b"\x15\x04")
        drain(0.6)
        try:
            os.close(fd)
        except OSError:
            pass
        os.waitpid(pid, 0)
    finally:
        server.close()
    text = re.sub(rb"\x1b\[[0-9;?]*[ -/]*[@-~]", b"", screen).decode("utf-8", "replace")
    auth = [line for block in headers for line in block.splitlines() if line.lower().startswith("authorization")]
    result = {
        "requests": len(headers),
        "authorization_present": bool(auth),
        "authorization_matches_key": any(KEY in line for line in auth),
        "reply_rendered": "Hello" in text,
        "saw_401": "401" in text,
    }
    print(json.dumps(result))
    ok = (
        result["requests"] == 1
        and result["authorization_present"]
        and result["authorization_matches_key"]
        and result["reply_rendered"]
        and not result["saw_401"]
    )
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
