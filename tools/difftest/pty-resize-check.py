#!/usr/bin/env python3
"""P7-6ai/M1：**缩放端到端验证**——自建 pty 启动 r3 pi-rs，改 TIOCSWINSZ 并发 SIGWINCH，
断言之后 ≤1 s 内出现按**新 cols** 渲染的帧。

判据：新宽度下，整宽边框行（`─`×cols）出现在输出里。
"""
import fcntl, json, os, pty, re, select, signal, struct, sys, tempfile, termios, time

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BIN = os.path.join(REPO, "target/debug/pi-rs")
COLS1, ROWS1, COLS2 = 100, 30, 60


def set_size(fd, cols, rows):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


def read_until(fd, needle: bytes, deadline: float, sink: bytearray) -> bool:
    while time.time() < deadline:
        ready, _, _ = select.select([fd], [], [], 0.1)
        if not ready:
            continue
        try:
            chunk = os.read(fd, 65536)
        except OSError:
            return False
        if not chunk:
            return False
        sink += chunk
        if needle in sink:
            return True
    return False


def main() -> int:
    if not os.path.exists(BIN):
        print(json.dumps({"ok": False, "reason": f"缺少二进制 {BIN}"}))
        return 1
    tmp = tempfile.mkdtemp(prefix="m1-")
    env = dict(os.environ)
    env.update({
        "PI_TUI_RENDERER": "r3",
        "PI_CODING_AGENT_DIR": os.path.join(tmp, "cfg"),
        "PI_CODING_AGENT_SESSION_DIR": os.path.join(tmp, "sess"),
        "HOME": os.path.join(tmp, "home"),
        "TERM": "xterm-256color",
    })
    for d in ("cfg", "sess", "home"):
        os.makedirs(os.path.join(tmp, d), exist_ok=True)
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(tmp)
        os.execve(BIN, [BIN], env)
    set_size(fd, COLS1, ROWS1)
    sink = bytearray()
    # 等首帧（出现 100 宽的整宽边框或任意内容）
    read_until(fd, b"\x1b[?25l", time.time() + 5, sink)
    time.sleep(0.6)
    first_len = len(sink)
    sink.clear()
    # 改尺寸 + SIGWINCH
    set_size(fd, COLS2, ROWS1)
    os.kill(pid, signal.SIGWINCH)
    wide = ("─" * COLS2).encode()
    ok = read_until(fd, wide, time.time() + 1.0, sink)
    os.kill(pid, signal.SIGKILL)
    try:
        os.waitpid(pid, 0)
    except ChildProcessError:
        pass
    print(json.dumps({
        "ok": bool(ok),
        "cols_before": COLS1,
        "cols_after": COLS2,
        "bytes_first_frame": first_len,
        "bytes_after_resize": len(sink),
        "found_60wide_border": bool(ok),
    }))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
