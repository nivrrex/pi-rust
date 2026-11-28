#!/usr/bin/env python3
"""F40/N1：真实扩展场景的退出取证夹具。

在同一 pty 上跑被测程序；以返回时刻为 t=0，0–1500ms 内每 10ms 采样：
- termios 的 ECHO/ICANON/ISIG/IEXTEN 位（shell 在 t=0 已把 tty 设为 raw，观察是否被改回）
- 以该 pty 为控制终端、或持有该 pty fd 的任何进程（含 init 收养的孤儿）
- 进程的 pid/ppid/pgid/命令行/fd0-2 指向/存活起止

用法：pty-exit-probe.py <agent_dir> <argv...>
输出：一行 JSON。
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

AGENT_DIR = sys.argv[1]
ARGV = sys.argv[2:]
BITS = {
    "echo": termios.ECHO,
    "icanon": termios.ICANON,
    "isig": termios.ISIG,
    "iexten": termios.IEXTEN,
}


def flags_of(fd):
    try:
        attrs = termios.tcgetattr(fd)
    except (OSError, termios.error):
        return None
    lflag = attrs[3]
    return {name: bool(lflag & bit) for name, bit in BITS.items()}


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


def proc_info(pid):
    try:
        with open(f"/proc/{pid}/stat", "r", encoding="utf8", errors="replace") as handle:
            raw = handle.read()
        rest = raw[raw.rfind(")") + 2 :].split()
        ppid, pgrp = int(rest[1]), int(rest[2])
        with open(f"/proc/{pid}/cmdline", "rb") as handle:
            cmdline = handle.read().decode("utf8", "replace").replace("\0", " ").strip()
        fds = {}
        for fd in ("0", "1", "2"):
            try:
                fds[fd] = os.readlink(f"/proc/{pid}/fd/{fd}")
            except OSError:
                fds[fd] = None
        return {"pid": pid, "ppid": ppid, "pgid": pgrp, "cmdline": cmdline[:160], "fds": fds}
    except (OSError, IndexError, ValueError):
        return None


def pty_device(pts_name):
    try:
        return os.stat(pts_name).st_rdev
    except OSError:
        return None


def controlling_tty_holders(dev):
    """以该 pty 为**控制终端**的进程（orphan 也可能留着 tty_nr）。"""
    if dev is None:
        return []
    out = []
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            with open(f"/proc/{entry}/stat", "r", encoding="utf8", errors="replace") as handle:
                raw = handle.read()
        except OSError:
            continue
        rest = raw[raw.rfind(")") + 2 :].split()
        if len(rest) < 5:
            continue
        try:
            tty_nr = int(rest[4])
        except ValueError:
            continue
        if tty_nr == dev:
            info = proc_info(int(entry))
            if info is not None:
                out.append(info)
    return out


def pty_holders(pts_name):
    """所有 fd0/1/2 指向该 pty 的进程（含孤儿）。"""
    out = []
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        info = proc_info(int(entry))
        if info is None:
            continue
        if any(value == pts_name for value in info["fds"].values() if value):
            out.append(info)
    return out


def main():
    env = dict(os.environ, PI_CODING_AGENT_DIR=AGENT_DIR, TERM="xterm-256color")
    work = "/tmp/f40work"
    os.makedirs(work, exist_ok=True)
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
    pts_name = os.ttyname(slave)
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
    sent_exit = False
    exit_at = None
    last_write = 0.0
    start = time.time()
    while time.time() - start < 40:
        if not sent_exit and (b"(auto)" in out or b"Warning: No models" in out):
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
                out.extend(chunk)
                last_write = time.time()
                if not answered and b"\x1b[c" in out:
                    answered = True
                    try:
                        os.write(master, b"\x1b[?1;2c\x1b[?5u")
                    except OSError:
                        pass
                continue
        if sent_exit:
            wpid, _status = os.waitpid(pid, os.WNOHANG)
            if wpid == pid:
                exit_at = time.time()
                break

    if exit_at is None:
        try:
            os.kill(pid, 9)
            os.waitpid(pid, 0)
        except OSError:
            pass
        exit_at = time.time()

    t0 = exit_at
    flags_at_exit = flags_of(slave)
    # 模拟真实 shell：立刻接管 tty（raw），此后任何改动都视为"有进程回写 termios"。
    set_raw(slave)
    baseline = flags_of(slave)

    samples = []
    seen = {}
    changed_at = None
    echo_seen = False
    dev = pty_device(pts_name)
    raw_seen_at = None
    # 前 500ms 用 2ms 粒度（抓"改回 cooked 又改回 raw"这类短窗口），之后 10ms。
    plan = [i * 0.002 for i in range(250)] + [0.5 + i * 0.01 for i in range(101)]
    for step, target in enumerate(plan):
        while time.time() - t0 < target:
            time.sleep(0.001)
        flags = flags_of(slave)
        if flags is not None and flags != baseline and changed_at is None:
            changed_at = round(target, 3)
        if flags is not None and any(flags.values()) and raw_seen_at is None and target < 0.5:
            raw_seen_at = round(target, 3)
        current = pty_holders(pts_name) + [
            info for info in controlling_tty_holders(dev)
            if info["pid"] not in {item["pid"] for item in pty_holders(pts_name)}
        ]
        for info in current:
            seen.setdefault(info["pid"], {"info": info, "first": round(target, 2), "last": round(target, 2)})
            seen[info["pid"]]["last"] = round(target, 2)
        if step % 10 == 0:
            samples.append(
                {
                    "t": round(target, 2),
                    "flags": flags,
                    "pids": sorted(info["pid"] for info in current),
                }
            )
        # 每 200ms 往 pty 写一次 ESC[A，检查是否被字面回显（说明 tty 已回到 cooked / 有人接管）
        if step % 20 == 0 and step > 0:
            try:
                os.write(master, b"\x1b[A")
            except OSError:
                pass
            r, _, _ = select.select([master], [], [], 0.02)
            if r:
                try:
                    data = os.read(master, 4096)
                except OSError:
                    data = b""
                if b"^[[A" in data or b"\x1b[A" == data:
                    echo_seen = True

    print(
        json.dumps(
            {
                "argv": ARGV,
                "pts": pts_name,
                "exit_after_last_write_ms": round((t0 - last_write) * 1000, 1) if last_write else None,
                "flags_at_exit": flags_at_exit,
                "baseline_after_shell_raw": baseline,
                "flags_changed_at": changed_at,
                "processes": [
                    {
                        "pid": item["info"]["pid"],
                        "ppid": item["info"]["ppid"],
                        "pgid": item["info"]["pgid"],
                        "cmdline": item["info"]["cmdline"],
                        "fds": item["info"]["fds"],
                        "alive_from": item["first"],
                        "alive_to": item["last"],
                    }
                    for item in seen.values()
                ],
                "cooked_echo_seen": echo_seen,
                "flags_differ_from_baseline_ever": changed_at is not None,
                "extension_labels": [
                    line.strip()
                    for line in out.decode("utf8", "replace").splitlines()
                    if line.strip().startswith(("context-gateway", "pi-", "web-"))
                ][:4],
                "screen_tail": out.decode("utf8", "replace")[-600:],
                "samples": samples[:8],
            },
            ensure_ascii=False,
        )
    )
    try:
        for item in seen.values():
            os.kill(item["info"]["pid"], 9)
    except OSError:
        pass
    os.close(master)
    os.close(slave)
    return 0


if __name__ == "__main__":
    sys.exit(main())
