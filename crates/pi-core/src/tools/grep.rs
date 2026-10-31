//! `grep` 工具：搜索文件内容（委托外部 `rg`，不自动下载）。
//!
//! 对应上游 `packages/coding-agent/src/core/tools/grep.ts`：
//! - schema：`pattern` / `path` / `glob` / `ignoreCase` / `literal` / `context` / `limit`；
//! - 调用：`rg --json --line-number --color=never --hidden [--ignore-case] [--fixed-strings]
//!   [--glob G] -- <pattern> <path>`（`:162-166`）；
//! - 输出行：`<相对路径>:<行号>: <该行文本>`（`:266-275`），单行超过 `GREP_MAX_LINE_LENGTH`
//!   会被截断（`:277-280`）；
//! - 无匹配 → `No matches found`（`:255-259`）；
//! - 达到 limit → 追加 `<N> matches limit reached. Use limit=<2N> for more, or refine pattern`
//!   （`:288-293`）；字节超限 → `<size> limit reached`；行被截断 →
//!   `Some lines truncated to <N> chars. Use read tool to see full lines`。
//!
//! 与上游的**唯一区别**：`rg` 缺失时**不尝试下载**，直接返回带安装指引的错误（本批要求）。

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::read_text::{format_size, truncate_head, DEFAULT_MAX_BYTES};
use super::tool::{Tool, ToolContext, ToolResult};

/// 默认匹配上限（`grep.ts:41`）。
const DEFAULT_LIMIT: usize = 100;
/// 单行最长字符数（上游 `truncate.ts` 的 `GREP_MAX_LINE_LENGTH`）。
const GREP_MAX_LINE_LENGTH: usize = 500;

/// `grep` 工具。
#[derive(Debug, Default)]
pub struct GrepTool;

/// 参数 schema（`grep.ts:29-39`）。
#[must_use]
pub fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "pattern": {"type": "string", "description": "Search pattern (regex or literal string)"},
            "path": {"type": "string", "description": "Directory or file to search (default: current directory)"},
            "glob": {"type": "string", "description": "Filter files by glob pattern, e.g. '*.ts' or '**/*.spec.ts'"},
            "ignoreCase": {"type": "boolean", "description": "Case-insensitive search (default: false)"},
            "literal": {"type": "boolean", "description": "Treat pattern as literal string instead of regex (default: false)"},
            "context": {"type": "number", "description": "Number of lines to show before and after each match (default: 0)"},
            "limit": {"type": "number", "description": "Maximum number of matches to return (default: 100)"}
        },
        "required": ["pattern"]
    })
}

impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Search file contents for patterns (respects .gitignore). Returns matches as `path:line: text`."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(schema)
    }

    fn execute<'a>(
        &'a self,
        arguments: Value,
        context: &'a ToolContext,
        cancel: &'a CancellationToken,
    ) -> futures::future::BoxFuture<'a, ToolResult> {
        Box::pin(async move {
            if cancel.is_cancelled() {
                return ToolResult::error("Operation aborted");
            }
            search(&arguments, &context.cwd)
        })
    }
}

