#!/usr/bin/env python3
"""F53/T5：对每一步的屏幕文本做**不变量断言**（两侧都查）。

检查项（与任务卡一致）：
1. 每行显示宽度 ≤ cols（CJK 记 2 列）；
2. 不含裸 CR / TAB / 其它控制字符（ANSI 已在 dump 时剥掉）；
3. 页脚：含 git 分支名与 thinking 档位（可参数化；`--expect-branch` / `--expect-thinking`）；
4. 内容保全：`--contains` 给出的子串必须**完整出现**在某一行里
   （用于 H：展开态 bash 头行必须完整保留 `(timeout 600s)` 与长路径，不能截断）。

用法：
    assert-steps.py --dir DIR --cols 110 [--expect-branch recover]
        [--expect-thinking high] [--contains "(timeout 600s)"]... [--steps N]
"""
import argparse
import os
import sys
import unicodedata


def display_width(text):
    width = 0
    for char in text:
        if unicodedata.combining(char):
            continue
        width += 2 if unicodedata.east_asian_width(char) in ("W", "F") else 1
    return width


def read_steps(directory):
    steps = {}
    for name in sorted(os.listdir(directory)):
        if not (name.startswith("step-") and name.endswith(".txt")):
            continue
        index = int(name[len("step-"):-len(".txt")])
        with open(os.path.join(directory, name), encoding="utf-8") as handle:
            steps[index] = handle.read().split("\n")
    return steps


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--dir", required=True)
    parser.add_argument("--cols", type=int, required=True)
    parser.add_argument("--expect-branch", default=None)
    parser.add_argument("--expect-thinking", default=None)
    parser.add_argument("--contains", action="append", default=[])
    parser.add_argument("--steps", type=int, default=None)
    args = parser.parse_args()

    steps = read_steps(args.dir)
    if not steps:
        print(f"assert-steps: {args.dir} 没有 step-*.txt")
        return 2
    failures = 0
    for index in sorted(steps):
        if args.steps is not None and index > args.steps:
            continue
        lines = steps[index]
        for lineno, line in enumerate(lines):
            if display_width(line) > args.cols:
                print(
                    f"[{os.path.basename(args.dir)}] step-{index} 行 {lineno} 超宽 "
                    f"{display_width(line)} > {args.cols}: {line!r}"
                )
                failures += 1
            for char in ("\r", "\t"):
                if char in line:
                    print(f"[{os.path.basename(args.dir)}] step-{index} 行 {lineno} 含裸控制字符: {line!r}")
                    failures += 1
                    break
            if any(ord(char) < 0x20 for char in line):
                print(f"[{os.path.basename(args.dir)}] step-{index} 行 {lineno} 含控制字符: {line!r}")
                failures += 1
        body = "\n".join(lines)
        if args.expect_branch and f"({args.expect_branch})" not in body:
            print(f"[{os.path.basename(args.dir)}] step-{index} 页脚缺分支名 ({args.expect_branch})")
            failures += 1
        if args.expect_thinking:
            wanted = f"• {args.expect_thinking}"
            if wanted not in body:
                print(f"[{os.path.basename(args.dir)}] step-{index} 页脚缺 thinking 档 ({args.expect_thinking})")
                failures += 1
        for needle in args.contains:
            if needle not in body:
                print(f"[{os.path.basename(args.dir)}] step-{index} 内容保全失败，缺 {needle!r}")
                failures += 1
    print(f"assert-steps: {args.dir} steps={len(steps)} failures={failures}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
