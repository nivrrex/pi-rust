#!/usr/bin/env python3
"""F53/T4：逐步比较两侧的 `step-N.txt`（同一 realcfg、会话、宽度、按键序列）。

判据只看**屏幕可见文本**（已去 ANSI），逐步输出 `same` / `different` 与首个差异行；
任何一步 different 即视为不等于 ref 行为。

用法：
    compare-steps.py --left DIR --right DIR [--steps N]
"""
import argparse
import os


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
    parser.add_argument("--left", required=True)
    parser.add_argument("--right", required=True)
    parser.add_argument("--steps", type=int, default=None)
    parser.add_argument("--label", default="")
    args = parser.parse_args()

    left = read_steps(args.left)
    right = read_steps(args.right)
    steps = sorted(set(left) & set(right))
    if args.steps is not None:
        steps = [step for step in steps if step <= args.steps]
    if not steps:
        print(f"compare-steps: {args.label} 没有可比较的步（left={sorted(left)} right={sorted(right)}）")
        return 2

    different = 0
    for step in steps:
        lhs = [line.rstrip() for line in left[step]]
        rhs = [line.rstrip() for line in right[step]]
        if lhs == rhs:
            print(f"step-{step} same")
            continue
        different += 1
        print(f"step-{step} different")
        for index in range(max(len(lhs), len(rhs))):
            a = lhs[index] if index < len(lhs) else "<缺行>"
            b = rhs[index] if index < len(rhs) else "<缺行>"
            if a != b:
                print(f"  行 {index}: ref={a!r}")
                print(f"           ours={b!r}")
                break
    print(f"compare-steps: {args.label} steps={len(steps)} different={different}")
    return 1 if different else 0


if __name__ == "__main__":
    raise SystemExit(main())
