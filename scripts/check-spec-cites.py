#!/usr/bin/env python3
"""检查 spec/*.md 中的源码引用是否真实存在。

规则（RUNBOOK-1 A0）：
- 每条事实是一个 bullet，以 `〔路径:起行-止行〕` 结尾，路径相对 ref/pi-0.87.1/。
- 每条事实至少含一个反引号包裹的标识符。
- 校验：文件存在；行号区间合法；至少一个标识符出现在被引用的行区间内。
- 以 `【待验证】` 开头的条目不参与校验，只统计数量。

用法：
    python3 scripts/check-spec-cites.py [spec/*.md ...]
        默认检查 spec/ 下所有 .md。使用 --ref 指定 ref 根目录（默认 ref/pi-0.87.1）。
    python3 scripts/check-spec-cites.py --self-test
        生成 3 个故意写错的样例，验证本脚本能报错。
"""

from __future__ import annotations

import argparse
import re
import sys
import tempfile
from pathlib import Path

BULLET_RE = re.compile(r"^\s*(?:[-*+]|\d+\.)\s+")
PROJECT_PREFIXES = ("golden/", "fixtures/", "scenarios/")
HEADING_RE = re.compile(r"^#{1,6}\s")
CITE_RE = re.compile(r"〔([^:：]+):(\d+)(?:-(\d+))?〕\s*$")
IDENT_RE = re.compile(r"`([^`\n]+)`")
PENDING_PREFIX = "【待验证】"


def split_blocks(text: str) -> list[str]:
    """把 markdown 切成 bullet 块；非 bullet 内容（如标题、表格、段落）返回为独立块。"""
    blocks: list[str] = []
    current: list[str] | None = None
    for line in text.splitlines():
        if BULLET_RE.match(line) or HEADING_RE.match(line):
            if current:
                blocks.append("\n".join(current).strip())
            current = [line] if BULLET_RE.match(line) else None
            if current is None:
                blocks.append(line.strip())
        elif current is not None:
            current.append(line)
        else:
            stripped = line.strip()
            if stripped:
                blocks.append(stripped)
    if current:
        blocks.append("\n".join(current).strip())
    return [b for b in blocks if b]


def resolve(rel_path: str, ref_root: Path) -> Path | None:
    """引用路径的解析规则。

    - 以 `golden/`、`fixtures/`、`scenarios/` 开头：相对**项目根**（check-spec-cites.py 所在仓库）。
    - 其余：相对 `ref/pi-0.87.1/`。
    """
    if rel_path.startswith(PROJECT_PREFIXES):
        return Path(__file__).resolve().parent.parent / rel_path
    return ref_root / rel_path


def check_file(spec_file: Path, ref_root: Path) -> tuple[int, int, list[str]]:
    """返回 (通过数, 待验证数, 失败信息列表)。"""
    errors: list[str] = []
    passed = 0
    pending = 0
    lines_cache: dict[str, list[str]] = {}

    for block in split_blocks(spec_file.read_text(encoding="utf-8")):
        if not BULLET_RE.match(block):
            continue
        if BULLET_RE.sub("", block, count=1).lstrip().startswith(PENDING_PREFIX):
            pending += 1
            continue

        m = CITE_RE.search(block)
        if not m:
            errors.append(f"{spec_file}: 引用缺失或不在行尾: {block.splitlines()[0][:90]}")
            continue

        rel_path, start_s, end_s = m.group(1), m.group(2), m.group(3)
        start = int(start_s)
        end = int(end_s) if end_s else start

        body = block[: m.start()]
        identifiers = [i.strip() for i in IDENT_RE.findall(body)]
        if not identifiers:
            errors.append(f"{spec_file}: 无反引号标识符: {block.splitlines()[0][:90]}")
            continue

        if rel_path not in lines_cache:
            target = resolve(rel_path, ref_root)
            if not target.is_file():
                errors.append(f"{spec_file}: 文件不存在 {rel_path} 〔{block.splitlines()[0][:60]}〕")
                lines_cache[rel_path] = []
                lines = None
            else:
                lines = target.read_text(encoding="utf-8", errors="replace").splitlines()
                lines_cache[rel_path] = lines
        else:
            lines = lines_cache[rel_path] or None

        if not lines:
            continue

        total = len(lines)
        if start < 1 or end < start or end > total:
            errors.append(
                f"{spec_file}: 行号越界 {rel_path}:{start}-{end}（文件共 {total} 行）"
                f" 〔{block.splitlines()[0][:60]}〕"
            )
            continue

        snippet = "\n".join(lines[start - 1 : end])
        if not any(ident in snippet for ident in identifiers):
            errors.append(
                f"{spec_file}: 标识符不在引用区间内 {rel_path}:{start}-{end} "
                f"候选={identifiers[:3]} 〔{block.splitlines()[0][:60]}〕"
            )
            continue

        passed += 1

    return passed, pending, errors


