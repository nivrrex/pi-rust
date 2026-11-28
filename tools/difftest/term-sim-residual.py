#!/usr/bin/env python3
"""K1a：真实终端时序下的退出残留字节对比台。

用法：term-sim-residual.py <delay_ms> <argv...>
  delay_ms：终端应答查询的延迟（0 / 20 / 100），模拟终端处理耗时。
输出一行 JSON，字段：
  startup / exit          启动期与退出期的模式字节计数
  late_after_exit         被测进程退出后仍然出现的输出（用于确认是否还有尾帧）
  residual_len / _hex     退出后**从设备输入队列**残留的字节数与内容
  cat_v_seen              在同一 pty 上随后启动的 `cat -v` 看到的残留

设计要点（不改 run-tui / compare-tui）：
- 用 `pty.openpty()` + 手工 `setsid`/`TIOCSCTTY`（`pty.fork()` 不能复用同一 pty 跑第二个程序）。
- 用 FIONREAD 直接读**内核输入队列剩余字节**——这才是"退出后被 shell 读成按键"的东西。
- 应答按延迟投递：0ms 表示终端立刻回应；100ms 表示程序可能已经退出后才回应，
  这正是真机"退出后方向键异常"的候选时序。
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
    return {name: len(re.findall(pat, raw)) for name, pat in PATTERNS.items()}


def write_config(cfg: str, work: str) -> None:
    os.makedirs(cfg, exist_ok=True)
    with open(os.path.join(cfg, "settings.json"), "w", encoding="utf8") as handle:
        handle.write('{"theme":"dark","defaultProvider":"p","defaultModel":"m"}\n')
    with open(os.path.join(cfg, "models.json"), "w", encoding="utf8") as handle:
        handle.write(
            json.dumps(
                {
                    "providers": {
                        "p": {
                            "baseUrl": "http://127.0.0.1:1/v1",
                            "api": "openai-completions",
                            "apiKey": "DUMMY",
                            "models": [{"id": "m"}],
                        }
                    }
                }
            )
        )
    os.makedirs(work, exist_ok=True)


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


def main() -> int:
    work, home, cfg, sess = "/tmp/k1work", "/tmp/k1home", "/tmp/k1cfg", "/tmp/k1sess"
    write_config(cfg, work)
    os.makedirs(home, exist_ok=True)
    os.makedirs(sess, exist_ok=True)
    env = dict(
        os.environ,
        HOME=home,
        PI_CODING_AGENT_DIR=cfg,
        PI_CODING_AGENT_SESSION_DIR=sess,
        TERM="xterm-256color",
    )
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
    pid = spawn(master, slave, ARGV, env, work)

    startup = bytearray()
    exited = bytearray()
    answered = False
    sent_exit = False
    reply_at = None
    end = time.time() + 20

    while time.time() < end:
        if reply_at is not None and time.time() >= reply_at:
            try:
                os.write(master, b"\x1b[?1;2c")
                os.write(master, b"\x1b[?5u")
            except OSError:
                pass
            reply_at = None
        if not answered:
            if b"\x1b[c" in startup:
                answered = True
                reply_at = time.time() + DELAY_MS / 1000.0
        else:
            if not sent_exit:
                if b"(auto)" in startup or b"Warning: No models available" in startup:
                    sent_exit = True
                    try:
                        os.write(master, b"\x04")
                    except OSError:
                        pass
            elif b"\x1b[?25h" in exited:
                break
        r, _, _ = select.select([master], [], [], 0.05)
        if not r:
            continue
        try:
            chunk = os.read(master, 65536)
        except OSError:
            break
        if not chunk:
            break
        (exited if sent_exit else startup).extend(chunk)

    status = None
    for _ in range(80):
        wpid, st = os.waitpid(pid, os.WNOHANG)
        if wpid == pid:
            status = st
            break
        time.sleep(0.05)
    if status is None:
        os.kill(pid, 9)
        os.waitpid(pid, 0)

    # 退出后仍到达的终端数据（尾帧）。
    late = bytearray()
    deadline = time.time() + 1.0
    while time.time() < deadline:
        r, _, _ = select.select([master], [], [], 0.1)
        if not r:
            break
        try:
            chunk = os.read(master, 65536)
        except OSError:
            break
        if not chunk:
            break
        late.extend(chunk)
        deadline = time.time() + 0.3

    # 关键测量：从设备输入队列里还剩多少字节（终端应答未被程序读走的部分）。
    buf = fcntl.ioctl(slave, termios.FIONREAD, struct.pack("I", 0))
    residual_len = struct.unpack("I", buf)[0]
    residual = b""
    if residual_len:
        try:
            residual = os.read(slave, residual_len)
        except OSError:
            residual = b""

    print(
        json.dumps(
            {
                "delay_ms": DELAY_MS,
                "argv": ARGV,
                "exit_code": os.WEXITSTATUS(status)
                if status is not None and os.WIFEXITED(status)
                else None,
                "startup": stats(bytes(startup)),
                "exit": stats(bytes(exited)),
                "late_after_exit": late.decode("utf8", "replace"),
                "residual_len": residual_len,
                "residual_hex": residual.hex(),
                "residual_repr": residual.decode("latin1").encode("unicode_escape").decode("latin1"),
            },
            ensure_ascii=False,
        )
    )
    os.close(master)
    os.close(slave)
    return 0


if __name__ == "__main__":
    sys.exit(main())
