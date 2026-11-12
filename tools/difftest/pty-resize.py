#!/usr/bin/env python3
"""PTY + SIGWINCH 辅助工具（F54/Z3）。

与 `pty-helper.py` **并列**的独立工具（不改 `pty-helper.py`，v1/v2 语料的时序不受影响）。
差异只有两点：

1. 脚本步骤里可写 `{"resize": {"cols": 60, "rows": 30}}`：改 `TIOCSWINSZ` 并向子进程**发
   `SIGWINCH`**（ref 走 `process.stdout.on("resize")`，`packages/tui/src/terminal.ts:174-191`；
   Node 的 `resize` 事件由 SIGWINCH 触发）。
2. `mark` 步骤的偏移额外记录当时窗口的 `cols`/`rows`，便于把"缩放前后"的屏幕分开比较。

用法：
    python3 pty-resize.py --cols 110 --rows 40 --out /tmp/raw.bin \
        --exit-code-file /tmp/code.txt --marks /tmp/marks.jsonl \
        --script /tmp/keys.json -- <cmd> [args...]

`--script` 是 JSON 数组，元素与 `pty-helper.py` 同构（`waitFor` / `waitStrip` / `waitStable` /
`delayMs` / `mark` / `text` / `raw`），另加 `resize`。

**假定已在网络命名空间内**（由调用方用 `unshare -n` 包一层）；本工具自身不做 unshare。
退出码：0 表示 helper 本身成功（被测进程退出码写在 `--exit-code-file`）；非 0 表示 helper 错误。
"""

import argparse
import errno
import fcntl
import hashlib
import json
import os
import pty
import select
import signal
import struct
import sys
import termios
import time

# 与 `pty-helper.py` 同口径的转义剥离（`waitStrip` 用；屏幕级子串匹配）。
import re

ANSI_PATTERN = re.compile(
    rb"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b\[[0-9;?]*[A-Za-z]|\x1b[=>]|\x1b\[[0-9;?]*[a-z]"
)

# 终端查询的可选应答（与 `pty-helper.py` 的 QUERY_REPLIES 保持一致；缩放手写脚本同样需要）。
QUERY_REPLIES = [
    (b"\x1b[c", b"\x1b[?62;c"),
    (b"\x1b[>7u", b"\x1b[>7;1u"),
    (b"\x1b[?u", b"\x1b[?0u"),
    (b"\x1b]11;?", b"\x1b]11;rgb:0000/0000/0000\x07"),
    (b"\x1b[?996n", b"\x1b[?996;0n"),
    (b"\x1b[?2031$p", b"\x1b[?2031;0$y"),
]


def strip_escapes(raw: bytes) -> str:
    return ANSI_PATTERN.sub(b"", raw).decode("utf-8", errors="replace")


def set_win_size(fd: int, cols: int, rows: int) -> None:
    # struct winsize { unsigned short ws_row, ws_col, ws_xpixel, ws_ypixel; }
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


def answer_queries(fd: int, chunk: bytes) -> None:
    """对 chunk 里出现过的终端查询逐条作答（同 `pty-helper.py` 的行为）。"""
    for query, reply in QUERY_REPLIES:
        if query in chunk:
            try:
                os.write(fd, reply)
            except OSError:
                return


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--cols", type=int, required=True)
    parser.add_argument("--rows", type=int, required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--exit-code-file", required=True)
    parser.add_argument("--script")
    parser.add_argument("--marks")
    parser.add_argument("--diag")
    parser.add_argument("cmd", nargs=argparse.REMAINDER)
    args = parser.parse_args()

    cmd = args.cmd
    if cmd and cmd[0] == "--":
        cmd = cmd[1:]
    if not cmd:
        print("pty-resize: 缺少被测命令", file=sys.stderr)
        return 2

    script = []
    if args.script:
        with open(args.script, encoding="utf-8") as handle:
            script = json.load(handle)

    diag = None
    if args.diag:
        diag = open(args.diag, "a", encoding="utf-8")

    def log(message: str) -> None:
        if diag is not None:
            diag.write(message + "\n")
            diag.flush()

    pid, fd = pty.fork()
    if pid == 0:  # 子进程
        os.execvpe(cmd[0], cmd, os.environ)
        os._exit(127)

    set_win_size(fd, args.cols, args.rows)
    log(f"master fd={fd} child_pid={pid} cols={args.cols} rows={args.rows}")

    marks_handle = None
    if args.marks:
        marks_handle = open(args.marks, "w", encoding="utf-8")
    raw = bytearray()
    started = time.monotonic()
    out_handle = open(args.out, "wb")
    current_cols, current_rows = args.cols, args.rows

    def drain(timeout: float) -> None:
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
                if error.errno == errno.EIO:
                    return
                raise
            if not chunk:
                return
            raw.extend(chunk)
            out_handle.write(chunk)
            out_handle.flush()
            answer_queries(fd, chunk)

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
                    print(f"pty-resize: waitFor 超时（{needle!r} 未出现）", file=sys.stderr)
                    os.kill(pid, signal.SIGKILL)
                    os.waitpid(pid, 0)
                    return 3
                drain(0.05)
        if "waitStrip" in step:
            needle = step["waitStrip"]["contains"]
            timeout = step["waitStrip"].get("timeoutMs", 10000) / 1000.0
            deadline = time.monotonic() + timeout
            while needle not in strip_escapes(bytes(raw)):
                if time.monotonic() > deadline:
                    log(f"step[{step_index}] waitStrip 超时: {needle!r}")
                    out_handle.flush()
                    out_handle.close()
                    print(f"pty-resize: waitStrip 超时（{needle!r} 未出现）", file=sys.stderr)
                    os.kill(pid, signal.SIGKILL)
                    os.waitpid(pid, 0)
                    return 3
                drain(0.05)
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
        if "delayMs" in step:
            end = time.monotonic() + step["delayMs"] / 1000.0
            while time.monotonic() < end:
                drain(0.05)
        if "resize" in step:
            # 先改窗口尺寸，再发 SIGWINCH（顺序与真实终端一致：内核先更新 winsize，
            # 再给前台进程组投递 SIGWINCH）。
            current_cols = int(step["resize"].get("cols", current_cols))
            current_rows = int(step["resize"].get("rows", current_rows))
            set_win_size(fd, current_cols, current_rows)
            try:
                os.kill(pid, signal.SIGWINCH)
            except OSError as error:
                log(f"step[{step_index}] SIGWINCH 发送失败: {error}")
            log(f"step[{step_index}] resize → {current_cols}x{current_rows}")
        if "mark" in step:
            drain(0.05)
            if marks_handle is not None:
                marks_handle.write(
                    json.dumps(
                        {
                            "name": step["mark"],
                            "offset": len(raw),
                            "t": time.monotonic(),
                            "cols": current_cols,
                            "rows": current_rows,
                        },
                        ensure_ascii=False,
                    )
                    + "\n"
                )
                marks_handle.flush()
                os.fsync(marks_handle.fileno())
            log(f"step[{step_index}] mark={step['mark']!r} offset={len(raw)}")
        payload = step.get("text", step.get("raw"))
        if payload is not None:
            data = payload.encode()
            try:
                os.write(fd, data)
            except OSError as error:
                log(f"step[{step_index}] write errno={error.errno} error={error}")

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
    out_handle.flush()
    out_handle.close()
    with open(args.exit_code_file, "w", encoding="utf-8") as handle:
        handle.write(str(code))
    return 0


if __name__ == "__main__":
    sys.exit(main())
