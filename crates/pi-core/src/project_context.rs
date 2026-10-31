//! 项目指令文件（`AGENTS.md` 等）的**发现规则**。
//!
//! 依据：`ref/pi-0.87.1/packages/coding-agent/src/core/resource-loader.ts:71-157`。
//! - `:72` 候选顺序（同目录内**首个命中即用**）；`:77-79` 非普通文件**无告警** `continue`；
//! - `:75` `existsSync(filePath)` 为假 ⇒ 静默跳过；
//! - `:80-83` 读出内容：`stripBom(readFileSync(filePath, "utf-8"))`；
//! - `:84-86` 读/stat 抛错 ⇒ `console.error("Warning: Could not read <path>: <error>")` 后**继续下一候选**；
//! - `:129-133` **先全局** `agentDir`；`:135-154` **再祖先**（cwd 逐级 `dirname` 到根，命中者 `unshift`
//!   ⇒ 输出顺序 = 最顶层祖先 → … → cwd）；`seenPaths` 按路径去重。
//!
//! ## 读文件语义（P6-1c 对齐，实测见 `notes/progress.md`）
//!
//! Node 的 `readFileSync(path, "utf-8")` **不会**因非法 UTF-8 抛错，而是以 U+FFFD 有损解码
//! （实测 10 条字节样本：合法/非法/截断/过长/代理/BOM/空/NUL 全部与
//! `String::from_utf8_lossy` 的码点序列逐条一致）；Rust 的 `read_to_string` 则返回
//! `Err(InvalidData)`，会**跳过**该候选 ⇒ 本模块改为「读字节 → 有损转换 → 去 BOM」。
//!
//! Node 的 `existsSync` 对**任何** stat 失败都返回 `false`（实测 ELOOP ⇒ `false`）⇒
//! ref 的 `:84-86` 实际只在**读**失败时进入（如 EIO），stat 失败表现为静默跳过。
//!
//! **未实现（已知限制，见 notes/decisions.md）**：linked worktree 的"遮蔽"分支
//! （ref `:101-117` 的 `findShadowedContextFile`）需要 git 内省，本批不做。
//!
//! 本模块的公开函数是纯 I/O 边界函数，不碰全局状态；找到的告警一并返回给调用方
//! （**不直接写 stderr**），由 CLI 决定输出位置。

use std::path::{Path, PathBuf};

/// 每个目录里按顺序尝试的候选文件名（ref `resource-loader.ts:72`，逐字一致）。
pub const CONTEXT_FILE_CANDIDATES: [&str; 5] = [
    "AGENTS.override.md",
    "AGENTS.md",
    "AGENTS.MD",
    "CLAUDE.md",
    "CLAUDE.MD",
];

/// 去掉 UTF-8 BOM（ref 的 `stripBom`，等价 `resource-loader.ts:55-69` 的 BOM 处理）。
fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

/// 与 Node `readFileSync(path, "utf-8")` 等价的解码：读出的字节**有损**转为 UTF-8
/// （非法序列替换为 U+FFFD），再去掉前导 BOM（顺序与 ref 的 `stripBom(readFileSync(...))` 一致）。
fn decode_like_node(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    strip_bom(&text).to_string()
}

/// 发现结果：文件清单（`(绝对路径, 内容)`，顺序即 ref 的输出顺序）+ 告警文本。
///
/// 告警**不由本模块输出**：print / rpc 两模式共用同一份 sections，输出位置交调用方决定。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectContextFiles {
    /// 按 ref `:129-157` 的顺序：全局目录 → 最顶层祖先 → … → cwd。
    pub files: Vec<(String, String)>,
    /// 读失败告警，逐字照 ref `:85` 的前缀（`<error>` 措辞见模块文档的已知差异）。
    pub warnings: Vec<String>,
}

