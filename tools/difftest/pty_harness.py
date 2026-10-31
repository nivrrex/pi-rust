#!/usr/bin/env python3
"""P7-6aj/H：**pty 骨架**——mock-provider + 真实 pi-rs（隔离配置目录）+ 自建 pty。

供 M1（缩放）/M3（不应答路径）/E（应答注入）复用。不修改 runner。

用法（各检查脚本 `from pty_harness import Harness`）：
    H = Harness(cols=100, rows=30)
    H.start()
    H.type("say hi\\r")
    H.wait_for("Hello", 10)
    H.set_size(60, 30)          # TIOCSWINSZ + SIGWINCH
    H.close()
"""
import json
import os
import pty
import re
import select
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time
import fcntl

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
PI_RS = os.path.join(REPO, "target/debug/pi-rs")
MOCK = os.path.join(REPO, "target/debug/mock-provider")
SCENARIO = os.path.join(REPO, "scenarios-tui", "T01.json")

ANSI = re.compile(rb"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[=>]")


class Harness:
    def __init__(self, cols=100, rows=30, scenario=SCENARIO, respond=False, cmd=None, kitty=False):
        self.cols, self.rows, self.scenario = cols, rows, scenario
        self.respond = respond          # True ⇒ 自动应答 DA1/kitty/OSC11（复刻 pty-helper）
        self.cmd = cmd                  # None ⇒ pi-rs；否则为命令 argv（如 TS bundle）
        # P7-6al/A：`kitty=True` ⇒ 对 `\x1b[?u` 应答 `\x1b[?7u`（flags=7，真机 Alacritty 的行为），
        # 这样才会走 CSI-u 编码 + press/release 路径（旧 helper 恒答 `?0u`，kitty 从未激活）。
        self.kitty = kitty
        self.answering = True           # M3 可置 False 模拟"终端不应答"
        self.buf = bytearray()
        self.pid = None
        self.fd = None
        self.mock = None
        self.tmp = None

    # ---- 生命周期 ----
    def start(self):
        self.tmp = tempfile.mkdtemp(prefix="ptyh-")
        for d in ("home", "cfg", "sess", "work"):
            os.makedirs(os.path.join(self.tmp, d), exist_ok=True)
        request_log = os.path.join(self.tmp, "requests.jsonl")
        self.mock = subprocess.Popen(
            [MOCK, self.scenario, "--port", "0", "--request-log", request_log],
            cwd=REPO, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
        )
        port = None
        deadline = time.time() + 15
        while time.time() < deadline:
            line = self.mock.stdout.readline()
            if not line:
                break
            m = re.search(r"listening\s+[\d.]+:(\d+)", line)
            if m:
                port = int(m.group(1))
                break
        if port is None:
            raise RuntimeError("mock-provider 启动失败（未读到端口）")
        scenario = json.load(open(self.scenario))
        provider = scenario.get("provider", "openai-completions")
        with open(os.path.join(self.tmp, "cfg", "models.json"), "w") as f:
            json.dump({"providers": {provider: {"baseUrl": f"http://127.0.0.1:{port}/v1",
                                                "api": provider, "apiKey": "test-key",
                                                "models": [{"id": scenario.get("model", "mock-model")}]}}}, f)
        with open(os.path.join(self.tmp, "cfg", "settings.json"), "w") as f:
            json.dump({"defaultProvider": provider, "defaultModel": scenario.get("model", "mock-model")}, f)
        env = dict(os.environ)
        env.update({
            "HOME": os.path.join(self.tmp, "home"),
            "PI_CODING_AGENT_DIR": os.path.join(self.tmp, "cfg"),
            "PI_CODING_AGENT_SESSION_DIR": os.path.join(self.tmp, "sess"),
            # 切默认后不再需要 PI_TUI_RENDERER；保留设置以兼容过渡期
            "TERM": "xterm-256color",
        })
        env.pop("PI_TUI_RENDERER", None) if os.environ.get("PTYH_NO_RENDERER") else env.setdefault("PI_TUI_RENDERER", "r3")
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.chdir(os.path.join(self.tmp, "work"))
            argv = self.cmd or [PI_RS]
            os.execvpe(argv[0], argv, env)
        self.set_size(self.cols, self.rows)
        return self

    def close(self):
        try:
            if self.pid:
                os.kill(self.pid, signal.SIGKILL)
                os.waitpid(self.pid, 0)
        except (ProcessLookupError, ChildProcessError):
            pass
        if self.mock:
            self.mock.kill()
            self.mock.wait()
        if self.tmp:
            shutil.rmtree(self.tmp, ignore_errors=True)

    # ---- I/O ----
    def set_size(self, cols, rows):
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        self.cols, self.rows = cols, rows

    def resize(self, cols, rows=None):
        self.set_size(cols, rows or self.rows)
        os.kill(self.pid, signal.SIGWINCH)

    def type(self, text):
        os.write(self.fd, text.encode())

    def send_keys(self, keys, mod: int = 1, code_map: dict = None):
        """按 kitty 协议注入按键：每键发 **press（:1）** 与 **release（:3）** 两个事件。

        `keys` 是字符串（按 ASCII 码点）或 `code_map` 给出的显式码点列表（如 `{"shift+enter": 13, "mod": 1}`）。
        """
        for key in keys:
            cp = ord(key)
            self.write_raw(f"\x1b[{cp};{mod}:1u".encode())
            self.pump(0.03)
            self.write_raw(f"\x1b[{cp};{mod}:3u".encode())
            self.pump(0.03)

    def write_raw(self, data: bytes):
        os.write(self.fd, data)

    def pump(self, seconds=0.2):
        end = time.time() + seconds
        while time.time() < end:
            ready, _, _ = select.select([self.fd], [], [], 0.05)
            if not ready:
                continue
            try:
                chunk = os.read(self.fd, 65536)
            except OSError:
                return
            if not chunk:
                return
            self.buf += chunk
            if self.respond and self.answering:
                self._maybe_answer(chunk)

    def _maybe_answer(self, chunk: bytes):
        kitty_reply = b"\x1b[?7u" if self.kitty else b"\x1b[?0u"
        replies = [(b"\x1b[c", b"\x1b[?62;c"), (b"\x1b[>7u", b"\x1b[>7;1u"),
                   (b"\x1b[?u", kitty_reply),
                   (b"\x1b]11;?\x07", b"\x1b]11;rgb:0000/0000/0000\x07")]
        for query, reply in replies:
            if query in chunk:
                os.write(self.fd, reply)

    # ---- 读屏 ----
    def text(self) -> str:
        return ANSI.sub(b"", bytes(self.buf)).decode("utf-8", "replace")

    def wait_for(self, needle, timeout=10.0) -> bool:
        end = time.time() + timeout
        while time.time() < end:
            if needle in self.text():
                return True
            self.pump(0.1)
        return needle in self.text()

    def clean_buffer(self):
        self.buf.clear()
