#!/usr/bin/env python3
"""重排 spec/unverified-map.md：按摘要把每行重新指到当前的 `文件:行号`。

为什么需要它：`spec/*.md` 里任何一处增删都会让**后面**的 `【待验证】` 行号漂移，
而 `scripts/check-unverified-map.py` 要求映射表行的 `文件:行号` **正好**指向一条存在的
`【待验证】` 行。手工改行号出过错，所以做成脚本。

匹配方式：按**摘要前缀**（表里第二列是 spec 行文本的截断）。找不到或重复就报错退出，
不猜、不静默丢弃。用法：`python3 scripts/resync-unverified-map.py [--check]`。
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SPEC = REPO / "spec"
MAP = SPEC / "unverified-map.md"
EXCLUDE = {"unverified-map.md", "00-index.md"}
CATEGORIES = ["已可证实", "部分证实", "不适用", "需读码", "需新场景", "待决"]
PREFIX = 18


def pending_items() -> list[tuple[str, int, str]]:
    """`spec/*.md` 里所有 `【待验证】` 行：`(文件名, 行号, 行文本)`。"""
    items: list[tuple[str, int, str]] = []
    for path in sorted(SPEC.glob("*.md")):
        if path.name in EXCLUDE:
            continue
        for number, line in enumerate(path.read_text().splitlines(), 1):
            if "【待验证】" in line:
                items.append((path.name, number, line.strip()))
    return items


def map_rows(lines: list[str]) -> list[tuple[int, dict[str, str]]]:
    """映射表的行：`(行下标, 各列)`。"""
    rows = []
    for index, line in enumerate(lines):
        m = re.match(r"^\|\s*`([^`]+):(\d+)`\s*\|(.*)\|\s*([^|]+?)\s*\|\s*([^|]+?)\s*\|\s*$", line)
        if m:
            rows.append(
                (
                    index,
                    {
                        "file": m.group(1),
                        "line": m.group(2),
                        "summary": m.group(3).strip(),
                        "note": m.group(4).strip(),
                        "category": m.group(5).strip(),
                    },
                )
            )
    return rows


def main() -> int:
    check_only = "--check" in sys.argv
    items = pending_items()
    lines = MAP.read_text().split("\n")
    rows = map_rows(lines)
    used: set[int] = set()
    updates: dict[int, str] = {}
    errors: list[str] = []
    counts = {name: 0 for name in CATEGORIES}
    for index, row in rows:
        if row["category"] not in CATEGORIES:
            errors.append(f"第 {index + 1} 行分类非法：{row['category']!r}")
            continue
        counts[row["category"]] += 1
        key = row["summary"].rstrip("…").strip()[:PREFIX]
        matches = [
            (position, item)
            for position, item in enumerate(items)
            if position not in used and item[2].split("】", 1)[-1].strip().startswith(key)
        ]
        if len(matches) != 1:
            errors.append(
                f"第 {index + 1} 行（摘要前缀 {key!r}）匹配到 {len(matches)} 条待验证项，应为 1 条"
            )
            continue
        position, item = matches[0]
        used.add(position)
        updates[index] = f"| `{item[0]}:{item[1]}` | {row['summary']} | {row['note']} | {row['category']} |"
    for position, item in enumerate(items):
        if position not in used:
            errors.append(f"待验证项没有映射行：{item[0]}:{item[1]} {item[2][:40]}")
    if errors:
        for error in errors:
            print(f"FAIL {error}")
        return 1
    total = len(rows)
    distribution = "、".join(f"{name} {counts[name]} 条" for name in CATEGORIES)
    summary = f"合计 {total} 条。判定分布：{distribution}。"
    for index in list(updates):
        lines[index] = updates[index]
    replaced = False
    for index, line in enumerate(lines):
        if line.startswith("合计 "):
            lines[index] = summary
            replaced = True
    if not replaced:
        print("FAIL 映射表里没有「合计 …」汇总行")
        return 1
    if not check_only:
        MAP.write_text("\n".join(lines))
        print(f"resync-unverified-map: 已重排 {len(updates)} 行；{summary}")
    else:
        print(f"resync-unverified-map: 可重排 {len(updates)} 行；{summary}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
