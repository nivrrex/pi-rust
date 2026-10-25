#!/usr/bin/env bash
# scripts/lint-banned.sh —— AGENTS.md「代码约束」里的禁止项检查。
#
# 检查范围：crates/*/src/**/*.rs 中的**非测试代码**（`#[cfg(test)]` 模块与 tests/ 目录被剔除）。
# 检查项：
#   1. unsafe（各 crate 另有 #![forbid(unsafe_code)] 兜底，这里做全仓文本级检查）
#   2. 非测试代码中的 unwrap() / expect() / panic!() / todo!() / unimplemented!()
#   3. 非测试代码中的 match 通配分支 `_ =>`
#      AGENTS.md 只禁止对核心枚举用 `_ =>`，本脚本按"一律禁止"实现以获得静态保证；
#      确需对非核心枚举做通配时，在该行加 `// lint-banned: allow-wildcard` 抑制。
#
# 用法：
#   scripts/lint-banned.sh            # 检查全仓
#   scripts/lint-banned.sh --self-test  # 用故意违规的样例证明本脚本能报错

set -uo pipefail

cd "$(dirname "$0")/.." || exit 2

PY="${PYTHON:-python3}"

read -r -d '' SCANNER <<'PYEOF'
import re
import sys
from pathlib import Path

RULES = [
    ("unsafe", re.compile(r"\bunsafe\b")),
    ("unwrap", re.compile(r"\.unwrap\s*\(\s*\)")),
    ("expect", re.compile(r"\.expect\s*\(")),
    ("panic", re.compile(r"\bpanic!\s*\(")),
    ("todo", re.compile(r"\btodo!\s*\(")),
    ("unimplemented", re.compile(r"\bunimplemented!\s*\(")),
    ("wildcard", re.compile(r"(?<![A-Za-z0-9_])_\s*(?:=>|if\b)")),
]

ALLOW = "// lint-banned: allow-wildcard"


def strip_comments(text: str) -> list[tuple[int, str]]:
    """去掉块注释与行注释，返回 (行号, 代码) 列表，行号从 1 开始。"""
    out: list[tuple[int, str]] = []
    in_block = False
    for lineno, raw in enumerate(text.splitlines(), start=1):
        line = raw
        if in_block:
            end = line.find("*/")
            if end == -1:
                continue
            line = " " * end + line[end + 2 :]
            in_block = False
        while "/*" in line:
            start = line.find("/*")
            end = line.find("*/", start + 2)
            if end == -1:
                line = line[:start]
                in_block = True
                break
            line = line[:start] + " " * (end + 2 - start) + line[end + 2 :]
        idx = line.find("//")
        if idx != -1:
            comment = line[idx:]
            line = line[:idx]
            if ALLOW in comment:
                # 保留抑制标记，供通配分支规则判断。
                line = line + " " + ALLOW
        out.append((lineno, line))
    return out


def strip_cfg_test(text: str) -> str:
    """把 #[cfg(test)] 起的模块整块删掉（按大括号配平）。"""
    out: list[str] = []
    i = 0
    while True:
        m = re.search(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]", text[i:])
        if not m:
            out.append(text[i:])
            break
        start = i + m.start()
        out.append(text[i:start])
        j = text.find("{", i + m.end())
        if j == -1:
            break
        depth = 0
        k = j
        while k < len(text):
            if text[k] == "{":
                depth += 1
            elif text[k] == "}":
                depth -= 1
                if depth == 0:
                    break
            k += 1
        i = k + 1
    return "".join(out)


def scan(paths: list[Path]) -> list[str]:
    findings: list[str] = []
    for path in paths:
        raw = path.read_text(encoding="utf-8")
        if path.name.endswith(".rs") and "tests/" not in path.as_posix():
            raw = strip_cfg_test(raw)
        for lineno, line in strip_comments(raw):
            for name, pattern in RULES:
                if not pattern.search(line):
                    continue
                if name == "wildcard" and ALLOW in line:
                    continue
                if name == "panic" and "#[should_panic]" in line:
                    continue
                findings.append(f"{path}:{lineno}: 禁止项 {name}: {line.strip()[:100]}")
    return findings


if __name__ == "__main__":
    args = sys.argv[1:]
    if args and args[0] == "--scan":
        findings = scan([Path(a) for a in args[1:]])
        for f in findings:
            print(f)
        sys.exit(1 if findings else 0)
    print("scanner: usage --scan <files>", file=sys.stderr)
    sys.exit(2)
PYEOF

scan_files() {
    find crates -type f -name '*.rs' -path '*/src/*' ! -path '*/tests/*' | sort
}

self_test() {
    local tmp findings
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' RETURN

    cat >"$tmp/unsafe.rs" <<'EOF'
pub fn f() { let x = 1; let _p = unsafe { &x }; }
EOF
    cat >"$tmp/unwrap.rs" <<'EOF'
pub fn f() -> i32 { let v: Vec<i32> = Vec::new(); v[0].unwrap() }
EOF
    cat >"$tmp/wildcard.rs" <<'EOF'
enum E { A, B }
pub fn f(e: E) -> i32 { match e { E::A => 1, _ => 0 } }
EOF
    cat >"$tmp/ok.rs" <<'EOF'
#[cfg(test)]
mod tests {
    #[test]
    fn t() { let v: Vec<i32> = Vec::new(); assert_eq!(v.first().unwrap(), None); }
}
EOF

    local rc=0
    for f in unsafe.rs unwrap.rs wildcard.rs; do
        findings="$("$PY" -c "$SCANNER" --scan "$tmp/$f" 2>/dev/null)"
        if [ -n "$findings" ]; then
            echo "  [报错(符合预期)] $f: ${findings%%:* 禁止项*}"
        else
            echo "  [未报错(自检失败)] $f"
            rc=1
        fi
    done
    findings="$("$PY" -c "$SCANNER" --scan "$tmp/ok.rs" 2>/dev/null)"
    if [ -z "$findings" ]; then
        echo "  [通过(符合预期)] ok.rs: 测试代码里的 unwrap 与 #[cfg(test)] 块均被放行"
    else
        echo "  [误报(自检失败)] ok.rs: $findings"
        rc=1
    fi

    if [ "$rc" -eq 0 ]; then
        echo "SELF-TEST PASS"
    else
        echo "SELF-TEST FAIL"
    fi
    return "$rc"
}

if [ "${1:-}" = "--self-test" ]; then
    self_test
    exit $?
fi

mapfile -t files < <(scan_files)
if [ "${#files[@]}" -eq 0 ]; then
    echo "lint-banned: 未找到待检查的 .rs 文件"
    exit 2
fi

echo "lint-banned: 检查 ${#files[@]} 个文件"
findings="$("$PY" -c "$SCANNER" --scan "${files[@]}")"
status=$?
if [ "$status" -ne 0 ]; then
    echo "$findings"
    echo "lint-banned: FAILED"
    exit 1
fi
echo "lint-banned: OK"
exit 0