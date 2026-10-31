#!/usr/bin/env python3
"""P7-8b/C：`/tree` 搜索框 + 中文输入（含分片 UTF-8 与退格）不得崩溃。

真机报告过"tree + 中文输入法"崩溃；这里在 pty 里复现该路径并断言：
- 进程退出码 0；
- `$PI_CODING_AGENT_DIR/crash.log` **不存在**；
- 屏幕最终仍显示树面板（没有退回 shell）。

用法：unshare -n sh -c 'ip link set lo up; python3 tools/difftest/pty_tree_cjk_check.py'
"""
import fcntl, json, os, pty, re, select, shutil, struct, sys, termios, time

REPO = str(__import__("pathlib").Path(__file__).resolve().parents[2])
# 允许用 `PI_RS_BIN` 指定被测二进制（P8a 用它跑 musl 静态构建）。
BINARY = os.environ.get("PI_RS_BIN", f"{REPO}/target/debug/pi-rs")
BASE = "/tmp/p78b-tree-cjk"
CFG, SESS, HOME, WORK = (os.path.join(BASE, name) for name in ("cfg", "sess", "home", "work"))


def prepare() -> None:
    for path in (CFG, SESS, HOME, WORK):
        shutil.rmtree(path, ignore_errors=True)
        os.makedirs(path, exist_ok=True)
    with open(os.path.join(CFG, "models.json"), "w") as handle:
        json.dump({"providers": {"mock": {"baseUrl": os.environ.get("MOCK_BASE", "http://127.0.0.1:9/v1"),
                                          "api": "openai-completions", "apiKey": "k",
                                          "models": [{"id": "mock-model"}]}}}, handle)
    with open(os.path.join(CFG, "settings.json"), "w") as handle:
        json.dump({"defaultProvider": "mock", "defaultModel": "mock-model"}, handle)
    with open(os.path.join(CFG, "auth.json"), "w") as handle:
        json.dump({"mock": {"type": "api_key", "key": "k"}}, handle)


def main() -> int:
    prepare()
    env = dict(os.environ, HOME=HOME, PI_CODING_AGENT_DIR=CFG, PI_CODING_AGENT_SESSION_DIR=SESS)
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(WORK)
        os.execvpe(BINARY, ["pi-rs"], env)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
    out = b""

    def drain(seconds: float) -> None:
        nonlocal out
        end = time.time() + seconds
        while time.time() < end:
            if select.select([fd], [], [], 0.03)[0]:
                try:
                    chunk = os.read(fd, 1 << 16)
                except OSError:
                    return
                if not chunk:
                    return
                out += chunk
                end = time.time() + 0.15

    drain(6.0)
    # kitty 激活应答（真机会先问）
    os.write(fd, b"\x1b[?1;1u\x1b[?62;c")
    drain(0.5)
    os.write(fd, b"/tree\r")
    drain(2.0)
    # 中文逐字（kitty press/release 成对）+ 分片 UTF-8（一个字符拆两次写）
    for ch in "中文検索":
        code = ord(ch)
        os.write(fd, f"\x1b[{code};1u\x1b[{code};1:3u".encode())
        time.sleep(0.03)
    drain(0.8)
    fragment = "测试".encode()
    os.write(fd, fragment[:2])
    time.sleep(0.2)
    os.write(fd, fragment[2:])
    drain(0.8)
    # 退格 6 次（逐字符删，含 CJK）
    for _ in range(6):
        os.write(fd, b"\x7f")
        time.sleep(0.05)
    drain(1.0)
    # 再输入 + 上下移动
    for ch in "树":
        os.write(fd, f"\x1b[{ord(ch)};1u".encode())
    os.write(fd, b"\x1b[A\x1b[B")
    drain(1.0)
    os.write(fd, b"\x1b")
    drain(0.5)
    os.write(fd, b"\x15\x04")
    drain(0.8)
    try:
        os.close(fd)
    except OSError:
        pass
    status = os.waitpid(pid, 0)[1]
    text = re.sub(rb"\x1b\[[0-9;?]*[ -/]*[@-~]", b"", out).decode("utf-8", "replace")
    crash = os.path.exists(os.path.join(CFG, "crash.log"))
    saw_tree = "Session Tree" in text
    print(json.dumps({"exit_code": status >> 8, "crash_log": crash, "tree_panel_seen": saw_tree}))
    return 0 if (status >> 8 == 0 and not crash and saw_tree) else 1


if __name__ == "__main__":
    sys.exit(main())
