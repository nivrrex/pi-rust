#!/usr/bin/env python3
"""P8a/M：TLS 证书校验与 DNS 失败路径（不联网、不用任何真实凭据）。

两个用例，各自起一个 pi-rs 会话（pty 驱动）：
1. **证书**：本地自签证书的 HTTPS 服务端 ⇒ 期望错误里出现证书类关键字
   （证明 TLS 握手走通、校验开启；根证书来自内置 `webpki-roots` 而不是系统信任库）；
2. **DNS**：`https://no-such-host.invalid/v1` ⇒ 期望错误里出现 `dns error`。

用法（DNS 用例需要把 resolv.conf 换成快速失败的版本，否则 `unshare -n` 里会等
不可达的 DNS 超时）：
```
unshare -n -m sh -c 'ip link set lo up; mount --bind /tmp/fast-fail-resolv.conf /etc/resolv.conf; \\
  PI_RS_BIN=<二进制> python3 tools/difftest/pty_tls_dns_check.py'
```
"""
import fcntl, json, os, pty, re, select, shutil, socket, ssl, struct, subprocess, sys, termios, threading, time

REPO = str(__import__("pathlib").Path(__file__).resolve().parents[2])
BINARY = os.environ.get("PI_RS_BIN", f"{REPO}/target/debug/pi-rs")
BASE = "/tmp/p8a-tls-dns"
CFG, SESS, HOME, WORK = (os.path.join(BASE, name) for name in ("cfg", "sess", "home", "work"))


def make_cert(directory: str) -> tuple:
    cert = os.path.join(directory, "cert.pem")
    key = os.path.join(directory, "key.pem")
    subprocess.run(
        ["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
         "-subj", "/CN=localhost", "-keyout", key, "-out", cert],
        check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return cert, key


def start_tls_server(cert: str, key: str) -> int:
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(cert, key)
    listener = socket.socket()
    listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    listener.bind(("127.0.0.1", 0))
    listener.listen(5)
    port = listener.getsockname()[1]

    def serve() -> None:
        while True:
            try:
                raw, _ = listener.accept()
            except OSError:
                return
            try:
                with context.wrap_socket(raw, server_side=True) as conn:
                    conn.recv(65536)
                    conn.sendall(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            except OSError:
                pass

    threading.Thread(target=serve, daemon=True).start()
    return port


def run_case(base_url: str, wait: float = 20.0) -> dict:
    for path in (CFG, SESS, HOME, WORK):
        shutil.rmtree(path, ignore_errors=True)
        os.makedirs(path, exist_ok=True)
    with open(os.path.join(CFG, "models.json"), "w") as handle:
        json.dump({"providers": {"mock": {"baseUrl": base_url, "api": "openai-completions",
                                          "apiKey": "k", "models": [{"id": "mock-model"}]}}}, handle)
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
    sent = False
    deadline = time.time() + wait
    next_send = time.time() + 5
    while time.time() < deadline:
        if not sent and time.time() >= next_send:
            os.write(fd, b"hello\r")
            sent = True
        if select.select([fd], [], [], 0.03)[0]:
            try:
                chunk = os.read(fd, 1 << 16)
            except OSError:
                break
            if not chunk:
                break
            buf += chunk
            if b"Error:" in buf:
                break
    try:
        os.write(fd, b"\x15\x04")
        time.sleep(0.3)
    except OSError:
        pass
    try:
        os.close(fd)
    except OSError:
        pass
    try:
        os.waitpid(pid, 0)
    except ChildProcessError:
        pass
    text = buf.decode("utf-8", "replace")
    text = re.sub(r"\x1b\][^\x07]*\x07", "", re.sub(r"\x1b\[[0-9;?]*[ -/]*[@-~]", "", text))
    flat = " ".join(line.strip() for line in text.splitlines() if line.strip())
    return {"saw_error": "Error:" in flat, "text": flat[-500:]}


def main() -> int:
    shutil.rmtree(BASE, ignore_errors=True)
    os.makedirs(BASE, exist_ok=True)
    cert, key = make_cert(BASE)
    port = start_tls_server(cert, key)
    cert_case = run_case(f"https://127.0.0.1:{port}/v1")
    cert_kind = "certificate" if re.search(
        r"certificat|unknownissuer|invalid peer|tls|ssl|handshake", cert_case["text"], re.I) else "other"
    dns_case = run_case("https://no-such-host.invalid/v1")
    dns_kind = "dns" if re.search(r"dns error|failed to lookup|name or service", dns_case["text"], re.I) else "other"
    result = {
        "binary": os.path.basename(BINARY),
        "cert_case": cert_kind,
        "dns_case": dns_kind,
        "cert_tail": cert_case["text"][-120:],
        "dns_tail": dns_case["text"][-120:],
    }
    print(json.dumps(result, ensure_ascii=False))
    return 0 if cert_kind == "certificate" and dns_kind == "dns" else 1


if __name__ == "__main__":
    sys.exit(main())
