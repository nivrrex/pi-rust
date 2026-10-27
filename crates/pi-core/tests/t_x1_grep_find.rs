//! X1：`grep` / `find` 的对外行为（委托外部 `rg`/`fd`）。
//!
//! 期望值取自上游 `core/tools/grep.ts`、`find.ts` 的格式化逻辑：
//! - grep：`<相对路径>:<行号>: <该行文本>`；无匹配 `No matches found`；达到 limit 追加
//!   `<N> matches limit reached. Use limit=<2N> for more, or refine pattern`。
//! - find：逐行相对路径；无结果 `No files found matching pattern`；达到 limit 追加
//!   `<N> results limit reached. Use limit=<2N> for more, or refine pattern`。

use std::fs;
use std::path::{Path, PathBuf};

use pi_core::tools::tool::{Tool, ToolContext, ToolResult};
use serde_json::json;
use tokio_util::sync::CancellationToken;

/// `fd` 不在 PATH 时这几条 find 用例无从验证——打印原因后跳过（不静默通过）。
fn require_fd() -> bool {
    if pi_core::tools::find::fd_binary().is_some() {
        return true;
    }
    eprintln!("跳过：本机没有 `fd`/`fdfind`（可用 PI_FD_BIN 指定）");
    false
}

fn fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pi-x1-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("src")).expect("mkdir");
    fs::write(dir.join("src/alpha.txt"), "needle one\nplain\nneedle two\n").expect("write");
    fs::write(dir.join("src/beta.txt"), "nothing here\n").expect("write");
    fs::write(dir.join("notes.md"), "Needle upper\n").expect("write");
    dir
}

fn context(cwd: &Path) -> ToolContext {
    ToolContext {
        cwd: cwd.to_path_buf(),
    }
}

fn text_of(result: &ToolResult) -> String {
    result
        .content
        .iter()
        .map(|block| match block {
            pi_core::tools::tool::ToolResultBlock::Text { text } => text.clone(),
            pi_core::tools::tool::ToolResultBlock::Image { .. } => String::new(),
        })
        .collect::<Vec<_>>()
        .join("")
}

#[tokio::test]
async fn grep_formats_matches_like_upstream() {
    let dir = fixture("grep-format");
    let context = context(&dir);
    let cancel = CancellationToken::new();
    let result = pi_core::tools::grep::GrepTool
        .execute(json!({"pattern": "needle"}), &context, &cancel)
        .await;
    let text = text_of(&result);
    assert_eq!(
        text,
        "src/alpha.txt:1: needle one\nsrc/alpha.txt:3: needle two"
    );
}

#[tokio::test]
async fn grep_reports_no_matches() {
    let dir = fixture("grep-none");
    let context = context(&dir);
    let cancel = CancellationToken::new();
    let result = pi_core::tools::grep::GrepTool
        .execute(json!({"pattern": "definitely-absent"}), &context, &cancel)
        .await;
    assert_eq!(text_of(&result), "No matches found");
}

#[tokio::test]
async fn grep_honours_limit_notice() {
    let dir = fixture("grep-limit");
    let context = context(&dir);
    let cancel = CancellationToken::new();
    let result = pi_core::tools::grep::GrepTool
        .execute(json!({"pattern": "needle", "limit": 1}), &context, &cancel)
        .await;
    let text = text_of(&result);
    assert!(text.starts_with("src/alpha.txt:1: needle one"), "{text}");
    assert!(
        text.contains("1 matches limit reached. Use limit=2 for more, or refine pattern"),
        "{text}"
    );
}

#[tokio::test]
async fn grep_ignore_case_and_glob() {
    let dir = fixture("grep-icase");
    let context = context(&dir);
    let cancel = CancellationToken::new();
    let result = pi_core::tools::grep::GrepTool
        .execute(
            json!({"pattern": "needle", "ignoreCase": true, "glob": "*.md"}),
            &context,
            &cancel,
        )
        .await;
    assert_eq!(text_of(&result), "notes.md:1: Needle upper");
}

/// fd 打印的是 `<搜索根>/<相对路径>`（绝对根时形如 `/tmp/x/./src/a.txt`）。
/// 断言时先剥掉 fixture 根与多余的 `./`，只比较**相对 fixture 的路径集合**，
/// 避免把 fd 的路径拼接细节写进用例。
fn relative_to(root: &std::path::Path, lines: &[String]) -> Vec<String> {
    let root = root.display().to_string();
    let mut out: Vec<String> = lines
        .iter()
        .map(|line| {
            let stripped = line.strip_prefix(&format!("{root}/")).unwrap_or(line);
            stripped.strip_prefix("./").unwrap_or(stripped).to_string()
        })
        .collect();
    out.sort();
    out
}

#[tokio::test]
async fn find_returns_relative_paths() {
    if !require_fd() {
        return;
    }
    let dir = fixture("find-paths");
    let context = context(&dir);
    let cancel = CancellationToken::new();
    let result = pi_core::tools::find::FindTool
        .execute(json!({"pattern": "*.txt"}), &context, &cancel)
        .await;
    let lines: Vec<String> = text_of(&result).lines().map(str::to_string).collect();
    assert_eq!(
        relative_to(&dir, &lines),
        vec!["src/alpha.txt", "src/beta.txt"]
    );
}

