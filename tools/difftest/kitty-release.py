#!/usr/bin/env python3
"""F3：复现"kitty 抬起事件迟到 ⇒ 退出后 shell 读到 `0;5:3u`"。

在一个**交互 sh** 里启动被测程序，模拟 kitty 终端（应答 `\\x1b[?u` → `\\x1b[?<flags>u`），
发送 kitty 编码的 Ctrl+D（按下 `\\x1b[100;5u`），等 `--delay-ms` 后再发抬起 `\\x1b[100;5:3u`，
然后看 shell 侧读到什么。

输出（最后一行，供测试断言）：`leak=<n> hex=<hex>`
  leak>0 表示 shell 侧收到了残留字节（即缺陷存在）。

用法：kitty-release.py --bin <path> --agent-dir <dir> [--delay-ms 60] [--cols 100] [--rows 30]
"""
import argparse
import fcntl
import os
import pty
import select
import signal
import struct
import sys
import termios
import time


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", required=True)
    ap.add_argument("--agent-dir", required=True)
    ap.add_argument("--delay-ms", type=float, default=60.0)
    ap.add_argument(
        "--force-release",
        action="store_true",
        help="即使应用没有请求事件类型上报（flags 位2）也发抬起事件（模拟不听话的终端）",
    )

    ap.add_argument(
        "--action",
        default="ctrl-d",
        choices=["ctrl-d", "ctrl-c2", "exit-cmd", "sigterm", "sighup", "suspend", "negotiating"],
        help="转折点：退出/挂起/协商期注入按键（F4 矩阵）",
    )
    args = ap.parse_args()

    pid_holder = [0]
    pid, fd = pty.fork()
    if pid == 0:

        env = dict(os.environ)
        env["PS1"] = "SHELLREADY> "
        env["TERM"] = "xterm-256color"
        env["PI_CODING_AGENT_DIR"] = args.agent_dir
        env["PI_CODING_AGENT_SESSION_DIR"] = os.path.join(args.agent_dir, "sess")
        env["PIRS"] = args.bin
        os.execvpe("sh", ["sh", "-i"], env)

    pid_holder[0] = pid
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
    os.set_blocking(fd, False)
    buf = bytearray()
    # 终端对 kitty push 的**真实**响应：只有应用请求了位2（上报事件类型）才发抬起事件。
    state = {"flags": 0}

    def pump(seconds: float) -> None:
        end = time.time() + seconds
        while time.time() < end:
            ready, _, _ = select.select([fd], [], [], 0.01)
            if not ready:
                continue
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                return
            if not chunk:
                return
            buf.extend(chunk)
            import re

            push = re.search(rb"\x1b\[>(\d+)u", chunk)
            if push:
                state["flags"] = int(push.group(1))
            if b"\x1b[?u" in chunk:
                os.write(fd, f"\x1b[?{state['flags'] or 7}u".encode())  # 支持，按应用请求的位
            if b"\x1b[c" in chunk:
                os.write(fd, b"\x1b[?62;c")

    pump(1.0)
    if args.action == "negotiating":
        # 协商期间按键：查询刚发出（应答未到）就注入 press，应答随后到达。
        os.write(fd, b"$PIRS --no-session\r")
        pump(0.05)
        mark = len(buf)
        os.write(fd, b"\x1b[100;5u")
        if args.delay_ms > 0:
            time.sleep(args.delay_ms / 1000.0)
        if state["flags"] & 2 or args.force_release:
            os.write(fd, b"\x1b[100;5:3u")
        pump(2.5)
    else:
        os.write(fd, b"$PIRS --no-session\r")
        pump(2.0)
        mark = len(buf)
        if args.action == "ctrl-d":
            os.write(fd, b"\x1b[100;5u")
            if args.delay_ms > 0:
                time.sleep(args.delay_ms / 1000.0)
            if state["flags"] & 2 or args.force_release:
                os.write(fd, b"\x1b[100;5:3u")
        elif args.action == "ctrl-c2":
            os.write(fd, b"\x1b[99;5u")  # Ctrl+C（kitty 编码）
            pump(0.15)
            if state["flags"] & 2 or args.force_release:
                os.write(fd, b"\x1b[99;5:3u")
            pump(0.15)
            os.write(fd, b"\x1b[99;5u")
            if args.delay_ms > 0:
                time.sleep(args.delay_ms / 1000.0)
            if state["flags"] & 2 or args.force_release:
                os.write(fd, b"\x1b[99;5:3u")
        elif args.action == "exit-cmd":
            os.write(fd, b"/quit\r")
            if args.delay_ms > 0:
                time.sleep(args.delay_ms / 1000.0)
            if state["flags"] & 2 or args.force_release:
                os.write(fd, b"\x1b[100;5:3u")
        elif args.action in ("sigterm", "sighup"):
            os.write(fd, b"\x1b[100;5u")
            time.sleep(0.05)
            if state["flags"] & 2 or args.force_release:
                os.write(fd, b"\x1b[100;5:3u")
            sig = 15 if args.action == "sigterm" else 1
            try:
                os.kill(pid_holder[0], sig)
            except (OSError, IndexError):
                pass
            if args.delay_ms > 0:
                time.sleep(args.delay_ms / 1000.0)
            if state["flags"] & 2 or args.force_release:
                os.write(fd, b"\x1b[100;5:3u")
        elif args.action == "suspend":
            os.write(fd, b"\x1b[122;5u")  # Ctrl+Z
            pump(0.6)
            if args.delay_ms > 0:
                time.sleep(args.delay_ms / 1000.0)
            if state["flags"] & 2 or args.force_release:
                os.write(fd, b"\x1b[122;5:3u")
            pump(0.4)
            os.write(fd, b"fg\r")
        pump(1.5)

    post = bytes(buf)[mark:]
    prompt = b"SHELLREADY> "
    index = post.rfind(prompt)
    after = post[index + len(prompt):] if index >= 0 else post[-80:]
    print(f"leak={len(after)} hex={after[:80].hex()}")
    try:
        os.write(fd, b"exit\r")
    except OSError:
        pass
    # 收尾：按 `PI_CODING_AGENT_DIR` 精确回收本次沙箱里还在跑的 pi-rs。
    # 不回收会留孤儿进程——实测 40 个残留把 load 顶到 44，让无关的并发测试超时失败。
    cleanup_sandbox(args.agent_dir)
    return 0


def cleanup_sandbox(agent_dir: str) -> None:
    marker = f"PI_CODING_AGENT_DIR={agent_dir}".encode()
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            with open(f"/proc/{entry}/environ", "rb") as handle:
                environ = handle.read()
        except OSError:
            continue
        if marker in environ:
            try:
                os.kill(int(entry), signal.SIGKILL)
            except OSError:
                pass


if __name__ == "__main__":
    sys.exit(main())
