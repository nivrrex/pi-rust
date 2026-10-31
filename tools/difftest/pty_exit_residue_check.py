#!/usr/bin/env python3
"""P7-8a/X：退出残留回归守护（用户实测 #4 已消失，这里只做守护）。

做法：退出前**无间隔**灌 200 个键事件（含 kitty press/release 成对）再发 Ctrl+D，
退出后读 slave 端剩余字节，必须为 0。重复 20 次，任一次非 0 即失败。

用法：unshare -n sh -c 'ip link set lo up; python3 tools/difftest/pty_exit_residue_check.py'
"""
import fcntl, json, os, pty, select, shutil, struct, sys, termios, time

REPO = str(__import__("pathlib").Path(__file__).resolve().parents[2])
# 允许用 `PI_RS_BIN` 指定被测二进制（P8a 用它跑 musl 静态构建）。
BINARY = os.environ.get("PI_RS_BIN", f"{REPO}/target/debug/pi-rs")
BASE = "/tmp/p78a-exit"
CFG, SESS, HOME, WORK = (os.path.join(BASE, name) for name in ("cfg", "sess", "home", "work"))
REPEATS = 20


def prepare() -> None:
    for path in (CFG, SESS, HOME, WORK):
        shutil.rmtree(path, ignore_errors=True)
        os.makedirs(path, exist_ok=True)
    with open(os.path.join(CFG, "models.json"), "w") as handle:
        json.dump({"providers": {"mock": {"baseUrl": "http://127.0.0.1:9/v1",
                                          "api": "openai-completions", "apiKey": "k",
                                          "models": [{"id": "mock-model"}]}}}, handle)
    with open(os.path.join(CFG, "settings.json"), "w") as handle:
        json.dump({"defaultProvider": "mock", "defaultModel": "mock-model"}, handle)


def once(index: int) -> int:
    env = dict(os.environ, HOME=HOME, PI_CODING_AGENT_DIR=CFG, PI_CODING_AGENT_SESSION_DIR=SESS)
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(WORK)
        os.execvpe(BINARY, [BINARY], env)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
    # 等启动帧
    deadline = time.time() + 20
    while time.time() < deadline:
        if select.select([fd], [], [], 0.05)[0]:
            try:
                if not os.read(fd, 65536):
                    break
            except OSError:
                break
        else:
            break
    payload = b""
    for step in range(200):
        ch = "abc 中文xyz"[step % 9]
        code = ord(ch)
        payload += f"\x1b[{code};1u\x1b[{code};1:3u".encode()
        payload += "\x1b[1;3u\x1b[1;1:3u".encode()  # 额外一路 release（模拟重复上报）
    # 注意：ref 里 ctrl+d 在输入非空时是 `deleteCharForward`（`keybindings.ts:95` 把
    # `delete` 与 `ctrl+d` 绑到同一动作），**不会清空输入**；要先 `ctrl+u` 清空再 ctrl+d 才退出。
    payload += b"\x15\x04"
    os.write(fd, payload)  # 无间隔一次灌完
    # 一边排空一边等子进程退出；**退出之后**仍能从 slave 读到的字节才算残留。
    deadline = time.time() + 15
    exited = False
    while time.time() < deadline:
        while select.select([fd], [], [], 0.02)[0]:
            try:
                if not os.read(fd, 65536):
                    break
            except OSError:
                break
        done, _status = os.waitpid(pid, os.WNOHANG)
        if done == pid:
            exited = True
            break
    residue = 0
    if not exited:
        # 没退出就是更严重的问题：也记为残留
        residue = -1
        try:
            os.kill(pid, 9)
        except OSError:
            pass
        try:
            os.waitpid(pid, 0)
        except ChildProcessError:
            pass
    else:
        while True:
            if not select.select([fd], [], [], 0.2)[0]:
                break
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                break
            if not chunk:
                break
            residue += len(chunk)
    try:
        os.close(fd)
    except OSError:
        pass
    return residue


def main() -> int:
    prepare()
    bad = []
    for index in range(REPEATS):
        leftover = once(index)
        if leftover != 0:
            bad.append((index, leftover))
    print(json.dumps({"repeats": REPEATS, "leftover_nonzero": bad}))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
