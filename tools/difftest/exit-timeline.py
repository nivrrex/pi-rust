#!/usr/bin/env python3
"""F39/M2a：退出后 tty 状态时间线取证夹具。

在**同一 pty** 上先跑被测程序，再用同一个从设备模拟 shell 行为：
- 以被测进程返回时刻为 t=0，0–500ms 内每 10ms 读一次 termios（ECHO/ICANON/ISIG/IEXTEN）；
- 每 5ms 探测宿主 node 子进程存活（pid/ppid）；
- 记录被测程序退出前最后一次写 tty 的时刻与各类查询/应答时刻；
- t=0 后立刻按"真实 shell"把 termios 设成 raw（关 ECHO/ICANON），看是否被改回；
- 再向 pty 写一次 ESC [ A，看它是被字面回显还是被读走。

用法：exit-timeline.py <argv...>
输出：一行 JSON。
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

ARGV = sys.argv[1:]

FLAGS = {
    "echo": termios.ECHO,
    "icanon": termios.ICANON,
    "isig": termios.ISIG,
    "iexten": termios.IEXTEN,
}

QUERY_PATTERNS = [
    ("DA1", rb"\x1b\[c"),
    ("kitty_query", rb"\x1b\[\?u"),
    ("kitty_push", rb"\x1b\[>\d+u"),
    ("kitty_pop", rb"\x1b\[<u"),
    ("mok_on", rb"\x1b\[>4;2m"),
    ("mok_off", rb"\x1b\[>4;0m"),
    ("paste_on", rb"\x1b\[\?2004h"),
    ("paste_off", rb"\x1b\[\?2004l"),
    ("cursor_show", rb"\x1b\[\?25h"),
    ("osc_bg", rb"\x1b\]11;\?"),
]


def flags_of(fd):
    try:
        attrs = termios.tcgetattr(fd)
    except (OSError, termios.error):
        return None
    lflag = attrs[3]
    return {name: bool(lflag & bit) for name, bit in FLAGS.items()}


def set_raw(fd):
    try:
        attrs = termios.tcgetattr(fd)
    except (OSError, termios.error):
        return False
    attrs[3] &= ~(termios.ECHO | termios.ICANON | termios.ISIG | termios.IEXTEN)
    attrs[0] &= ~(termios.ICRNL | termios.IXON)
    try:
        termios.tcsetattr(fd, termios.TCSANOW, attrs)
    except (OSError, termios.error):
        return False
    return True


def descendants(root):
    """返回 root 的全部后代 pid（读 /proc/*/stat 的 ppid 列）。"""
    children = {}
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            with open(f"/proc/{entry}/stat", "r", encoding="utf8", errors="replace") as handle:
                data = handle.read()
        except OSError:
            continue
        # comm 可能含空格/括号 ⇒ 取最后一个 ')' 之后的字段
        rest = data[data.rfind(")") + 2 :].split()
        if len(rest) < 2:
            continue
        pid = int(entry)
        ppid = int(rest[1])
        children.setdefault(ppid, []).append(pid)
    out = []
    stack = [root]
    while stack:
        current = stack.pop()
        for child in children.get(current, []):
            out.append(child)
            stack.append(child)
    return out


def alive(pid):
    return os.path.exists(f"/proc/{pid}")


def spawn(master, slave, env, cwd):
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
            os.execvpe(ARGV[0], ARGV, env)
        except BaseException:
            os._exit(127)
    return pid


def main():
    work, home, cfg, sess = "/tmp/m2work", "/tmp/m2home", "/tmp/m2cfg", "/tmp/m2sess"
    for path in (work, home, cfg, sess):
        os.makedirs(path, exist_ok=True)
    with open(os.path.join(cfg, "settings.json"), "w", encoding="utf8") as handle:
        handle.write('{"theme":"dark","defaultProvider":"p","defaultModel":"m"}\n')
    with open(os.path.join(cfg, "models.json"), "w", encoding="utf8") as handle:
        json.dump(
            {
                "providers": {
                    "p": {
                        "baseUrl": "http://127.0.0.1:1/v1",
                        "api": "openai-completions",
                        "apiKey": "DUMMY",
                        "models": [{"id": "m"}],
                    }
                }
            },
            handle,
        )
    env = dict(
        os.environ,
        HOME=home,
        PI_CODING_AGENT_DIR=cfg,
        PI_CODING_AGENT_SESSION_DIR=sess,
        TERM="xterm-256color",
    )
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
    root = spawn(master, slave, env, work)

    events = []
    out = bytearray()
    last_write_at = 0.0
    answered = False
    sent_exit = False
    exit_at = None
    start = time.time()

    while time.time() - start < 25:
        if exit_at is None and not sent_exit and (b"(auto)" in out or b"Warning: No models" in out):
            sent_exit = True
            try:
                os.write(master, b"\x04")
            except OSError:
                pass
        r, _, _ = select.select([master], [], [], 0.02)
        if r:
            try:
                chunk = os.read(master, 65536)
            except OSError:
                chunk = b""
            if chunk:
                now = time.time()
                for name, pattern in QUERY_PATTERNS:
                    for _ in re.finditer(pattern, chunk):
                        events.append({"t": round(now - start, 3), "event": name})
                out.extend(chunk)
                last_write_at = now
                if not answered and b"\x1b[c" in out:
                    answered = True
                    try:
                        os.write(master, b"\x1b[?1;2c\x1b[?5u")
                    except OSError:
                        pass
                continue
        if sent_exit:
            wpid, status = os.waitpid(root, os.WNOHANG)
            if wpid == root:
                exit_at = time.time()
                events.append({"t": round(exit_at - start, 3), "event": "process_exit"})
                break

    t0 = exit_at if exit_at is not None else time.time()
    flags_at_exit = flags_of(slave)
    # 宿主 node 子进程：按 cmdline 找 `runner.mjs`（比按 ppid 找更可靠——宿主是
    # `setsid` 后的独立进程组，ppid 关系在退出瞬间可能已断）。
    kids = []
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            with open(f"/proc/{entry}/cmdline", "rb") as handle:
                cmdline = handle.read().decode("utf8", "replace")
        except OSError:
            continue
        if "runner.mjs" in cmdline:
            kids.append(int(entry))

    # (A) 先**不改** termios：模拟"用户刚退出、shell 还没起 readline 就把按键打进来"，
    # 看这些字节是否被行规程**字面回显**（真机 `^[[A` 的判据）。
    try:
        os.write(master, b"\x1b[A")
    except OSError:
        pass
    time.sleep(0.25)
    echo_a = b""
    r, _, _ = select.select([master], [], [], 0.3)
    if r:
        try:
            echo_a = os.read(master, 4096)
        except OSError:
            echo_a = b""
    buf_a = fcntl.ioctl(slave, termios.FIONREAD, struct.pack("I", 0))
    residual_a = struct.unpack("I", buf_a)[0]
    # 把可能残留的按键读掉，避免干扰下一段探测。
    if residual_a:
        try:
            os.read(slave, residual_a)
        except OSError:
            pass

    after = []
    set_raw_ok = set_raw(slave)
    after.append({"t": 0.0, "action": "shell_set_raw", "ok": set_raw_ok})
    kid_alive = {}
    for step in range(51):
        target = step * 0.01
        while time.time() - t0 < target:
            time.sleep(0.001)
        snapshot = flags_of(slave)
        after.append({"t": round(target, 2), "flags": snapshot})
        for pid in kids:
            kid_alive.setdefault(pid, []).append((round(target, 2), alive(pid)))

    # 模拟 shell 读按键：写入 ESC [ A，看它是被读走还是被字面回显。
    try:
        os.write(master, b"\x1b[A")
    except OSError:
        pass
    time.sleep(0.2)
    echoed = b""
    r, _, _ = select.select([master], [], [], 0.3)
    if r:
        try:
            echoed = os.read(master, 4096)
        except OSError:
            echoed = b""
    buf = fcntl.ioctl(slave, termios.FIONREAD, struct.pack("I", 0))
    residual = struct.unpack("I", buf)[0]

    print(
        json.dumps(
            {
                "argv": ARGV,
                "events": events,
                "exit_after_last_write_ms": round((t0 - last_write_at) * 1000, 1) if last_write_at else None,
                "child_pids": kids,
                "child_runner_found": bool(kids),
                "child_alive_samples": {
                    str(pid): {
                        "first_dead_at": next((t for t, ok in samples if not ok), None),
                        "alive_at_500ms": samples[-1][1] if samples else None,
                    }
                    for pid, samples in kid_alive.items()
                },
                "flags_at_exit": flags_at_exit,
                "termios_flags_samples": [item for item in after if "flags" in item],
                "flags_changed_after_raw": len(
                    {json.dumps(item["flags"], sort_keys=True) for item in after if item.get("flags")}
                )
                > 1,
                "cooked_phase_echo": echo_a.decode("latin1"),
                "cooked_phase_residual": residual_a,
                "post_exit_write_echo": echoed.decode("latin1"),
                "post_exit_residual_len": residual,
            },
            ensure_ascii=False,
        )
    )
    try:
        for pid in kids:
            os.kill(pid, 9)
    except OSError:
        pass
    try:
        os.kill(root, 9)
    except OSError:
        pass
    os.close(master)
    os.close(slave)
    return 0


if __name__ == "__main__":
    sys.exit(main())
