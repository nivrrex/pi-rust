#!/usr/bin/env python3
"""K1a：真实终端时序下的退出残留字节对比台。

用法：term-sim-residual.py <delay_ms> <binary...>
  delay_ms：终端应答查询的延迟（0 / 20 / 100），模拟终端处理耗时。
输出 JSON：启动字节统计、退出字节统计、退出后 tty 输入队列残留字节（十六进制 + 可读形态）。

设计要点（不改 run-tui / compare-tui）：
- 用 `pty.openpty()` + 手工 `setsid`/`TIOCSCTTY` 而不是 `pty.fork()`，因为要在**同一个 pty**
  上先后跑「被测程序」与「随后启动的 cat -v」，`pty.fork()` 每次都会新建一对。
- 退出后用 `FIONREAD` 读**从设备输入队列**里还剩多少字节：这些字节就是终端应答查询后
  程序没有读走、退出后被 shell 读成按键的东西（真机 `^[[A` 类现象的候选来源）。
- 之后再在同一 pty 上跑 `cat -v`，把残留字节按原样打印出来，作为人能看懂的证据。
"""

import fcntl
import json
import os
import pty
import re
import select
import struct
import sys
import termios
import time

DELAY_MS = int(sys.argv[1])
ARGV = sys.argv[2:]

STARTUP_STATS = ("kitty_push", "kitty_pop", "mok_on", "mok_off", "paste_on", "paste_off", "cursor_show")
PATTERNS = {
    "kitty_push": rb"\x1b\[>\d+u",
    "kitty_pop": rb"\x1b\[<u",
    "mok_on": rb"\x1b\[>4;2m",
    "mok_off": rb"\x1b\[>4;0m",
    "paste_on": rb"\x1b\[\?2004h",
    "paste_off": rb"\x1b\[\?2004l",
    "cursor_show": rb"\x1b\[\?25h",
}


def stats(raw: bytes) -> dict:
    out = {}
    for name, pat in PATTERNS.items():
        out[name] = len(re.findall(pat, raw))
    return out


def spawn(master: int, slave: int, argv: list, env: dict, cwd: str) -> int:
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
            os.chdir(cwd)
            os.execvpe(argv[0], argv, env)
        except BaseException:
            os._exit(127)
    return pid


def read_until(pid: int, master: int, predicate, on_chunk, deadline_s: float) -> bytes:
    out = b""
    end = time.time() + deadline_s
    while time.time() < end:
        if predicate(out):
            break
        r, _, _ = select.select([master], [], [], 0.1)
        if not r:
            continue
        try:
            chunk = os.read(master, 65536)
        except OSError:
            break
        if not chunk:
            break
        out += chunk
        on_chunk(out, chunk)
    return out


def main() -> int:
    work = "/tmp/k1work"
    home = "/tmp/k1home"
    cfg = "/tmp/k1cfg"
    sess = "/tmp/k1sess"
    for d in (work, home, cfg, sess):
        os.makedirs(d, exist_ok=True)
    env = dict(
        os.environ,
        HOME=home,
        PI_CODING_AGENT_DIR=cfg,
        PI_CODING_AGENT_SESSION_DIR=sess,
        TERM="xterm-256color",
    )
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
    # 从设备留在父进程里（第二个 cat -v 复用），子进程 dup 后自己关。
    pid = spawn(master, slave, ARGV, env, work)

    state = {"answered": False, "exited_keys": False}
    startup = b""
    exited = b""

    def on_chunk(buf: bytes, chunk: bytes) -> None:
        nonlocal startup, exited
        if not state["answered"] and b"\x1b[c" in buf:
            state["answered"] = True

            def reply() -> None:
                if DELAY_MS:
                    time.sleep(DELAY_MS / 1000.0)
                try:
                    os.write(master, b"\x1b[?1;2c")
                    os.write(master, b"\x1b[?5u")
                except OSError:
                    pass

            reply()
        if not state["exited_keys"] and b"(auto)" in buf:
            state["exited_keys"] = True
            try:
                os.write(master, b"\x04")
            except OSError:
                pass
        if state["exited_keys"]:
            exited += chunk
        else:
            startup += chunk

    read_until(pid, master, lambda _b: state["exited_keys"], on_chunk, 15.0)
    # 等进程真正退出（最多 6s）。
    status = None
    for _ in range(60):
        wpid, st = os.waitpid(pid, os.WNOHANG)
        if wpid == pid:
            status = st
            break
        time.sleep(0.1)
    if status is None:
        os.kill(pid, 9)
        os.waitpid(pid, 0)

    # 退出后，终端**仍然**会把我们早先查询的应答写入 pty（真实终端就是这样）。
    # 程序此时已经退出 ⇒ 这些字节只能留在从设备的输入队列里。
    late = b""
    for _ in range(5):
        r, _, _ = select.select([master], [], [], 0.3)
        if r:
            try:
                chunk = os.read(master, 65536)
            except OSError:
                break
            if not chunk:
                break
            exited += chunk
            late += chunk
            continue
        break
    for seq in (b"\x1b[?5u", b"\x1b[?1;2c"):
        try:
            os.write(master, seq)
        except OSError:
            pass
    time.sleep(0.2)

    buf = fcntl.ioctl(slave, termios.FIONREAD, struct.pack("I", 0))
    residual_len = struct.unpack("I", buf)[0]
    residual = b""
    if residual_len:
        try:
            residual = os.read(slave, residual_len)
        except OSError:
            residual = b""

    # 用同一 pty 上随后启动的 `cat -v` 复看残留（人能读懂的形态）。
    cat_pid = spawn(master, slave, ["cat", "-v"], env, work)
    time.sleep(0.3)
    cat_out = b""
    r, _, _ = select.select([master], [], [], 0.5)
    if r:
        try:
            cat_out = os.read(master, 65536)
        except OSError:
            cat_out = b""
    try:
        os.kill(cat_pid, 9)
        os.waitpid(cat_pid, 0)
    except OSError:
        pass

    print(
        json.dumps(
            {
                "delay_ms": DELAY_MS,
                "argv": ARGV,
                "exited": os.WIFEXITED(status) if status is not None else False,
                "exit_code": os.WEXITSTATUS(status) if status is not None and os.WIFEXITED(status) else None,
                "startup": stats(startup),
                "exit": stats(exited),
                "late_after_exit": late.decode("utf8", "replace"),
                "residual_len": residual_len,
                "residual_hex": residual.hex(),
                "residual_repr": repr(residual.decode("latin1")),
                "cat_v_seen": repr(cat_out.decode("latin1")),
            },
            ensure_ascii=False,
        )
    )
    os.close(master)
    os.close(slave)
    return 0


if __name__ == "__main__":
    sys.exit(main())
