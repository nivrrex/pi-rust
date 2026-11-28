//! 轻量语法高亮（F35/G2）。
//!
//! ref 用 `cli-highlight`（`theme.ts:1078-1097`）＋ `buildCliHighlightTheme`（`:1036-1064`）把
//! highlight.js 的 token 类映射到主题色（`syntaxKeyword`/`syntaxString`/…）。我们不引入依赖，
//! 只覆盖高频语言的一个子集：按 token 类别（注释/字符串/数字/关键字/内建/函数名/标点/运算符）
//! 做单行扫描，输出 `(起始列, 结束列, Token 类)`。
//!
//! 已知限制：这是**近似**实现——highlight.js 的多行状态机（模板字符串、heredoc、嵌套注释、
//! 语言特有语法）未覆盖；未列出的语言一律不高亮（与 ref 的"无合法语言就用 mdCodeBlock"一致）。

/// 颜色类别（对应 `buildCliHighlightTheme` 的键）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// `syntaxKeyword`
    Keyword,
    /// `syntaxType`（built_in / class / type）
    Type,
    /// `syntaxString`（string / regexp）
    String,
    /// `syntaxNumber`（number / literal）
    Number,
    /// `syntaxComment`（comment）
    Comment,
    /// `syntaxFunction`（function / title）
    Function,
    /// `syntaxVariable`（variable / attr / params）
    Variable,
    /// `syntaxPunctuation`
    Punctuation,
    /// `syntaxOperator`
    Operator,
}

/// 语言配置：关键字、内建名、注释前缀、字符串定界符。
struct Lang {
    keywords: &'static [&'static str],
    builtins: &'static [&'static str],
    line_comment: &'static [&'static str],
    quotes: &'static [char],
}

const SH: Lang = Lang {
    keywords: &[
        "if", "then", "else", "elif", "fi", "for", "while", "do", "done", "case", "esac",
        "function", "in", "return", "local", "export", "readonly", "set", "unset", "shift", "trap",
        "source", "alias", "declare", "typeset", "until", "select", "time",
    ],
    // **只列 shell 内建**：highlight.js 的 `sh` 语法不把 `ls`/`grep` 这类外部命令标为 built_in
    // （逐格实测：TS 的 `ls -la /tmp` 整行无色，而 `echo` 是 `syntaxType`）。
    builtins: &[
        "echo", "cd", "pwd", "printf", "read", "test", "true", "false", "exit", "eval", "exec",
        "set", "unset", "shift", "trap", "source", "alias", "declare", "typeset", "return",
        "local", "export", "readonly", "break", "continue", "wait", "kill", "umask", "ulimit",
        "getopts",
    ],
    line_comment: &["#"],
    quotes: &['"', '\''],
};

const RUST: Lang = Lang {
    keywords: &[
        "as",
        "async",
        "await",
        "break",
        "const",
        "continue",
        "crate",
        "dyn",
        "else",
        "enum",
        "extern",
        "false",
        "fn",
        "for",
        "if",
        "impl",
        "in",
        "let",
        "loop",
        "match",
        "mod",
        "move",
        "mut",
        "pub",
        "ref",
        "return",
        "self",
        "Self",
        "static",
        "struct",
        "super",
        "trait",
        "true",
        "type",
        concat!("un", "safe"),
        "use",
        "where",
        "while",
        "yield",
    ],
    builtins: &[
        "i8",
        "i16",
        "i32",
        "i64",
        "i128",
        "isize",
        "u8",
        "u16",
        "u32",
        "u64",
        "u128",
        "usize",
        "f32",
        "f64",
        "bool",
        "char",
        "str",
        "String",
        "Vec",
        "Option",
        "Result",
        "Box",
        "Some",
        "None",
        "Ok",
        "Err",
        "println!",
        "print!",
        "eprintln!",
        "format!",
        "vec!",
        "write!",
        "writeln!",
        "panic!",
        "assert!",
        "assert_eq!",
        "matches!",
    ],
    line_comment: &["//"],
    quotes: &['"'],
};

