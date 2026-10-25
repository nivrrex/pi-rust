#!/usr/bin/env python3
"""核对文档里的 ref 引用：被引用的标识符必须**逐字**落在所引的行/区间里。

背景：P4/P5 的文档里引用了大量 `file.ts:NNN`。写错行号或写错标识符时，
读文档的人（和后续的 AI）会照着错的行号去实现 —— 这类错误在 P5-0 的引用里出现过，
所以这里做成可重复执行的检查，而不是靠人肉核对。

用法：
    python3 scripts/check-ref-citations.py <文档> [<文档> ...]

规则：
- 只核对能解析到 `ref/pi-0.87.1/` 下的引用；解析不到的报告为"非 ref 引用（跳过）"。
- 标识符的取法（按优先级）：
  1) 同行紧跟其后的反引号标识符（表格行 `| :12 | \\`createFileOps()\\` |` 这种"表格式"）；
  2) 同行紧挨在引用**之前**的反引号标识符（`\\`prepareCompaction\\`（\\`compaction.ts:894\\`）`）；
  3) 引用之后的第一个反引号标识符。
- 允许标识符带 `()`、参数、`const ` 等修饰：核对时做**去修饰**（去掉反引号、`()`、参数、
  前缀 `const/export/interface/type/function/async`、后缀 `= …`），只比较"名字"是否逐字出现在该行。
- 缩略引用（`\\`:170\\``）继承最近一次出现过的**完整文件名**（同一文档内、向后）。
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
REF = REPO / "ref" / "pi-0.87.1"

CITATION = re.compile(r"`?([A-Za-z0-9_./-]+\.ts)(?::(\d+)(?:-(\d+))?)`?")
LINE_ONLY = re.compile(r"`:(\d+)(?:-(\d+))?`")
BACKTICKED = re.compile(r"`([^`]+)`")


def ref_files_by_basename() -> dict[str, list[Path]]:
    table: dict[str, list[Path]] = {}
    for path in REF.rglob("*.ts"):
        table.setdefault(path.name, []).append(path)
    return table


def resolve(name: str, context: str | None, table: dict[str, list[Path]]) -> Path | None:
    """把引用里的文件名解析成 ref 下的真实文件。"""
    if "/" in name:
        for base in (REF / "packages" / "coding-agent" / "src", REF / "packages" / "ai" / "src"):
            candidate = base / name
            if candidate.exists():
                return candidate
        for candidate in REF.rglob(name):
            return candidate
        return None
    candidates = table.get(name, [])
    if not candidates:
        return None
    if len(candidates) == 1:
        return candidates[0]
    # 多个同名文件时按优先级消歧：
    # ① 最近上下文所在**完整目录**（同名不同处时最可靠）；② `coding-agent` 的权威副本；
    # ③ `compaction` 目录；④ 第一个。
    if context is not None:
        for candidate in candidates:
            if str(candidate.parent) == context:
                return candidate
    for candidate in candidates:
        if "packages/coding-agent/" in str(candidate):
            return candidate
    for candidate in candidates:
        if candidate.parent.name == "compaction":
            return candidate
    return candidates[0]


def looks_like_identifier(token: str) -> bool:
    """过滤掉不是标识符的 token：导入路径、带引号的模块名、目录路径。"""
    text = token.strip().strip("`").strip()
    if not text or text.startswith('"') or text.startswith("'") or text.startswith("./"):
        return False
    if "/" in text or text.endswith("/"):
        return False
    return True


def strip_decoration(identifier: str) -> str:
    text = identifier.strip().strip("`").strip()
    text = re.sub(r"^(>|-|\*|\+)\s*", "", text)  # markdown 列表/引用标记
    text = re.sub(r"^\d+\.\s*", "", text)
    text = re.sub(r"^(export\s+)?(async\s+)?(function|const|let|var|interface|type|class)\s+", "", text)
    text = text.split("=")[0].strip()
    text = text.split("{")[0].strip()   # `CompactionSettings {enabled, …}` → 只比名字
    text = text.split("<")[0].strip()   # `CompactionResult<T>` → 只比名字
    text = text.split("(")[0].strip()
    text = text.rstrip("?;,")
    return text.strip()


def check_document(path: Path) -> tuple[int, int, int]:
    table = ref_files_by_basename()
    lines = path.read_text().splitlines()
    checked = skipped = failed = 0
    context: str | None = None  # 最近一次解析到的完整路径文件名（用于同名消歧）
    last_name: str | None = None  # 最近一次出现过的完整文件名（用于缩略 `:NNN`）
    current_file: Path | None = None

    def identifier_for(line: str, match: re.Match[str]) -> str | None:
        """取要核对的标识符。

        两种常见写法（本仓库文档里都出现过）：
        ① 表格式：`| :12 | \`interface FileOperations\` |` —— 标识符在引用**之后**；
        ② 括号式：`\`prepareCompaction\`（\`compaction.ts:894\`）` —— 标识符在引用**之前**。
        判据用"引用是否紧跟在 `（`/`(` 之后"：是 → 取前面的；否 → 取后面的。
        """
        prefix = line[: match.start()].rstrip()
        inside_parens = prefix.endswith("（") or prefix.endswith("(")
        ordered = (
            list(reversed(BACKTICKED.findall(line[: match.start()])))
            if inside_parens
            else BACKTICKED.findall(line[match.end() :])
        )
        for token in ordered:
            if not token.endswith(".ts") and not token.startswith(":") and looks_like_identifier(token):
                return token
        # 兜底：另一侧
        fallback = (
            BACKTICKED.findall(line[match.end() :])
            if inside_parens
            else reversed(BACKTICKED.findall(line[: match.start()]))
        )
        for token in fallback:
            if not token.endswith(".ts") and not token.startswith(":") and looks_like_identifier(token):
                return token
        return None

    for number, line in enumerate(lines, 1):
        # 先把本行出现的**完整文件名**更新到 last_name（含只有文件名、没有行号的标题行，
        # 例如 `### \`core/compaction/utils.ts\`（158 行）`），缩略引用 `:NNN` 才能继承对。
        for token in BACKTICKED.findall(line):
            if token.endswith(".ts") and not token.startswith(":"):
                last_name = token
        citations = [(m, m.group(1), m.group(2), m.group(3)) for m in CITATION.finditer(line)]
        citations += [(m, None, m.group(1), m.group(2)) for m in LINE_ONLY.finditer(line)]
        for match, name, start, end in citations:
            start_line = int(start)
            end_line = int(end) if end else start_line
            if name is None:
                if last_name is None:
                    print(f"  [{path}:{number}] 缩略引用 `:{start}` 之前没有出现过文件名（跳过）")
                    skipped += 1
                    continue
                name = last_name
                current_file = current_file
            else:
                last_name = name
            resolved = resolve(name, context, table)
            if resolved is None:
                print(f"  [{path}:{number}] 非 ref 引用（跳过）：{name}")
                skipped += 1
                continue
            context = str(resolved.parent)
            current_file = resolved
            identifier = identifier_for(line, match)
            if identifier is None:
                print(f"  [{path}:{number}] 找不到标识符（跳过）：{line.strip()[:90]}")
                skipped += 1
                continue
            bare = strip_decoration(identifier)
            total = len(resolved.read_text().splitlines())
            if end_line > total:
                print(
                    f"  [{path}:{number}] **越界**：{name}:{start_line}-{end_line} 超出文件行数 {total}"
                )
                failed += 1
                continue
            window = "\n".join(resolved.read_text().splitlines()[start_line - 1 : end_line])
            checked += 1
            if bare not in window:
                rel = resolved.relative_to(REPO)
                print(
                    f"  [{path}:{number}] **不符**：标识符 `{bare}` 不在 {rel}:{start_line}"
                    f"{'' if end_line == start_line else f'-{end_line}'} 里；"
                    f"该行原文：{window.strip()[:100]}"
                )
                failed += 1
    return checked, skipped, failed


def main(argv: list[str]) -> int:
    if not argv:
        print(__doc__)
        return 2
    total = [0, 0, 0]
    for argument in argv:
        path = Path(argument)
        if not path.exists():
            print(f"找不到文档：{path}")
            return 2
        print(f"== 核对 {path}")
        checked, skipped, failed = check_document(path)
        print(f"   核对 {checked} 条，跳过 {skipped} 条，不符/越界 {failed} 条")
        total = [a + b for a, b in zip(total, (checked, skipped, failed))]
    print(f"合计：核对 {total[0]}，跳过 {total[1]}，不符/越界 {total[2]}")
    return 1 if total[2] else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
