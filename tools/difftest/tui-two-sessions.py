#!/usr/bin/env python3
"""F43/R0 辅助：在 pty 里连续跑两次 TUI 会话，收集每次退出后写到 stderr 的内容。

用法：tui-two-sessions.py <agent_dir> <argv...>
输出 JSON：`{"first_exit_stderr": "...", "second_exit_stderr": "...", "log_file": "..."}`
"""

import fcntl
import json
import os
import pty
import select
import struct
import sys
import termios
import time

AGENT = sys.argv[1]
ARGV = sys.argv[2:]


def run_once(work: str) -> str:
    env = dict(os.environ, PI_CODING_AGENT_DIR=AGENT, TERM="xterm-256color")
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
    pid = os.fork()
    if pid == 0:
        try:
            os.close(master)
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
            for fd in (0, 1, 2):
                os.dup2(slave, fd)
            if slave > 2:
                os.close(slave)
            os.chdir(work)
            os.execvpe(ARGV[0], ARGV, env)
        except BaseException:
            os._exit(127)
    out = bytearray()
    answered = False
    sent = False
    start = time.time()
    while time.time() - start < 40:
        r, _, _ = select.select([master], [], [], 0.01)
        if r:
            try:
                chunk = os.read(master, 65536)
            except OSError:
                break
            if not chunk:
                break
            out.extend(chunk)
            if not answered and b"\x1b[c" in out:
                answered = True
                try:
                    os.write(master, b"\x1b[?1;2c\x1b[?5u")
                except OSError:
                    pass
            continue
        if not sent and (b"(auto)" in out or b"Warning: No models" in out):
            sent = True
            try:
                os.write(master, b"\x04")
            except OSError:
                pass
            continue
        if sent:
            wpid, _st = os.waitpid(pid, os.WNOHANG)
            if wpid == pid:
                break
    # 读干净剩余输出
    time.sleep(0.2)
    while True:
        r, _, _ = select.select([master], [], [], 0.1)
        if not r:
            break
        try:
            chunk = os.read(master, 65536)
        except OSError:
            break
        if not chunk:
            break
        out.extend(chunk)
    try:
        os.kill(pid, 9)
        os.waitpid(pid, 0)
    except OSError:
        pass
    os.close(master)
    os.close(slave)
    text = out.decode("utf8", "replace")
    # 退出后、`?2004l` 之后的内容即"退出时一次性输出"的部分
    index = text.rfind("\x1b[?2004l")
    return text[index:] if index >= 0 else ""


def main() -> int:
    work = "/tmp/r0work"
    os.makedirs(work, exist_ok=True)
    first = run_once(work)
    log_after_first = ""
    log_path = os.path.join(AGENT, "extensions.log")
    if os.path.exists(log_path):
        with open(log_path, "r", encoding="utf8", errors="replace") as handle:
            log_after_first = handle.read()
    second = run_once(work)
    log_after_second = ""
    if os.path.exists(log_path):
        with open(log_path, "r", encoding="utf8", errors="replace") as handle:
            log_after_second = handle.read()
    print(
        json.dumps(
            {
                "first_exit_stderr": first,
                "second_exit_stderr": second,
                "log_after_first": log_after_first,
                "log_after_second": log_after_second,
            },
            ensure_ascii=False,
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
