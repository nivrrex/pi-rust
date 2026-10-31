#!/usr/bin/env python3
"""PTY 辅助工具（P7-2 W2）。

在**伪终端**里启动一个被测进程，设置固定窗口尺寸，按脚本注入按键（含时序），
把被测进程的 **真实退出码** 单独写到一个文件（不经管道 ⇒ 不被掩盖），
并把原始字节流原样落到输出文件。

用法：
    python3 pty-helper.py --cols 100 --rows 30 --exit-code-file /tmp/code.txt \
        --out /tmp/raw.bin --script /tmp/keys.json -- <cmd> [args...]

`--script` 是 JSON 数组，元素为：
    {"delayMs": 200, "text": "say hi\\r"}     # 延时后注入文本
    {"delayMs": 200, "raw": "\\u001b[A"}      # 同 text，但按原样（便于写转义）
    {"waitFor": {"contains": "> ", "timeoutMs": 10000}}   # 等到原始输出出现子串
    {"waitStable": {"quietMs": 300, "timeoutMs": 10000}}  # 等到 quietMs 内没有新输出

**假定已在网络命名空间内**（由调用方用 `unshare -n` 包一层）；本工具自身不做 unshare。
退出码：0 表示 helper 本身成功（被测进程退出码写在 --exit-code-file）；非 0 表示 helper 错误。
"""

import argparse
import errno
import hashlib
import fcntl
import json
import os
import re
import pty
import select
import signal
import struct
import sys
import termios
import time


ANSI_PATTERN = re.compile(rb"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b\[[0-9;?]*[A-Za-z]|\x1b[=>]|\x1b\[[0-9;?]*[a-z]")


def strip_escapes(raw: bytes) -> str:
    """剥掉 CSI/OSC 等转义序列，按 UTF-8 有损解码，供 `waitStrip` 做"屏幕级"子串匹配。"""
    return ANSI_PATTERN.sub(b"", raw).decode("utf-8", errors="replace")


def set_win_size(fd: int, cols: int, rows: int) -> None:
    # struct winsize { unsigned short ws_row, ws_col, ws_xpixel, ws_ypixel; }
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


# 终端查询的**可选应答**（P7-2 W3）：TS 的 TUI 会发 `\x1b[c`（设备属性）、`\x1b[>7u`（kitty 键盘）、
# `\x1b[?u`（kitty 支持查询）、`\x1b]11;?`（背景色）等并等待回复；不答可能导致渲染时序/内容异常。
QUERY_REPLIES = [
    (b"\x1b[c", b"\x1b[?62;c"),
    (b"\x1b[>7u", b"\x1b[>7;1u"),
    (b"\x1b[?u", b"\x1b[?0u"),
    (b"\x1b]11;?\x07", b"\x1b]11;rgb:0000/0000/0000\x07"),
    (b"\x1b]10;?\x07", b"\x1b]10;rgb:ffff/ffff/ffff\x07"),
]