def self_test(ref_root: Path) -> int:
    """3 个故意写错的样例必须各自报错。"""
    cases = {
        "文件不存在": "- `Message` 的判定函数是 `isMessage`〔packages/does-not-exist.ts:1-5〕\n",
        "行号越界": "- `Message` 的判定函数是 `isMessage`〔packages/coding-agent/src/core/messages.ts:900000-900001〕\n",
        "标识符不在区间": "- 该函数的返回类型是 `TotallyAbsentSymbolName`〔packages/coding-agent/src/core/messages.ts:1-5〕\n",
        "golden 路径解析": "- S01 的退出码是 `exitCode`〔golden/S01/does-not-exist.json:1-2〕\n",
        "fixtures 路径解析": "- 夹具里的固定 id 是 `chatcmpl-mock`〔fixtures/sse/openai/nope.sse:1-2〕\n",
    }
    ok = True
    with tempfile.TemporaryDirectory() as td:
        # 额外验证：合法引用（golden/fixtures 路径）不应报错。
        for label, content in cases.items():
            f = Path(td) / "sample.md"
            f.write_text("# 样例\n\n" + content, encoding="utf-8")
            _, _, errors = check_file(f, ref_root)
            status = "报错(符合预期)" if errors else "未报错(自检失败)"
            if not errors:
                ok = False
            print(f"  [{status}] {label}: {errors[0] if errors else '-'}")

        # 正向样例：golden/ 与 fixtures/ 下的真实文件应通过，说明项目根相对引用能解析。
        for rel, ident in [
            ("golden/S01/meta.json", "exitCode"),
            ("fixtures/sse/openai/text-basic.sse", "chatcmpl-mock"),
        ]:
            target = Path(rel)
            if not target.is_file():
                print(f"  [跳过(自检文件不存在)] {rel}")
                continue
            f = Path(td) / "positive.md"
            f.write_text(
                f"- 该值是 `{ident}`〔{rel}:1-{len(target.read_text(encoding='utf-8').splitlines())}〕\n",
                encoding="utf-8",
            )
            _, _, errors = check_file(f, ref_root)
            if errors:
                print(f"  [误报(自检失败)] {rel}: {errors[0]}")
                ok = False
            else:
                print(f"  [通过(符合预期)] {rel}: 项目根相对引用可解析")

    print("SELF-TEST", "PASS" if ok else "FAIL")
    return 0 if ok else 1


def main() -> int:
    ap = argparse.ArgumentParser(description="校验 spec 的源码引用")
    ap.add_argument("specs", nargs="*", help="要检查的 spec 文件，默认 spec/*.md")
    ap.add_argument("--ref", default="ref/pi-0.87.1", help="ref 根目录")
    ap.add_argument("--self-test", action="store_true", help="用 3 个错误样例自检")
    args = ap.parse_args()

    ref_root = Path(args.ref)
    if args.self_test:
        return self_test(ref_root)

    if not ref_root.is_dir():
        # 归档/干净克隆里没有 `ref/`（上游源码不随仓库分发）⇒ 该项**跳过**而不是失败；
        # 需要跑它时把上游 checkout 放到 `ref/pi-0.87.1/`。
        print(f"跳过：{ref_root} 不存在（未提供上游源码 checkout）")
        return 0

    files = [Path(s) for s in args.specs] if args.specs else sorted(Path("spec").glob("*.md"))
    # 00-index 与 unverified-map 是索引类文件，里面的条目不是"带引用的事实"。
    files = [f for f in files if f.name not in ("00-index.md", "unverified-map.md")]

    total_pass = 0
    total_pending = 0
    all_errors: list[str] = []
    for f in files:
        p, pend, errs = check_file(f, ref_root)
        total_pass += p
        total_pending += pend
        all_errors.extend(errs)
        print(f"{f}: 通过 {p}，待验证 {pend}，失败 {len(errs)}")

    print(f"合计：通过 {total_pass}，待验证 {total_pending}，失败 {len(all_errors)}")
    for e in all_errors:
        print(f"  FAIL {e}")
    return 1 if all_errors else 0


if __name__ == "__main__":
    sys.exit(main())