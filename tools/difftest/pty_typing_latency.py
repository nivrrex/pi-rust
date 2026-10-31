#!/usr/bin/env python3
"""P7-8a/F：击键→回显 延迟探针（pi-rs 与 TS 同口径）。

用法（**必须在 `unshare -n` 里跑**，见 AGENTS.md）：
  unshare -n sh -c 'ip link set lo up; env PATH=... TS_PI_ENTRY=... \
     python3 tools/difftest/pty_typing_latency.py --impl pi-rs --rate 40 --state idle'

要点：
- 先 `TIOCSWINSZ` 设窗口（否则 cols=0、渲染全空行、测出来的都是假数字）；
- 每个键前把输出**排空**，再用"输出里出现该字符"判定回显，避免把上一帧算进来；
- kitty press/release 成对发送（release 必须被丢弃，不得造成额外帧）。
"""
import argparse, fcntl, json, os, pty, select, shutil, statistics, struct, sys, termios, time

REPO = str(__import__("pathlib").Path(__file__).resolve().parents[2])
CFG = "/tmp/p78a-latency/cfg"
SESS = "/tmp/p78a-latency/sess"
HOME = "/tmp/p78a-latency/home"
WORK = "/tmp/p78a-latency/work"

MIXED = "abc 中文xyz"  # ASCII + CJK 混合


def prepare(sessions: int) -> None:
    for path in (CFG, SESS, HOME, WORK):
        shutil.rmtree(path, ignore_errors=True)
        os.makedirs(path, exist_ok=True)
    cfg = {
        "providers": {
            "mock": {
                "baseUrl": os.environ.get("MOCK_BASE", "http://127.0.0.1:9/v1"),
                "api": "openai-completions",
                "apiKey": "test-key",
                "models": [{"id": "mock-model", "name": "mock-model",
                            "contextWindow": 128000, "maxTokens": 4096}],
            }
        }
    }
    with open(os.path.join(CFG, "models.json"), "w") as handle:
        json.dump(cfg, handle)
    with open(os.path.join(CFG, "settings.json"), "w") as handle:
        json.dump({"defaultProvider": "mock", "defaultModel": "mock-model"}, handle)
    with open(os.path.join(CFG, "auth.json"), "w") as handle:
        json.dump({"mock": {"type": "api_key", "key": "test-key"}}, handle)
    # 200 条会话：每条 1 个 user + 1 个 assistant
    for index in range(sessions):
        with open(os.path.join(SESS, f"2026-01-01T00-00-{index:02d}-000Z_s{index}.jsonl"), "w") as handle:
            handle.write(json.dumps({"type": "session", "version": 1,
                                     "id": f"s{index}", "cwd": WORK, "timestamp": 1767225600000}) + "\n")
            handle.write(json.dumps({"type": "message", "id": f"u{index}", "parentId": None,
                                     "timestamp": 1767225600000, "message": {
                                         "role": "user", "content": f"问题 {index}",
                                         "timestamp": 1767225600000}}) + "\n")


def spawn(impl: str, extra: tuple = ()):
    env = dict(os.environ, HOME=HOME, PI_CODING_AGENT_DIR=CFG,
               PI_CODING_AGENT_SESSION_DIR=SESS)
    # 注意：**不要**设 PI_PACKAGE_DIR 为 cli.js 文件路径——TS 会去读  而崩。
    if impl == "pi-rs":
        argv = [f"{REPO}/target/debug/pi-rs", *extra]
    else:
        argv = [shutil.which("node"), os.environ["TS_PI_ENTRY"], *extra]
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(WORK)
        os.execvpe(argv[0], argv, env)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
    return pid, fd


def drain(fd, quiet: float = 0.05, limit: float = 3.0) -> bytes:
    out, end = b"", time.time() + limit
    while time.time() < end:
        if select.select([fd], [], [], quiet)[0]:
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                return out
            if not chunk:
                return out
            out += chunk
            end = time.time() + limit
        else:
            return out
    return out