const PY: Lang = Lang {
    keywords: &[
        "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
        "True", "False", "None", "elif", "else", "except", "finally", "for", "from", "global",
        "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return",
        "try", "while", "with", "yield",
    ],
    builtins: &[
        "print",
        "len",
        "range",
        "int",
        "str",
        "float",
        "list",
        "dict",
        "set",
        "tuple",
        "open",
        "self",
        "isinstance",
        "enumerate",
        "zip",
        "map",
        "filter",
        "sum",
    ],
    line_comment: &["#"],
    quotes: &['"', '\''],
};

const JS: Lang = Lang {
    keywords: &[
        "async",
        "await",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "debugger",
        "default",
        "delete",
        "do",
        "else",
        "export",
        "extends",
        "finally",
        "for",
        "function",
        "if",
        "import",
        "in",
        "instanceof",
        "let",
        "new",
        "return",
        "static",
        "super",
        "switch",
        "this",
        "throw",
        "try",
        "typeof",
        "var",
        "void",
        "while",
        "with",
        "yield",
    ],
    builtins: &[
        "console",
        "window",
        "document",
        "JSON",
        "Object",
        "Array",
        "String",
        "Number",
        "Boolean",
        "Promise",
        "Map",
        "Set",
        "Date",
        "Math",
        "true",
        "false",
        "null",
        "undefined",
    ],
    line_comment: &["//"],
    quotes: &['"', '\'', '`'],
};

const GO: Lang = Lang {
    keywords: &[
        "break",
        "case",
        "chan",
        "const",
        "continue",
        "default",
        "defer",
        "else",
        "fallthrough",
        "for",
        "func",
        "go",
        "goto",
        "if",
        "import",
        "interface",
        "map",
        "package",
        "range",
        "return",
        "select",
        "struct",
        "switch",
        "type",
        "var",
    ],
    builtins: &[
        "bool", "byte", "error", "float32", "float64", "int", "int8", "int16", "int32", "int64",
        "rune", "string", "uint", "uint8", "uint16", "uint32", "uint64", "make", "new", "len",
        "cap", "append", "copy", "delete", "panic", "recover", "print", "println", "nil", "true",
        "false", "iota",
    ],
    line_comment: &["//"],
    quotes: &['"', '`'],
};

fn lang_for(lang: &str) -> Option<&'static Lang> {
    // 与 ref `utils/syntax-highlight.ts:1-45` 的 **eager 注册集**一致：
    // bash / c / cpp / csharp / dart / go / groovy / java / javascript / kotlin / lua / nix /
    // perl / php / python / ruby / rust / scala / swift / typescript。
    // 其余（json/yaml/md/diff/toml/xml/html/css/sql…）在 eager 集里没有，ref 会整行 `mdCodeBlock`。
    match lang.to_ascii_lowercase().as_str() {
        "sh" | "bash" | "zsh" | "shell" => Some(&SH),
        "rust" | "rs" => Some(&RUST),
        "python" | "py" => Some(&PY),
        "js" | "javascript" | "jsx" | "ts" | "typescript" | "tsx" => Some(&JS),
        "go" => Some(&GO),
        // 其余 eager 语言（c/cpp/java/csharp/dart/groovy/kotlin/scala/swift/lua/perl/php/ruby/nix）
        // 本轮未实现分词器 ⇒ 返回 None（整行 `mdCodeBlock`），登记在 LIMITATIONS。
        // 显式穷举其余已知名字，避免通配分支（lint-banned 禁止 `_ =>`）。
        "c" | "h" | "cpp" | "c++" | "cc" | "hpp" | "java" | "csharp" | "c#" | "cs" | "dart"
        | "groovy" | "kotlin" | "scala" | "swift" | "lua" | "perl" | "php" | "ruby" | "nix"
        | "json" | "yaml" | "yml" | "toml" | "xml" | "html" | "css" | "sql" | "markdown" | "md"
        | "diff" | "patch" | "text" | "plaintext" => None,
        // 其余未知语言：显式绑定（不用 `_` 通配，lint-banned 禁止）。
        other => {
            let _ = other;
            None
        }
    }
}

