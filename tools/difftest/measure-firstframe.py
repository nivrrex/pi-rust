#!/usr/bin/env python3
"""F56/D：首帧与总耗时量具（不设固定等待，不设墙钟阈值，只报数）。

为什么另写：`pty-helper.py` 的 `marks.jsonl` 里 `t` 是 mark 时刻的**单调时钟**，
但没有"进程启动时刻"作基准，无法单独算出首帧耗时。这里在**同一进程内**
记录 `t0 = time.monotonic()` 再拉起 helper，于是 `mark.t - t0` 就是"启动到首帧稳定"。

判据（与 pty-helper 一致）：
- 脚本第一步 `waitStable{quietMs,timeoutMs}`：以"输出在 quietMs 内没有新字节"判首帧稳定；
- 稳定后立刻 `mark`（t1），再 `ctrl+d` 退出并记 t2 = 进程退出。
- `first_frame = t1 - t0`，`total = t2 - t0`。

用法：
    python3 tools/difftest/measure-firstframe.py --cmd "<命令含参数>" --out DIR \
        [--cols 110] [--rows 40] [--quiet-ms 1500]

环境：直接继承（调用方负责设 PI_CODING_AGENT_DIR 等）。
"""
import argparse
import json
import os
import subprocess
import sys
import tempfile
import time


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--cmd", required=True, help="被测命令（含参数，按 shlex 切分）")
    parser.add_argument("--out", required=True)
    parser.add_argument("--cols", type=int, default=110)
    parser.add_argument("--rows", type=int, default=40)
    parser.add_argument("--quiet-ms", type=int, default=1500)
    parser.add_argument("--timeout-ms", type=int, default=240000)
    parser.add_argument(
        "--footer-needle",
        default="CH",
        help="页脚特征子串（默认 `CH`：缓存命中率字段，只出现在**恢复后**的状态行；"
        "启动屏的占位行是 `0.0%%/1.0M (auto)`，不含它）",
    )
    args = parser.parse_args()

    repo = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    os.makedirs(args.out, exist_ok=True)
    keys = os.path.join(args.out, "keys.json")
    with open(keys, "w", encoding="utf-8") as handle:
        json.dump(
            [
                # 先等到**页脚出现**（`(auto)` 那一行的特征），再等静止：
                # 大会话（116 MB / 6.4 万条）的转录渲染要好几秒，只看"静止"会在
                # 启动屏刚画完时就判定首帧稳定（实测 1.6 s，明显偏早）。
                # `waitStrip` 是"去转义后的子串"，页脚行在 ANSI 里被切成多段也能匹配。
                {
                    "waitStrip": {
                        "contains": args.footer_needle,
                        "timeoutMs": args.timeout_ms,
                    }
                },
                {"waitStable": {"quietMs": args.quiet_ms, "timeoutMs": args.timeout_ms}},
                {"mark": "first"},
                {"text": "\u0004"},
                {"delayMs": 600},
            ],
            handle,
        )

    import shlex

    cmd = shlex.split(args.cmd)
    helper = [
        sys.executable,
        os.path.join(repo, "tools/difftest/pty-helper.py"),
        "--cols", str(args.cols),
        "--rows", str(args.rows),
        "--out", os.path.join(args.out, "raw.bin"),
        "--exit-code-file", os.path.join(args.out, "code.txt"),
        "--script", keys,
        "--marks", os.path.join(args.out, "marks.jsonl"),
        "--",
    ] + cmd

    t0 = time.monotonic()
    completed = subprocess.run(helper, check=False)
    t2 = time.monotonic()
    if completed.returncode != 0:
        print(f"helper 失败 rc={completed.returncode}", file=sys.stderr)
        return 1

    marks_path = os.path.join(args.out, "marks.jsonl")
    marks = []
    if os.path.exists(marks_path):
        with open(marks_path, encoding="utf-8") as handle:
            marks = [json.loads(line) for line in handle if line.strip()]
    if not marks:
        print("没有 mark（首帧未稳定）", file=sys.stderr)
        return 1
    first = marks[0]["t"] - t0
    total = t2 - t0
    with open(os.path.join(args.out, "timing.json"), "w", encoding="utf-8") as handle:
        json.dump(
            {"first_frame_s": round(first, 3), "total_s": round(total, 3), "marks": len(marks)},
            handle,
        )
    print(f"first_frame={first:.2f}s total={total:.2f}s")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
