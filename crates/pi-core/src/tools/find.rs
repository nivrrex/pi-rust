//! `find` 工具：按 glob 找文件（委托外部 `fd`，不自动下载）。
//!
//! 对应上游 `packages/coding-agent/src/core/tools/find.ts`：
//! - schema：`pattern`（glob）/ `path` / `limit`（默认 1000）；
//! - 调用：`fd --glob --color=never --hidden [--no-require-git] --max-results N -- <pattern> <path>`
//!   （`:182-200`；不在 git 仓库里时加 `--no-require-git`，否则 fd 会忽略 .gitignore）；
//! - 输出：相对路径逐行；无结果 → `No files found matching pattern`（`:136/261`）；
//! - 达到 limit → 追加 `<N> results limit reached. Use limit=<2N> for more, or refine pattern`
//!   （`:283`）；字节超限 → `<size> limit reached`（`:288`）。
//!
//! 与上游的唯一区别：`fd` 缺失时不尝试下载，直接返回带安装指引的错误。

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::read_text::{format_size, truncate_head, DEFAULT_MAX_BYTES};
use super::tool::{Tool, ToolContext, ToolResult};

/// 默认结果上限（`find.ts:36`）。
const DEFAULT_LIMIT: usize = 1000;

/// `find` 工具。
#[derive(Debug, Default)]
pub struct FindTool;

/// 参数 schema（`find.ts:26-33`）。
#[must_use]
pub fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "pattern": {"type": "string", "description": "Glob pattern to match files, e.g. '*.ts', '**/*.json', or 'src/**/*.spec.ts'"},
            "path": {"type": "string", "description": "Directory to search in (default: current directory)"},
            "limit": {"type": "number", "description": "Maximum number of results (default: 1000)"}
        },
        "required": ["pattern"]
    })
}

impl Tool for FindTool {
    fn name(&self) -> &str {
        "find"
    }

    fn description(&self) -> &str {
        "Find files by glob pattern (respects .gitignore). Returns matching paths, one per line."
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
        "--glob".to_string(),
        "--color=never".to_string(),
        "--hidden".to_string(),
    ];
    if !inside_git_repo(&target) {
        args.push("--no-require-git".to_string());
    }
    args.push("--max-results".to_string());
    args.push(limit.to_string());
    // ref `find.ts:203-211`：`fd --glob` 默认只匹配**basename**；模式里带 `/`
    // 时必须加 `--full-path`，且不以 `/`/`**/` 开头时要补 `**/` 前缀，
    // 否则 fd 会把 `src/**/*.txt` 当作字面 basename 而不匹配任何文件。
    let mut effective_pattern = pattern.to_string();
    if pattern.contains('/') {
        args.push("--full-path".to_string());
        if !pattern.starts_with('/') && !pattern.starts_with("**/") && pattern != "**" {
            effective_pattern = format!("**/{pattern}");
        }
    }
    args.push("--".to_string());
    args.push(effective_pattern);
    args.push(target.display().to_string());

    let Some(binary) = fd_binary() else {
        return ToolResult::error(
            "`fd` (fd-find) is not available. Install it first, e.g. `apt install fd-find` \
             (Debian/Ubuntu, binary may be named `fdfind`), then retry.",
        );
    };
    let output = match Command::new(binary).args(&args).output() {
        Ok(output) => output,
        Err(error) => return ToolResult::error(format!("Failed to run fd: {error}")),
    };
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines: Vec<String> = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    if !output.status.success() && lines.is_empty() {
        return ToolResult::error(if stderr.is_empty() {
            format!("fd exited with code {}", output.status.code().unwrap_or(2))
        } else {
            stderr
        });
    }
    if lines.is_empty() {
        return ToolResult::text("No files found matching pattern");
    }
    let limit_reached = lines.len() >= limit;
    lines.truncate(limit);
    let raw = lines.join("\n");
    let truncation = truncate_head(&raw, usize::MAX, DEFAULT_MAX_BYTES);
    let mut text = truncation.content;
    let mut notices: Vec<String> = Vec::new();
    if limit_reached {
        notices.push(format!(
            "{limit} results limit reached. Use limit={} for more, or refine pattern",
            limit * 2
        ));
    }
    if truncation.truncated {
        notices.push(format!("{} limit reached", format_size(DEFAULT_MAX_BYTES)));
    }
    if !notices.is_empty() {
        text.push_str("\n\n[");
        text.push_str(&notices.join(". "));
        text.push(']');
    }
    ToolResult::text(text)
}

/// 找 `fd` 可执行文件：优先 `PI_FD_BIN`，其次 `fd`，最后 Debian 的 `fdfind`。
///
/// Debian/Ubuntu 的包名是 `fd-find`，二进制叫 `fdfind`（避免与 `find` 系工具冲突）。
#[must_use]
pub fn fd_binary() -> Option<String> {
    if let Ok(explicit) = std::env::var("PI_FD_BIN") {
        if !explicit.trim().is_empty() {
            return Some(explicit);
        }
    }
    for candidate in ["fd", "fdfind"] {
        if Command::new(candidate)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            return Some(candidate.to_string());
        }
    }
    None
}

/// 目标是否在 git 仓库内（向上找 `.git`；`find.ts:186-196`）。
fn inside_git_repo(start: &Path) -> bool {
    let mut current = start.to_path_buf();
    loop {
        if current.join(".git").exists() {
            return true;
        }
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => return false,
        }
    }
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
