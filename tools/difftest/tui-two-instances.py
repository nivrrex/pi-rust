#!/usr/bin/env python3
"""F44/S0 辅助：并发跑两个 pi-rs 实例（A、B），各自在 pty 里，按顺序退出，收集退出输出。

用法：tui-two-instances.py <agent_dir> <argv...>
输出 JSON：`{"a_exit": "...", "b_exit": "...", "log_file": "..."}`
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


class Session:
    def __init__(self, work: str, tag: str) -> None:
        self.tag = tag
        env = dict(
            os.environ,
            PI_CODING_AGENT_DIR=AGENT,
            TERM="xterm-256color",
            # 让扩展为每个实例打唯一标记（测试夹具读它）。
            PI_F44_INSTANCE=tag,
        )
        self.master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
        pid = os.fork()
        if pid == 0:
            try:
                os.close(self.master)
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
        self.pid = pid
        self.slave = slave
        self.out = bytearray()
        self.answered = False
        self.sent = False

    def pump(self, seconds: float) -> None:
        end = time.time() + seconds
        while time.time() < end:
            r, _, _ = select.select([self.master], [], [], 0.01)
            if not r:
                continue
            try:
                chunk = os.read(self.master, 65536)
            except OSError:
                return
            if not chunk:
                return
            self.out.extend(chunk)
            if not self.answered and b"\x1b[c" in self.out:
                self.answered = True
                try:
                    os.write(self.master, b"\x1b[?1;2c\x1b[?5u")
                except OSError:
                    pass

    def ready(self) -> bool:
        return b"(auto)" in self.out or b"Warning: No models" in self.out

    def quit(self) -> str:
        try:
            os.write(self.master, b"\x04")
        except OSError:
            pass
        start = time.time()
        while time.time() - start < 20:
            self.pump(0.05)
            wpid, _st = os.waitpid(self.pid, os.WNOHANG)
            if wpid == self.pid:
                break
        self.pump(0.3)
        try:
            os.kill(self.pid, 9)
            os.waitpid(self.pid, 0)
        except OSError:
            pass
        os.close(self.master)
        os.close(self.slave)
        text = self.out.decode("utf8", "replace")
        index = text.rfind("\x1b[?2004l")
        return text[index:] if index >= 0 else text


def main() -> int:
    work = "/tmp/f44work"
    os.makedirs(work, exist_ok=True)
    a = Session(work, "A")
    b = Session(work, "B")
    # 等两个都进 TUI（宿主都已启动）。
    deadline = time.time() + 45
    while time.time() < deadline and not (a.ready() and b.ready()):
        a.pump(0.05)
        b.pump(0.05)
    # 给扩展/宿主一点时间把标记日志写出来。
    a.pump(0.6)
    b.pump(0.6)
    a_exit = a.quit()
    b_exit = b.quit()
    log_path = os.path.join(AGENT, "extensions.log")
    log = ""
    if os.path.exists(log_path):
        with open(log_path, "r", encoding="utf8", errors="replace") as handle:
            log = handle.read()
    print(json.dumps({"a_exit": a_exit, "b_exit": b_exit, "log_file": log}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
