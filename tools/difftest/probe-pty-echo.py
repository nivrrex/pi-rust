#!/usr/bin/env python3
"""P7-6ad/D.4：**最小 pty 实验**——在 pty-helper 同样的 pty 配置下，子进程先写出
`\\x1b[?2004h\\x1b[>7u\\x1b[?u\\x1b[c\\x1b[?25l`，再进入 raw 读 fd 0，记录收到的全部字节。

用途：判定"启动序列写回自身 stdin"是否与 **pty/环境**有关（与 pi-rs 无关）。
输出：JSON 一行（收到的字节十六进制 + 相对时间 + termios 关键标志）。
"""
import json
import os
import pty
import select
import struct
import sys
import termios
import time
import fcntl

STARTUP = b"\x1b[?2004h\x1b[>7u\x1b[?u\x1b[c\x1b[?25l"


def child(fd: int) -> None:
    # 与 crossterm::enable_raw_mode 等价：关闭 ICANON/ECHO 等
    attrs = termios.tcgetattr(fd)
    attrs[3] &= ~(termios.ECHO | termios.ICANON | termios.IEXTEN | termios.ISIG)
    termios.tcsetattr(fd, termios.TCSANOW, attrs)
    raw_after = termios.tcgetattr(fd)
    os.write(fd, STARTUP)
    received = []
    start = time.time()
    deadline = start + 1.2
    while time.time() < deadline:
        ready, _, _ = select.select([fd], [], [], 0.1)
        if not ready:
            continue
        try:
            data = os.read(fd, 4096)
        except OSError:
            break
        if not data:
            break
        received.append({"dt_ms": round((time.time() - start) * 1000), "hex": data.hex()})
        if "".join(item["hex"] for item in received).find("37623175") >= 0:  # "7;1u"
            break
    lflag = raw_after[3]
    print(json.dumps({
        "received": received,
        "lflag": hex(lflag),
        "echo": bool(lflag & termios.ECHO),
        "icanon": bool(lflag & termios.ICANON),
        "isatty": os.isatty(fd),
    }), flush=True)
    os._exit(0)


def main() -> None:
    pid, fd = pty.fork()
    if pid == 0:
        child(0)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
    out = b""
    while True:
        ready, _, _ = select.select([fd], [], [], 3.0)
        if not ready:
            break
        try:
            chunk = os.read(fd, 65536)
        except OSError:
            break
        if not chunk:
            break
        out += chunk
    sys.stdout.write(out.decode("utf-8", "replace"))


if __name__ == "__main__":
    main()
