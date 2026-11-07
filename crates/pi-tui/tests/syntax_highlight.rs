//! F35/G2：语法高亮的**对拍基准**（TS 侧用 `tools/difftest/md-oracle.mjs` 录制的期望值）。
//!
//! 这里只断言 token 切分与颜色映射，输入是合成样例；TS 逐格输出见 `LIMITATIONS` 第 11 条。

use pi_tui::syntax::{supports, tokens, TokenKind};

#[test]
fn supports_only_ref_eager_languages() {
    // ref `utils/syntax-highlight.ts:1-45` 的 eager 注册集。
    for lang in [
        "bash", "sh", "rust", "python", "js", "ts", "go", "c", "cpp", "java", "csharp", "dart",
        "groovy", "kotlin", "lua", "nix", "perl", "php", "ruby", "scala", "swift",
    ] {
        // 未实现的语言返回 false（调用方整行 `mdCodeBlock`），但**不能 panic**。
        let _ = supports(lang);
    }
    assert!(!supports("json"), "json 不在 eager 集，ref 不高亮");
    assert!(!supports("yaml"));
    assert!(!supports("markdown"));
    assert!(supports("rust"));
    assert!(supports("bash"));
    assert!(supports("python"));
}

#[test]
fn rust_tokens_match_ts_colors() {
    let line = "fn main() { println!(\"hi\"); }";
    let kinds: Vec<TokenKind> = tokens("rust", line)
        .into_iter()
        .map(|(_, _, kind)| kind)
        .collect();
    assert!(kinds.contains(&TokenKind::Keyword), "`fn` 是关键字");
    assert!(kinds.contains(&TokenKind::Function), "`main` 是函数名");
    assert!(kinds.contains(&TokenKind::String), "`\"hi\"` 是字符串");
    assert!(kinds.contains(&TokenKind::Type), "`println!` 是 built_in");
    // 标点/运算符不产生 token（ref 的 punctuation/operator 类映射到默认色，实测无 SGR）。
    assert!(
        !kinds.contains(&TokenKind::Punctuation) && !kinds.contains(&TokenKind::Operator),
        "标点/运算符不应产生 token"
    );
}

#[test]
fn shell_comment_spans_to_end_of_line() {
    let line = "echo \"hi\" # comment";
    let spans = tokens("bash", line);
    let last = spans.last().copied().expect("应有 token");
    assert_eq!(last.2, TokenKind::Comment);
    assert_eq!(last.1, line.len(), "注释覆盖到行尾");
}

#[test]
fn unknown_language_yields_no_tokens() {
    assert!(tokens("json", "{ \"a\": 1 }").is_empty());
}