fn search(arguments: &Value, cwd: &Path) -> ToolResult {
    let Some(pattern) = arguments.get("pattern").and_then(Value::as_str) else {
        return ToolResult::error("pattern is required");
    };
    let requested = arguments
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or(".");
    let target: PathBuf = resolve(cwd, requested);
    if !target.exists() {
        return ToolResult::error(format!("Path not found: {}", target.display()));
    }
    let limit = arguments
        .get("limit")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value > 0.0)
        .map_or(DEFAULT_LIMIT, |value| value as usize);

    let mut args: Vec<String> = vec![
        "--json".to_string(),
        "--line-number".to_string(),
        "--color=never".to_string(),
        "--hidden".to_string(),
    ];
    if arguments.get("ignoreCase").and_then(Value::as_bool) == Some(true) {
        args.push("--ignore-case".to_string());
    }
    if arguments.get("literal").and_then(Value::as_bool) == Some(true) {
        args.push("--fixed-strings".to_string());
    }
    if let Some(glob) = arguments.get("glob").and_then(Value::as_str) {
        args.push("--glob".to_string());
        args.push(glob.to_string());
    }
    args.push("--".to_string());
    args.push(pattern.to_string());
    args.push(target.display().to_string());

    let output = match Command::new("rg").args(&args).output() {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return ToolResult::error(
                "`rg` (ripgrep) is not available. Install it first, e.g. `apt install ripgrep` \
                 (Debian/Ubuntu) or `pacman -S ripgrep` (Arch), then retry.",
            );
        }
        Err(error) => return ToolResult::error(format!("Failed to run ripgrep: {error}")),
    };
    // `rg` 的退出码：0 = 有匹配，1 = 无匹配，2 = 出错（`grep.ts:247-252`）。
    let code = output.status.code().unwrap_or(2);
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if code != 0 && code != 1 {
        return ToolResult::error(if stderr.is_empty() {
            format!("ripgrep exited with code {code}")
        } else {
            stderr
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines: Vec<String> = Vec::new();
    let mut match_limit_reached = false;
    let mut lines_truncated = false;
    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if lines.len() >= limit {
            match_limit_reached = true;
            break;
        }
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if event.get("type").and_then(Value::as_str) != Some("match") {
            continue;
        }
        let data = event.get("data");
        let file_path = data
            .and_then(|data| data.get("path"))
            .and_then(|path| path.get("text"))
            .and_then(Value::as_str);
        let line_number = data
            .and_then(|data| data.get("line_number"))
            .and_then(Value::as_u64);
        let (Some(file_path), Some(line_number)) = (file_path, line_number) else {
            continue;
        };
        let text = data
            .and_then(|data| data.get("lines"))
            .and_then(|lines| lines.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .replace("\r\n", "\n")
            .replace('\r', "");
        let text = text.strip_suffix('\n').unwrap_or(&text).to_string();
        let (text, truncated) = truncate_line(&text);
        if truncated {
            lines_truncated = true;
        }
        lines.push(format!(
            "{}:{line_number}: {text}",
            display_path(cwd, &target, file_path)
        ));
        if lines.len() >= limit {
            match_limit_reached = true;
            break;
        }
    }

    if lines.is_empty() {
        return ToolResult::text("No matches found");
    }
    let raw = lines.join("\n");
    let truncation = truncate_head(&raw, usize::MAX, DEFAULT_MAX_BYTES);
    let mut output = truncation.content;
    let mut notices: Vec<String> = Vec::new();
    if match_limit_reached {
        notices.push(format!(
            "{limit} matches limit reached. Use limit={} for more, or refine pattern",
            limit * 2
        ));
    }
    if truncation.truncated {
        notices.push(format!("{} limit reached", format_size(DEFAULT_MAX_BYTES)));
    }
    if lines_truncated {
        notices.push(format!(
            "Some lines truncated to {GREP_MAX_LINE_LENGTH} chars. Use read tool to see full lines"
        ));
    }
    if !notices.is_empty() {
        output.push_str("\n\n[");
        output.push_str(&notices.join(". "));
        output.push(']');
    }
    ToolResult::text(output)
}

/// 把 `rg` 给的路径转成"相对搜索起点"的显示形式（`grep.ts:266-268` 的 `formatPath`）。
fn display_path(cwd: &Path, target: &Path, file_path: &str) -> String {
    let path = Path::new(file_path);
    // 先按 `cwd` 归一（`path.relative(cwd, …)` 的口径），再按搜索起点归一。
    for base in [cwd, target] {
        if let Ok(relative) = path.strip_prefix(base) {
            return relative.display().to_string();
        }
    }
    path.display().to_string()
}

/// 单行超长时截断（`grep.ts:277-280`）。
fn truncate_line(line: &str) -> (String, bool) {
    if line.chars().count() <= GREP_MAX_LINE_LENGTH {
        return (line.to_string(), false);
    }
    let truncated: String = line.chars().take(GREP_MAX_LINE_LENGTH).collect();
    (truncated, true)
}

/// `cwd` 相对的路径解析（绝对路径原样返回）。
fn resolve(cwd: &Path, requested: &str) -> PathBuf {
    let path = Path::new(requested);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}