def answer_queries(fd: int, chunk: bytes) -> None:
    for query, reply in QUERY_REPLIES:
        if query in chunk:
            try:
                os.write(fd, reply)
            except OSError:
                return


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--cols", type=int, default=100)
    ap.add_argument("--rows", type=int, default=30)
    ap.add_argument("--out", required=True, help="原始字节流输出文件")
    ap.add_argument("--exit-code-file", required=True, help="被测进程真实退出码写入此文件")
    ap.add_argument("--script", help="按键/等待脚本 JSON")
    ap.add_argument("--marks", help="（P7-4/W0e）把每个 mark 步骤的当前原始流字节偏移写成本文件的一行 JSON")
    ap.add_argument("--snapshot-dir", help="（P7-3）每步后把当前原始流写 <dir>/<n>.bin，便于实证二分")
    ap.add_argument("--termios-after", help="（P7-3/W4）子进程退出后把 master 的 termios 关键标志写此文件（JSON）")
    ap.add_argument("--signal-after-ms", type=int, help="（P7-3/W4）启动后延时多少毫秒向子进程发信号（配合 --signal）")
    ap.add_argument("--signal", default="INT", help="（P7-3/W4）信号名：INT 或 TERM")
    ap.add_argument("--diag", help="（P7-3/W4）诊断日志：写入返回值/errno、master 状态、子进程 isatty")
    ap.add_argument("cmd", nargs=argparse.REMAINDER, help="-- 之后的被测命令")
    args = ap.parse_args()

    cmd = args.cmd
    if cmd and cmd[0] == "--":
        cmd = cmd[1:]
    if not cmd:
        print("pty-helper: 缺少被测命令", file=sys.stderr)
        return 2

    script = []
    if args.script:
        with open(args.script, "r", encoding="utf-8") as handle:
            script = json.load(handle)

    pid, fd = pty.fork()
    if pid == 0:  # 子进程：成为被测进程
        try:
            os.execvp(cmd[0], cmd)
        except OSError as error:
            print(f"pty-helper: exec 失败: {error}", file=sys.stderr)
            os._exit(127)
        os._exit(127)

    diag = None
    if args.diag:
        diag = open(args.diag, "a", encoding="utf-8")

    def log(message: str) -> None:
        if diag is not None:
            diag.write(message + "\n")
            diag.flush()

    set_win_size(fd, args.cols, args.rows)

    log(f"master isatty={os.isatty(fd)} fd={fd} child_pid={pid}")
    try:
        attrs = termios.tcgetattr(fd)
        log(f"master lflag={attrs[3]:#x} icanon={bool(attrs[3] & termios.ICANON)} echo={bool(attrs[3] & termios.ECHO)}")
    except OSError as error:
        log(f"master tcgetattr 失败: {error}")

    marks_handle = None
    if args.marks:
        marks_handle = open(args.marks, "w", encoding="utf-8")
    raw = bytearray()
    started = time.monotonic()
    # **增量落盘**（P7-3 W4 教训）：若只在收尾写 `--out`，一旦外层 timeout 杀掉就什么诊断都没有。
    # 这里保持一个打开的文件句柄，每读到一段就追加写并 flush。
    out_handle = open(args.out, "wb")

    def drain(timeout: float) -> None:
        """把当前可读的输出尽量读进 raw。"""
        end = time.monotonic() + timeout
        while True:
            remaining = end - time.monotonic()
            if remaining <= 0:
                return
            ready, _, _ = select.select([fd], [], [], remaining)
            if not ready:
                return
            try:
                chunk = os.read(fd, 65536)
            except OSError as error:
                if error.errno == errno.EIO:  # 子进程已关闭 PTY
                    return
                raise
            if not chunk:
                return
            raw.extend(chunk)
            out_handle.write(chunk)
            out_handle.flush()
            answer_queries(fd, chunk)

    # W4：可选的"延时后向子进程发信号"（SIGINT/SIGTERM 的终端恢复自测）——**放在步骤循环之外**，
    # 否则会打乱循环内的块结构（P7-3 实测：写块被吞掉、按键从未发出）。
    if args.signal_after_ms is not None:
        time.sleep(args.signal_after_ms / 1000.0)
        sig = signal.SIGINT if args.signal.upper().startswith("INT") else signal.SIGTERM
        try:
            os.kill(pid, sig)
        except OSError:
            pass

    for step_index, step in enumerate(script):
        log(f"step[{step_index}] keys={sorted(step.keys())}")
        if "waitFor" in step:
            needle = step["waitFor"]["contains"].encode()
            timeout = step["waitFor"].get("timeoutMs", 10000) / 1000.0
            deadline = time.monotonic() + timeout
            while needle not in bytes(raw):
                if time.monotonic() > deadline:
                    log(f"step[{step_index}] waitFor 超时: {needle!r}")
                    out_handle.flush()
                    out_handle.close()
                    print(f"pty-helper: waitFor 超时（{needle!r} 未出现）", file=sys.stderr)
                    os.kill(pid, signal.SIGKILL)
                    os.waitpid(pid, 0)
                    return 3
                drain(0.05)
            log(f"step[{step_index}] waitFor 匹配成功")
        if "waitStrip" in step:
            # **去转义后的文本等待**（P7-4 W1 新增能力）：把 CSI/OSC 等转义序列剥掉再找子串，
            # 因为 TUI 的回复/用量文本在原始流里会被 SGR 与重绘切分，`waitFor` 匹配不到。
            needle = step["waitStrip"]["contains"]
            timeout = step["waitStrip"].get("timeoutMs", 10000) / 1000.0
            deadline = time.monotonic() + timeout
            while needle not in strip_escapes(bytes(raw)):
                if time.monotonic() > deadline:
                    log(f"step[{step_index}] waitStrip 超时: {needle!r}")
                    out_handle.flush()
                    out_handle.close()
                    print(f"pty-helper: waitStrip 超时（{needle!r} 未出现）", file=sys.stderr)
                    os.kill(pid, signal.SIGKILL)
                    os.waitpid(pid, 0)
                    return 3
                drain(0.05)
            log(f"step[{step_index}] waitStrip 匹配成功")
        if "waitStable" in step:
            quiet = step["waitStable"].get("quietMs", 300) / 1000.0
            timeout = step["waitStable"].get("timeoutMs", 10000) / 1000.0
            deadline = time.monotonic() + timeout
            last_digest = hashlib.sha256(bytes(raw)).hexdigest()
            last_change = time.monotonic()
            while time.monotonic() < deadline:
                drain(0.05)
                digest = hashlib.sha256(bytes(raw)).hexdigest()
                if digest != last_digest:
                    last_digest = digest
                    last_change = time.monotonic()
                elif time.monotonic() - last_change >= quiet:
                    break
            log(f"step[{step_index}] waitStable 结束")
        if "delayMs" in step:
            log(f"step[{step_index}] delayMs={step['delayMs']} 开始")
            end = time.monotonic() + step["delayMs"] / 1000.0
            while time.monotonic() < end:
                drain(0.05)
            log(f"step[{step_index}] delayMs 结束")
        if "mark" in step:
            # 记录**此刻**已收到的原始流长度（P7-4/W0e 截断口径）：比较时截到这里，
            # 退出后追加的 resume 提示就不会覆盖会话中的最后两行（cwd、footer）。
            drain(0.05)
            if marks_handle is not None:
                marks_handle.write(json.dumps({"name": step["mark"], "offset": len(raw)}, ensure_ascii=False) + "\n")
                marks_handle.flush()
                os.fsync(marks_handle.fileno())
            log(f"step[{step_index}] mark={step['mark']!r} offset={len(raw)}")
        payload = step.get("text", step.get("raw"))
        if payload is not None:
            data = payload.encode()
            log(f"step[{step_index}] about-to-write len={len(data)} data={data!r}")
            try:
                written = os.write(fd, data)
                log(f"step[{step_index}] write ret={written}")
            except OSError as error:
                log(f"step[{step_index}] write errno={error.errno} error={error}")

    # 收尾：读到子进程退出为止。
    while True:
        drain(0.1)
        done, status = os.waitpid(pid, os.WNOHANG)
        if done == pid:
            code = os.waitstatus_to_exitcode(status)
            break
        if time.monotonic() - started > 300:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
            code = -9
            break
    drain(0.2)

    # W4：回读 master 的 termios 关键标志（断言"退出后回到 cooked/echo"）。
    if args.termios_after:
        flags = {}
        try:
            attrs = termios.tcgetattr(fd)
            lflag = attrs[3]
            flags = {
                "icanon": bool(lflag & termios.ICANON),
                "echo": bool(lflag & termios.ECHO),
                "isig": bool(lflag & termios.ISIG),
            }
        except OSError as error:
            flags = {"error": str(error)}
        with open(args.termios_after, "w", encoding="utf-8") as handle:
            handle.write(json.dumps(flags))

    out_handle.flush()
    out_handle.close()
    with open(args.exit_code_file, "w", encoding="utf-8") as handle:
        handle.write(str(code))
    return 0


if __name__ == "__main__":
    sys.exit(main())
