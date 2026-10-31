#!/usr/bin/env python3
"""校验 spec 里的【待验证】条目与 spec/unverified-map.md 的映射表一一对应。

规则（RUNBOOK-3 A3）：
- `spec/*.md`（排除索引类文件）里每条以 `- 【待验证】` 开头的条目，必须在映射表里**恰好出现一次**；
- 映射表里每一行的 `文件:行号` 必须指向一个真实存在的条目（行号漂移即报错）；
- 判定列只能取固定枚举；
- 表头给出的汇总数字必须等于表格实际行数，且各分类计数之和等于总数。

用法：
    python3 scripts/check-unverified-map.py
    python3 scripts/check-unverified-map.py --self-test
"""

from __future__ import annotations

import re
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SPEC_DIR = REPO / "spec"
MAP_FILE = SPEC_DIR / "unverified-map.md"
EXCLUDED = {"00-index.md", "unverified-map.md"}

CATEGORIES = ["已可证实", "部分证实", "不适用", "需读码", "需新场景", "待决"]

PENDING_RE = re.compile(r"^\s*-\s*【待验证】")
ROW_RE = re.compile(r"^\|\s*`(?P<file>[^`:]+\.md):(?P<line>\d+)`\s*\|(?P<rest>.*)$")


def spec_items() -> list[tuple[str, int, str]]:
    """返回 (文件名, 行号, 条目文本)，按文件名与行号排序。"""
    items: list[tuple[str, int, str]] = []
    for path in sorted(SPEC_DIR.glob("*.md")):
        if path.name in EXCLUDED:
            continue
        for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if PENDING_RE.match(line):
                items.append((path.name, lineno, line.strip()))
    return items


def map_rows() -> tuple[list[tuple[str, int, str]], str]:
    """返回映射表行与汇总数字原文。"""
    rows: list[tuple[str, int, str]] = []
    summary = ""
    for line in MAP_FILE.read_text(encoding="utf-8").splitlines():
        if line.startswith("合计 "):
            summary = line
        m = ROW_RE.match(line)
        if m:
            cells = [c.strip() for c in m.group("rest").split("|") if c.strip()]
            rows.append((m.group("file"), int(m.group("line")), cells[-1]))
    return rows, summary


def parse_counts(summary: str) -> dict[str, int] | None:
    counts: dict[str, int] = {}
    for name in CATEGORIES:
        m = re.search(rf"{name}\s+(\d+)\s*条", summary)
        if not m:
            return None
        counts[name] = int(m.group(1))
    m = re.search(r"合计\s+(\d+)\s*条", summary)
    if not m:
        return None
    counts["合计"] = int(m.group(1))
    return counts


def check() -> list[str]:
    errors: list[str] = []
    items = spec_items()
    rows, summary = map_rows()

    item_keys = [(f, ln) for f, ln, _ in items]
    row_keys = [(f, ln) for f, ln, _ in rows]

    from collections import Counter

    for key, n in Counter(item_keys).items():
        if n > 1:
            errors.append(f"spec 中 {key[0]}:{key[1]} 出现 {n} 次")
    dupes = [k for k, n in Counter(row_keys).items() if n > 1]
    for key in dupes:
        errors.append(f"映射表中 {key[0]}:{key[1]} 重复出现")

    missing = [k for k in item_keys if k not in row_keys]
    for key in missing:
        errors.append(f"映射表缺少条目 {key[0]}:{key[1]}")

    item_set = set(item_keys)
    for key in row_keys:
        if key not in item_set:
            errors.append(f"映射表指向不存在的条目 {key[0]}:{key[1]}（行号漂移？）")

    for (fname, lineno), verdict in zip(row_keys, [v for _, _, v in rows]):
        if verdict not in CATEGORIES:
            errors.append(f"{fname}:{lineno} 的判定 '{verdict}' 不在固定枚举 {CATEGORIES}")

    counts = parse_counts(summary)
    if counts is None:
        errors.append("映射表缺少可解析的汇总数字（形如「合计 N 条。判定分布：…」）")
    else:
        actual = Counter(v for _, _, v in rows)
        for name in CATEGORIES:
            if counts[name] != actual.get(name, 0):
                errors.append(
                    f"汇总里 {name} 写 {counts[name]} 条，表里实际 {actual.get(name, 0)} 条"
                )
        if counts["合计"] != len(rows):
            errors.append(f"汇总合计 {counts['合计']} 与表格行数 {len(rows)} 不符")
        if sum(actual.get(c, 0) for c in CATEGORIES) != len(rows):
            errors.append("各分类计数之和与表格行数不符")
    return errors


def self_test() -> int:
    """故意造三种错误，验证脚本能报出来。"""
    good_items = "- 【待验证】示例条目一\n- 【待验证】示例条目二\n"
    good_map = (
        "# t\n\n合计 2 条。判定分布：已可证实 1 条、部分证实 1 条、不适用 0 条、"
        "需读码 0 条、需新场景 0 条、待决 0 条。\n\n"
        "| spec 位置 | 主题 | 证实途径 | 判定 |\n|---|---|---|---|\n"
        "| `t.md:1` | 甲 | x | 已可证实 |\n"
        "| `t.md:2` | 乙 | x | 部分证实 |\n"
    )
    bad_cases = {
        "缺条目": good_map.replace("| `t.md:2` | 乙 | x | 部分证实 |\n", ""),
        "行号漂移": good_map.replace("`t.md:2`", "`t.md:9`"),
        "非法判定": good_map.replace("| 部分证实 |", "| 差不多 |"),
        "汇总不符": good_map.replace("已可证实 1 条", "已可证实 2 条"),
    }
    ok = True
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        (root / "spec").mkdir()
        (root / "scripts").mkdir()
        for label, map_text in [("正确样例", good_map)] + list(bad_cases.items()):
            (root / "spec" / "t.md").write_text(good_items, encoding="utf-8")
            (root / "spec" / "unverified-map.md").write_text(map_text, encoding="utf-8")
            errs = _check_against(root)
            expected_ok = label == "正确样例"
            if expected_ok and errs:
                print(f"  [误报(自检失败)] {label}: {errs[0]}")
                ok = False
            elif not expected_ok and not errs:
                print(f"  [未报错(自检失败)] {label}")
                ok = False
            else:
                first = errs[0] if errs else "-"
                print(f"  [{'通过' if expected_ok else '报错(符合预期)'}] {label}: {first}")
    print("SELF-TEST", "PASS" if ok else "FAIL")
    return 0 if ok else 1


def _check_against(root: Path) -> list[str]:
    """在指定根目录上跑一遍核账（供自检用）。"""
    global SPEC_DIR, MAP_FILE
    old_spec, old_map = SPEC_DIR, MAP_FILE
    SPEC_DIR, MAP_FILE = root / "spec", root / "spec" / "unverified-map.md"
    try:
        return check()
    finally:
        SPEC_DIR, MAP_FILE = old_spec, old_map


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not MAP_FILE.exists():
        print(f"映射表不存在: {MAP_FILE}")
        return 2
    errors = check()
    items = spec_items()
    rows, _ = map_rows()
    print(f"spec 中【待验证】条目: {len(items)}")
    print(f"映射表行数: {len(rows)}")
    if errors:
        for e in errors:
            print(f"  FAIL {e}")
        print(f"check-unverified-map: FAILED（{len(errors)} 项）")
        return 1
    print("check-unverified-map: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())