#[tokio::test]
async fn find_reports_no_files() {
    if !require_fd() {
        return;
    }
    let dir = fixture("find-none");
    let context = context(&dir);
    let cancel = CancellationToken::new();
    let result = pi_core::tools::find::FindTool
        .execute(json!({"pattern": "*.nomatch"}), &context, &cancel)
        .await;
    assert_eq!(text_of(&result), "No files found matching pattern");
}

#[tokio::test]
async fn find_honours_limit_notice() {
    if !require_fd() {
        return;
    }
    let dir = fixture("find-limit");
    let context = context(&dir);
    let cancel = CancellationToken::new();
    let result = pi_core::tools::find::FindTool
        .execute(json!({"pattern": "*.txt", "limit": 1}), &context, &cancel)
        .await;
    let text = text_of(&result);
    assert!(
        text.contains("1 results limit reached. Use limit=2 for more, or refine pattern"),
        "{text}"
    );
}

#[tokio::test]
async fn grep_literal_treats_pattern_as_text() {
    let dir = fixture("grep-literal");
    fs::write(
        dir.join("src/symbols.txt"),
        "a.b
axb
",
    )
    .expect("write");
    let context = context(&dir);
    let cancel = CancellationToken::new();
    let result = pi_core::tools::grep::GrepTool
        .execute(
            json!({"pattern": "a.b", "literal": true}),
            &context,
            &cancel,
        )
        .await;
    assert_eq!(text_of(&result), "src/symbols.txt:1: a.b");
}

#[tokio::test]
async fn grep_accepts_single_file_path() {
    let dir = fixture("grep-file");
    let context = context(&dir);
    let cancel = CancellationToken::new();
    let result = pi_core::tools::grep::GrepTool
        .execute(
            json!({"pattern": "plain", "path": "src/alpha.txt"}),
            &context,
            &cancel,
        )
        .await;
    assert_eq!(text_of(&result), "src/alpha.txt:2: plain");
}

#[tokio::test]
async fn grep_reports_missing_path() {
    let dir = fixture("grep-missing");
    let context = context(&dir);
    let cancel = CancellationToken::new();
    let result = pi_core::tools::grep::GrepTool
        .execute(
            json!({"pattern": "x", "path": "no-such-dir"}),
            &context,
            &cancel,
        )
        .await;
    assert!(
        text_of(&result).starts_with("Path not found:"),
        "{}",
        text_of(&result)
    );
}

#[tokio::test]
async fn grep_glob_without_match_returns_none() {
    let dir = fixture("grep-glob-none");
    let context = context(&dir);
    let cancel = CancellationToken::new();
    let result = pi_core::tools::grep::GrepTool
        .execute(
            json!({"pattern": "needle", "glob": "*.rs"}),
            &context,
            &cancel,
        )
        .await;
    assert_eq!(text_of(&result), "No matches found");
}

#[tokio::test]
async fn grep_handles_utf8_text() {
    let dir = fixture("grep-utf8");
    fs::write(dir.join("src/中文.txt"), "第一行\n需要的中文\n").expect("write");
    let context = context(&dir);
    let cancel = CancellationToken::new();
    let result = pi_core::tools::grep::GrepTool
        .execute(json!({"pattern": "中文"}), &context, &cancel)
        .await;
    let text = text_of(&result);
    assert!(text.contains(":2: 需要的中文"), "{text}");
}

#[tokio::test]
async fn find_glob_star_star_matches_nested() {
    if !require_fd() {
        return;
    }
    let dir = fixture("find-nested");
    fs::create_dir_all(dir.join("src/deep")).expect("mkdir");
    fs::write(dir.join("src/deep/gamma.txt"), "x\n").expect("write");
    let context = context(&dir);
    let cancel = CancellationToken::new();
    let result = pi_core::tools::find::FindTool
        .execute(json!({"pattern": "**/*.txt"}), &context, &cancel)
        .await;
    let lines: Vec<String> = text_of(&result).lines().map(str::to_string).collect();
    assert_eq!(
        relative_to(&dir, &lines),
        vec!["src/alpha.txt", "src/beta.txt", "src/deep/gamma.txt"]
    );
}

#[tokio::test]
async fn find_searches_subdirectory() {
    if !require_fd() {
        return;
    }
    let dir = fixture("find-subdir");
    let context = context(&dir);
    let cancel = CancellationToken::new();
    let result = pi_core::tools::find::FindTool
        .execute(
            json!({"pattern": "*.txt", "path": "src"}),
            &context,
            &cancel,
        )
        .await;
    let lines: Vec<String> = text_of(&result).lines().map(str::to_string).collect();
    assert_eq!(
        relative_to(&dir.join("src"), &lines),
        vec!["alpha.txt", "beta.txt"]
    );
}

#[tokio::test]
async fn find_reports_missing_path() {
    if !require_fd() {
        return;
    }
    let dir = fixture("find-missing");
    let context = context(&dir);
    let cancel = CancellationToken::new();
    let result = pi_core::tools::find::FindTool
        .execute(
            json!({"pattern": "*.txt", "path": "nope"}),
            &context,
            &cancel,
        )
        .await;
    assert!(text_of(&result).starts_with("Path not found:"));
}