/// 在 `dir` 里按候选顺序找**首个**可读的普通文件（ref `:71-90`）。
///
/// 读失败时把告警追加进 `warnings` 并继续下一候选（ref `:84-86`）。
#[must_use]
pub fn load_context_file_from_dir(
    dir: &Path,
    warnings: &mut Vec<String>,
) -> Option<(PathBuf, String)> {
    for name in CONTEXT_FILE_CANDIDATES {
        let path = dir.join(name);
        // ref `:75 existsSync(filePath)`：Node 的 `existsSync` 对任何 stat 失败都返回 false
        // （实测 ELOOP ⇒ false），因此 stat/metadata 失败一律**静默跳过**，不进告警分支。
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        // ref `:77-79`：存在但不是普通文件 ⇒ 无告警 continue，试下一候选。
        if !meta.is_file() {
            continue;
        }
        // ref `:80-83` 的 readFileSync 若抛错 ⇒ `:84-86` 告警后继续。
        match std::fs::read(&path) {
            Ok(bytes) => return Some((path, decode_like_node(&bytes))),
            Err(error) => {
                warnings.push(format!(
                    "Warning: Could not read {}: {error}",
                    path.display()
                ));
            }
        }
    }
    None
}

/// ref `:119-157`：先全局目录、再祖先目录（自顶向下），按路径去重。
#[must_use]
pub fn load_project_context_files(cwd: &Path, agent_dir: &Path) -> ProjectContextFiles {
    let mut result = ProjectContextFiles::default();
    let mut seen: Vec<PathBuf> = Vec::new();

    let mut push = |path: PathBuf, content: String, result: &mut ProjectContextFiles| {
        if seen.contains(&path) {
            return;
        }
        result
            .files
            .push((path.to_string_lossy().to_string(), content));
        seen.push(path);
    };

    // ① 全局目录优先（ref `:129-133`）。
    if let Some((path, content)) = load_context_file_from_dir(agent_dir, &mut result.warnings) {
        push(path, content, &mut result);
    }

    // ② 祖先目录：从 cwd 向上收集，最后反转（等价 ref `:135-154` 的 `unshift`）。
    let mut ancestors: Vec<(PathBuf, String)> = Vec::new();
    let mut current = Some(cwd.to_path_buf());
    while let Some(dir) = current {
        if let Some(entry) = load_context_file_from_dir(&dir, &mut result.warnings) {
            ancestors.push(entry);
        }
        current = dir.parent().map(Path::to_path_buf);
    }
    for (path, content) in ancestors.into_iter().rev() {
        push(path, content, &mut result);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{load_context_file_from_dir, load_project_context_files, CONTEXT_FILE_CANDIDATES};
    use std::path::Path;

    /// 写文件；`bytes` 允许非法 UTF-8（N3 的样本同源）。
    fn write_bytes(dir: &Path, name: &str, bytes: &[u8]) {
        std::fs::create_dir_all(dir).expect("建目录");
        std::fs::write(dir.join(name), bytes).expect("写文件");
    }

    fn write(dir: &Path, name: &str, content: &str) {
        write_bytes(dir, name, content.as_bytes());
    }

    fn names(result: &super::ProjectContextFiles) -> Vec<String> {
        result
            .files
            .iter()
            .map(|(path, _)| {
                Path::new(path)
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_default()
            })
            .collect()
    }

    /// **a)** 祖先自顶向下 + 全局在最前（ref `:129-154`）。
    #[test]
    fn ancestors_top_down_with_global_first() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let agent = tmp.path().join("agent");
        let top = tmp.path().join("top");
        let mid = top.join("mid");
        let leaf = mid.join("leaf");
        write(&agent, "CLAUDE.md", "全局");
        write(&top, "AGENTS.md", "顶层");
        write(&mid, "AGENTS.md", "中层");
        write(&leaf, "AGENTS.override.md", "叶层");
        let result = load_project_context_files(&leaf, &agent);
        // 有效产出闸门：三条目标条目确实产出（不是空对空）
        assert!(
            result.files.len() >= 4,
            "应产出 4 条，实际 {:?}",
            names(&result)
        );
        assert_eq!(
            names(&result),
            vec![
                "CLAUDE.md".to_string(),
                "AGENTS.md".to_string(),
                "AGENTS.md".to_string(),
                "AGENTS.override.md".to_string(),
            ]
        );
        assert_eq!(
            result
                .files
                .iter()
                .map(|(_, c)| c.as_str())
                .collect::<Vec<_>>(),
            vec!["全局", "顶层", "中层", "叶层"],
            "顺序必须是 全局 → 顶层 → 中层 → 叶层"
        );
        assert!(result.warnings.is_empty());
    }

    /// **b)** 按路径去重：`agent_dir` 与某个祖先目录**同路径**（ref `:127` 的 `seenPaths`）。
    #[test]
    fn dedups_when_agent_dir_is_an_ancestor() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let mid = tmp.path().join("top").join("mid");
        let leaf = mid.join("leaf");
        write(&mid, "AGENTS.md", "中层");
        write(&leaf, "AGENTS.md", "叶层");
        let result = load_project_context_files(&leaf, &mid);
        assert_eq!(
            names(&result),
            vec!["AGENTS.md".to_string(), "AGENTS.md".to_string()]
        );
        assert_eq!(
            result
                .files
                .iter()
                .map(|(_, c)| c.as_str())
                .collect::<Vec<_>>(),
            vec!["中层", "叶层"],
            "中层只应出现一次（全局与祖先同路径 ⇒ 去重）"
        );
    }

    /// **c)** 同名目录被跳过后落到下一候选（ref `:77-79`）。
    #[test]
    fn skips_directory_and_falls_through_to_next_candidate() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let cwd = tmp.path().join("work");
        std::fs::create_dir_all(cwd.join("AGENTS.override.md")).expect("建同名目录");
        write(&cwd, "AGENTS.md", "第二候选");
        let mut warnings = Vec::new();
        let found = load_context_file_from_dir(&cwd, &mut warnings).expect("应命中第二候选");
        assert_eq!(
            found.0.file_name().expect("文件名").to_string_lossy(),
            "AGENTS.md"
        );
        assert_eq!(found.1, "第二候选");
        assert!(warnings.is_empty(), "非普通文件不得告警");
    }

    /// **d)** 非法 UTF-8 **仍被采用**且含 U+FFFD（Node `readFileSync(…, "utf-8")` 语义）。
    #[test]
    fn invalid_utf8_is_adopted_lossily() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let cwd = tmp.path().join("work");
        write_bytes(&cwd, "AGENTS.md", b"ok\xff\xfeend");
        let mut warnings = Vec::new();
        let found = load_context_file_from_dir(&cwd, &mut warnings).expect("非法 UTF-8 也应被采用");
        assert_eq!(found.1, "ok\u{fffd}\u{fffd}end");
        assert!(
            found.1.contains('\u{fffd}'),
            "内容必须含 U+FFFD 替换字符，实际 {:?}",
            found.1
        );
        assert!(warnings.is_empty(), "非法 UTF-8 不是读失败，不得告警");
    }

    /// **e)** BOM、空文件、BOM 加非法（顺序：有损解码 → 去 BOM，ref `:80-83`）。
    #[test]
    fn bom_empty_and_bom_with_invalid_bytes() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let bom = tmp.path().join("bom");
        write_bytes(&bom, "AGENTS.md", b"\xef\xbb\xbf# hi");
        let mut warnings = Vec::new();
        assert_eq!(
            load_context_file_from_dir(&bom, &mut warnings)
                .expect("BOM 样本")
                .1,
            "# hi",
            "前导 BOM 必须去掉"
        );

        let empty = tmp.path().join("empty");
        write_bytes(&empty, "AGENTS.md", b"");
        let found = load_context_file_from_dir(&empty, &mut warnings).expect("空文件也应产出");
        assert_eq!(found.1, "", "空文件产出空内容");

        let bom_invalid = tmp.path().join("bom-invalid");
        write_bytes(&bom_invalid, "AGENTS.md", b"\xef\xbb\xbf\xff\xfe");
        assert_eq!(
            load_context_file_from_dir(&bom_invalid, &mut warnings)
                .expect("BOM+非法")
                .1,
            "\u{fffd}\u{fffd}",
            "BOM 去掉后剩下的非法字节各自替换为 U+FFFD"
        );
        assert!(warnings.is_empty());
    }

    /// **f)** 候选顺序五级各一条（ref `:72`）：单放一层各命中自己；五者同放取 `AGENTS.override.md`。
    #[test]
    fn candidate_order_five_levels() {
        assert_eq!(
            CONTEXT_FILE_CANDIDATES,
            [
                "AGENTS.override.md",
                "AGENTS.md",
                "AGENTS.MD",
                "CLAUDE.md",
                "CLAUDE.MD"
            ]
        );
        let tmp = tempfile::tempdir().expect("临时目录");
        let mut warnings = Vec::new();
        for name in CONTEXT_FILE_CANDIDATES {
            let dir = tmp.path().join(name.replace('.', "_"));
            write(&dir, name, name);
            let found = load_context_file_from_dir(&dir, &mut warnings).expect("单独一层应命中");
            assert_eq!(
                found.0.file_name().expect("文件名").to_string_lossy(),
                name,
                "只有 {name} 时必须命中它"
            );
        }
        let all = tmp.path().join("all");
        for name in CONTEXT_FILE_CANDIDATES {
            write(&all, name, name);
        }
        let found = load_context_file_from_dir(&all, &mut warnings).expect("五者同放");
        assert_eq!(
            found.0.file_name().expect("文件名").to_string_lossy(),
            "AGENTS.override.md",
            "五者同放必须取候选第 1 项"
        );
        assert!(warnings.is_empty());
    }

    /// **g)** 读失败 ⇒ 告警 + 继续下一候选（ref `:84-86`）。
    ///
    /// 触发点在 root 下同样有效：候选路径是指向 `/proc/self/mem` 的符号链接
    /// （`metadata` 成功且 `is_file()` 为真 = 等价 `existsSync` 为真，`read` 必然 EIO）。
    #[cfg(unix)]
    #[test]
    fn read_failure_warns_and_continues() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let cwd = tmp.path().join("work");
        std::fs::create_dir_all(&cwd).expect("建目录");
        let target = cwd.join("AGENTS.md");
        std::os::unix::fs::symlink("/proc/self/mem", &target).expect("建符号链接");
        // 构造自检（闸门）：existsSync 等价为真、是普通文件、读确实失败
        assert!(
            std::fs::metadata(&target)
                .expect("metadata 应成功")
                .is_file(),
            "构造前提：metadata 成功且 is_file() 为真"
        );
        assert!(
            std::fs::read(&target).is_err(),
            "构造前提：root 下 read 也必须真的失败"
        );
        let mut warnings = Vec::new();
        assert!(
            load_context_file_from_dir(&cwd, &mut warnings).is_none(),
            "读失败且无后续候选 ⇒ 不该产出条目"
        );
        assert_eq!(warnings.len(), 1, "应产生恰好一条告警，实际 {warnings:?}");
        assert!(
            warnings[0].starts_with("Warning: Could not read "),
            "告警前缀须逐字照 ref `:85`，实际 {:?}",
            warnings[0]
        );
        assert!(
            warnings[0].contains("AGENTS.md"),
            "告警须含路径，实际 {:?}",
            warnings[0]
        );

        // 有后续候选时：告警 + 继续，产出后面的 AGENTS.MD
        write(&cwd, "AGENTS.MD", "后备候选");
        let mut warnings = Vec::new();
        let found = load_context_file_from_dir(&cwd, &mut warnings).expect("应落到后备候选");
        assert_eq!(found.1, "后备候选");
        assert_eq!(warnings.len(), 1, "告警一次，实际 {warnings:?}");
    }
}