def keys_for(ch: str) -> tuple:
    """返回 (press, release)。kitty 协议：`ESC [ <cp> ; 1 u`，release 带 `:3`。"""
    cp = ord(ch)
    press = f"\x1b[{cp};1u".encode()
    release = f"\x1b[{cp};1:3u".encode()
    return press, release


def measure(impl: str, rate: int, state: str, keys: int = 40) -> dict:
    prepare(200 if state == "sessions" else 0)
    extra = ()
    if state == "long":
        prepare(0)
        messages = []
        for index in range(200):
            messages.append({"type": "message", "id": f"m{index}", "parentId": None,
                             "timestamp": 1767225600000 + index, "message": {
                                 "role": "user" if index % 2 == 0 else "assistant",
                                 "content": f"第 {index} 条消息：这是一段用来把历史撑长的中文内容。",
                                 "timestamp": 1767225600000 + index}})
        path = os.path.join(SESS, "2026-01-01T00-00-00-000Z_long.jsonl")
        with open(path, "w") as handle:
            handle.write(json.dumps({"type": "session", "version": 1, "id": "long",
                                     "cwd": WORK, "timestamp": 1767225600000}) + "\n")
            for item in messages:
                handle.write(json.dumps(item, ensure_ascii=False) + "\n")
        extra = ("-c",)
    pid, fd = spawn(impl, extra)
    drain(fd, 0.3, 20.0)
    if state == "streaming":
        os.write(fd, "写一段长回复\r".encode())
        for _ in range(200):  # 等到出现 Working/流式文本，但不等结束
            chunk = drain(fd, 0.05, 0.5)
            if b"Hello" in chunk or b"Working" in chunk:
                break
    latencies, frames, bytes_per_key = [], 0, []
    interval = 1.0 / rate
    cpu0 = os.times()
    for index in range(keys):
        ch = MIXED[index % len(MIXED)]
        press, release = keys_for(ch)
        drain(fd, 0.02, 0.3)
        start = time.time()
        os.write(fd, press + release)
        seen = b""
        while time.time() - start < 5.0:
            if select.select([fd], [], [], 0.005)[0]:
                try:
                    chunk = os.read(fd, 65536)
                except OSError:
                    break
                if not chunk:
                    break
                seen += chunk
                if ch.encode() in seen:
                    break
        latencies.append((time.time() - start) * 1000.0)
        bytes_per_key.append(len(seen))
        frames += 1
        rest = interval - (time.time() - start)
        if rest > 0:
            time.sleep(rest)
    cpu1 = os.times()
    drained = drain(fd, 0.05, 0.5)
    os.write(fd, b"\x04")
    time.sleep(0.4)
    drain(fd, 0.05, 1.0)
    try:
        os.close(fd)
    except OSError:
        pass
    try:
        os.waitpid(pid, 0)
    except ChildProcessError:
        pass
    latencies.sort()
    def pct(p):
        if not latencies:
            return 0.0
        idx = min(len(latencies) - 1, int(round(p / 100.0 * (len(latencies) - 1))))
        return latencies[idx]
    return {
        "impl": impl, "rate": rate, "state": state, "keys": len(latencies),
        "p50": round(pct(50), 2), "p99": round(pct(99), 2),
        "max": round(latencies[-1], 2) if latencies else 0.0,
        "cpu_ms": round(((cpu1.user - cpu0.user) + (cpu1.system - cpu0.system)) * 1000, 1),
        "bytes_p50": int(statistics.median(bytes_per_key)) if bytes_per_key else 0,
        "bytes_max": max(bytes_per_key) if bytes_per_key else 0,
        "series": [round(v, 1) for v in latencies],
        "tail_bytes_after_ctrl_d": len(drained),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--impl", required=True, choices=["pi-rs", "ts"])
    parser.add_argument("--rate", type=int, default=40)
    parser.add_argument("--state", default="idle", choices=["idle", "streaming", "sessions", "long"])
    parser.add_argument("--keys", type=int, default=40)
    args = parser.parse_args()
    print(json.dumps(measure(args.impl, args.rate, args.state, args.keys)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