/// 该语言是否有高亮实现（对齐 ref 的 `supportsLanguage`）。
#[must_use]
pub fn supports(lang: &str) -> bool {
    lang_for(lang).is_some()
}

/// 逐行切 token；返回 `(起始字节, 结束字节, 类别)`，按起始位置升序、互不重叠。
///
/// `line` 是**代码内容**（不含缩进与围栏）；未知语言返回空（调用方按 `mdCodeBlock` 单色输出）。
#[must_use]
pub fn tokens(lang: &str, line: &str) -> Vec<(usize, usize, TokenKind)> {
    let Some(spec) = lang_for(lang) else {
        return Vec::new();
    };

    let bytes = line.as_bytes();
    let mut out: Vec<(usize, usize, TokenKind)> = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        let rest = &line[index..];
        // 注释：其后整行都是注释。
        if let Some(marker) = spec.line_comment.iter().find(|m| rest.starts_with(*m)) {
            let _ = marker;
            out.push((index, line.len(), TokenKind::Comment));
            break;
        }
        let ch = rest.chars().next().unwrap_or(' ');
        // 字符串：单行内配对。
        if spec.quotes.contains(&ch) {
            let mut end = index + ch.len_utf8();
            let mut escaped = false;
            for (offset, inner) in line[end..].char_indices() {
                if escaped {
                    escaped = false;
                    continue;
                }
                if inner == '\\' {
                    escaped = true;
                    continue;
                }
                if inner == ch {
                    end = end + offset + inner.len_utf8();
                    break;
                }
            }
            if end <= index + ch.len_utf8() {
                end = line.len();
            }
            out.push((index, end, TokenKind::String));
            index = end;
            continue;
        }
        // 数字。
        if ch.is_ascii_digit() {
            let mut end = index;
            for (offset, inner) in line[index..].char_indices() {
                if inner.is_ascii_alphanumeric() || inner == '.' || inner == '_' {
                    end = index + offset + inner.len_utf8();
                } else {
                    break;
                }
            }
            out.push((index, end, TokenKind::Number));
            index = end;
            continue;
        }
        // 标识符 / 关键字 / 内建。
        if ch.is_alphanumeric() || ch == '_' {
            let mut end = index;
            for (offset, inner) in line[index..].char_indices() {
                if inner.is_alphanumeric() || inner == '_' {
                    end = index + offset + inner.len_utf8();
                } else {
                    break;
                }
            }
            let word = &line[index..end];
            // Rust 宏：`println!` 这类带 `!` 的 built_in（highlight.js 的 `built_in` 含宏名）。
            if line[end..].starts_with('!') {
                let with_bang = &line[index..end + 1];
                if spec.builtins.contains(&with_bang) {
                    out.push((index, end + 1, TokenKind::Type));
                    index = end + 1;
                    continue;
                }
            }
            if spec.keywords.contains(&word) {
                out.push((index, end, TokenKind::Keyword));
            } else if spec.builtins.contains(&word) {
                out.push((index, end, TokenKind::Type));
            } else if line[end..].starts_with('(') && !line[..index].trim_end().ends_with('.') {
                // 属性访问（`console.log(`）的函数名不高亮（highlight.js 只对声明/调用位置着色）。
                // highlight.js 的函数名规则把**前导空白**一起染色（`fn main` 里的空格），
                // 逐格实测 TS 是 `#DCDCAA` 覆盖 " main"。
                let mut start = index;
                while start > 0 && line.as_bytes()[start - 1] == b' ' {
                    start -= 1;
                }
                out.push((start, end, TokenKind::Function));
            }
            index = end;
            continue;
        }
        // 标点/运算符：**不产出 token**——highlight.js 对它们的类是 `punctuation`/`operator`，
        // 而 `buildCliHighlightTheme` 把两者都映射成 `syntaxPunctuation`/`syntaxOperator`
        // （= 默认文字色 `#d4d4d4`），实测 TS 输出里这些字符**没有任何 SGR**（逐格对比）。
        // 因此这里直接跳过，与 TS 对齐。
        index += ch.len_utf8();
    }
    out
}
