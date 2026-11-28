//! CHANGELOG 解析与链接归一化（ref `utils/changelog.ts` 逐项转写）。
//!
//! `/changelog` 的数据来源是**随包**的 `CHANGELOG.md`（ref `config.ts:455-457`
//! 的 `getChangelogPath()` = `<packageDir>/CHANGELOG.md`）。pi-rs 自身没有这份文件
//! ——它属于 TS 包，所以这里按 `PI_PACKAGE_DIR` 解析；没有文件时按 ref 语义
//! 显示 `No changelog entries found.`（`interactive-mode.ts:6515`），不是报错。
//!
//! 本模块是纯函数（解析 + 改写字符串），文件读取在边界层。

/// 一条 changelog 条目（ref `ChangelogEntry`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangelogEntry {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// 该条目的原始 Markdown（**含** `## [x.y.z]` 标题行，ref 同样保留）。
    pub content: String,
}

impl ChangelogEntry {
    #[must_use]
    pub fn version(&self) -> String {
        format!("{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// ref `parseChangelog`（`changelog.ts:117-180`）。
///
/// 规则：
/// - 逐行扫；遇到 `## ` 开头 ⇒ 先把**上一条**收尾，再尝试从该行解析 `x.y.z`；
/// - 解析成功 ⇒ 开始新条目（当前行作为条目的第一行）；
/// - 解析失败 ⇒ 重置（该 `## ` 段及其后内容**不属于任何条目**）；
/// - 非 `## ` 行且当前在条目内 ⇒ 追加；
/// - 末尾收尾最后一条；
/// - 每个条目的 `content` 都 `trim()`（ref 的 `currentLines.join("\n").trim()`）。
#[must_use]
pub fn parse(text: &str) -> Vec<ChangelogEntry> {
    let mut entries: Vec<ChangelogEntry> = Vec::new();
    let mut current_lines: Vec<&str> = Vec::new();
    let mut current_version: Option<(u64, u64, u64)> = None;
    for line in text.split('\n') {
        if line.starts_with("## ") {
            if let Some((major, minor, patch)) = current_version {
                if !current_lines.is_empty() {
                    entries.push(ChangelogEntry {
                        major,
                        minor,
                        patch,
                        content: current_lines.join("\n").trim().to_string(),
                    });
                }
            }
            match parse_version_header(line) {
                Some(version) => {
                    current_version = Some(version);
                    current_lines = vec![line];
                }
                None => {
                    current_version = None;
                    current_lines.clear();
                }
            }
        } else if current_version.is_some() {
            current_lines.push(line);
        }
    }
    if let Some((major, minor, patch)) = current_version {
        if !current_lines.is_empty() {
            entries.push(ChangelogEntry {
                major,
                minor,
                patch,
                content: current_lines.join("\n").trim().to_string(),
            });
        }
    }
    entries
}

/// 从 `## [1.2.3]` / `## 1.2.3` 里取版本号（ref 的正则
/// `##\s+\[?(\d+)\.(\d+)\.(\d+)\]?`）。
fn parse_version_header(line: &str) -> Option<(u64, u64, u64)> {
    let rest = line.strip_prefix("## ")?;
    // `\s+`：`## ` 之后可能还有空格再跟 `[`。
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('[').unwrap_or(rest);
    let mut parts = rest.split('.');
    let major = parse_leading_digits(parts.next()?)?;
    let minor = parse_leading_digits(parts.next()?)?;
    let third = parts.next()?;
    let patch = parse_leading_digits(third)?;
    // `third` 的数字之后必须是 `]`、空白或行尾（避免把 `3.4.5.6` 当成 3.4.5）。
    let tail = third.trim_start_matches(|ch: char| ch.is_ascii_digit());
    let tail = tail.strip_prefix(']').unwrap_or(tail);
    if !tail.is_empty() && !tail.starts_with(char::is_whitespace) {
        return None;
    }
    Some((major, minor, patch))
}

fn parse_leading_digits(text: &str) -> Option<u64> {
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

/// 把条目按版本**升序**排列后拼接（ref `interactive-mode.ts:6509-6514`：
/// `allEntries.reverse().map(normalizeChangelogLinks).join("\n\n")`）。
///
/// `parse` 的结果是文件顺序（新→旧），`reverse()` 后是旧→新。
#[must_use]
pub fn render_all(entries: &[ChangelogEntry]) -> String {
    let mut reversed: Vec<&ChangelogEntry> = entries.iter().collect();
    reversed.reverse();
    reversed
        .iter()
        .map(|entry| normalize_links(&entry.content, entry))
        .collect::<Vec<String>>()
        .join("\n\n")
}

/// 读文件并解析（文件不存在 ⇒ 空，ref `parseChangelog` 对缺失文件返回 `[]`）。
///
/// # Errors
/// 本函数**不**返回错误：ref 在读取异常时只打一条警告并返回空列表
/// （`changelog.ts:120-124` 的 `catch`），调用方据此显示"没有条目"。
#[must_use]
pub fn parse_file(path: &std::path::Path) -> Vec<ChangelogEntry> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(_error) => Vec::new(),
    }
}

const GITHUB_REPO: &str = "earendil-works/pi";
const CHANGELOG_LINK_BASE_PATH: &str = "packages/coding-agent";

/// ref `normalizeChangelogLinks`（`changelog.ts:100-106`）。
///
/// 把 changelog 里的**相对链接**改写成指向固定 tag 的 GitHub 绝对 URL。
#[must_use]
pub fn normalize_links(markdown: &str, version: &ChangelogEntry) -> String {
    let tag = format!("v{}", version.version());
    let mut out = String::with_capacity(markdown.len());
    let mut rest = markdown;
    while let Some(open) = rest.find("](") {
        // 找 `[` 前的文本起点：只需把 `](target)` 的 target 换掉。
        let (before, after_open) = rest.split_at(open + 2);
        out.push_str(before);
        let Some(close) = after_open.find(')') else {
            out.push_str(after_open);
            return out;
        };
        let raw_target = &after_open[..close];
        // `target` 可能含 ` "title"`（ref 的 `INLINE_MARKDOWN_LINK_RE` 把它留在 suffix 里）。
        let (target, suffix) = split_target_suffix(raw_target);
        out.push_str(&normalize_link_target(target, &tag));
        out.push_str(suffix);
        out.push(')');
        rest = &after_open[close + 1..];
    }
    out.push_str(rest);
    out
}

/// 把 `path "title"` 拆成 `(path, " \"title\"")`。
fn split_target_suffix(raw: &str) -> (&str, &str) {
    match raw.find(char::is_whitespace) {
        Some(index) => (&raw[..index], &raw[index..]),
        None => (raw, ""),
    }
}

/// ref `normalizeChangelogLinkTarget`（`changelog.ts:74-98`）。
fn normalize_link_target(target: &str, tag: &str) -> String {
    let repo_url = format!("https://github.com/{GITHUB_REPO}");
    // 旧仓库名（`badlogic/pi-mono`、`earendil-works/pi-mono`）换成新仓库。
    let mut canonical = replace_legacy_repo(target);
    // 浮动分支引用（`/blob/main/`、`/tree/master/`）改写成固定 tag。
    for route in ["blob", "tree"] {
        for branch in ["main", "master"] {
            let prefix = format!("{repo_url}/{route}/{branch}/");
            if let Some(rest) = canonical.strip_prefix(&prefix) {
                canonical = format!("{repo_url}/{route}/{tag}/{rest}");
            }
        }
    }
    if canonical.starts_with('#') || canonical.starts_with("//") || has_url_scheme(&canonical) {
        return canonical;
    }
    let (path_part, query, fragment) = split_local_target(&canonical);
    if path_part.is_empty() {
        return canonical;
    }
    let Some(repository_path) = resolve_repository_path(&path_part) else {
        return canonical;
    };
    let route = if is_directory_target(&path_part, &repository_path) {
        "tree"
    } else {
        "blob"
    };
    format!(
        "{repo_url}/{route}/{tag}/{}{query}{fragment}",
        percent_encode(&repository_path)
    )
}

fn replace_legacy_repo(target: &str) -> String {
    for legacy in [
        "https://github.com/badlogic/pi-mono",
        "https://github.com/earendil-works/pi-mono",
    ] {
        if let Some(rest) = target.strip_prefix(legacy) {
            if rest.is_empty() || rest.starts_with('/') {
                return format!("https://github.com/{GITHUB_REPO}{rest}");
            }
        }
    }
    target.to_string()
}

fn has_url_scheme(target: &str) -> bool {
    let Some(colon) = target.find(':') else {
        return false;
    };
    let scheme = &target[..colon];
    !scheme.is_empty()
        && scheme
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
}

/// 拆 `path?query#fragment`（ref `splitLocalTarget`）。
fn split_local_target(target: &str) -> (String, String, String) {
    let (before_hash, fragment) = match target.find('#') {
        Some(index) => (&target[..index], &target[index..]),
        None => (target, ""),
    };
    let (path_part, query) = match before_hash.find('?') {
        Some(index) => (&before_hash[..index], &before_hash[index..]),
        None => (before_hash, ""),
    };
    (
        path_part.to_string(),
        query.to_string(),
        fragment.to_string(),
    )
}

/// 把相对路径解析成仓库内路径（ref `resolveRepositoryPath`）；越界 ⇒ `None`。
fn resolve_repository_path(path_part: &str) -> Option<String> {
    let normalized = path_part.replace('\\', "/");
    let joined = if let Some(absolute) = normalized.strip_prefix('/') {
        posix_normalize(absolute)
    } else {
        posix_normalize(&format!("{CHANGELOG_LINK_BASE_PATH}/{normalized}"))
    };
    if joined == "." || joined == ".." || joined.starts_with("../") {
        return None;
    }
    Some(joined)
}

/// 词法归一并去掉末尾 `/`（近似 `path.posix.normalize`）。
fn posix_normalize(path: &str) -> String {
    // 开头的 `..` **必须保留**：`path.posix.normalize` 对
    // `packages/coding-agent/../../../etc` 返回 `../etc`，而不是 `etc`。
    // 否则下面 `resolve_repository_path` 的越界判断永远不会命中，
    // `../../../etc/passwd` 会被改写成指向仓库根的链接（实测）。
    let mut leading_parents = 0_usize;
    let mut parts: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if parts.is_empty() {
                    leading_parents += 1;
                } else {
                    parts.pop();
                }
            }
            other => parts.push(other),
        }
    }
    let mut out = String::new();
    for _ in 0..leading_parents {
        out.push_str("../");
    }
    out.push_str(&parts.join("/"));
    if out.is_empty() {
        ".".to_string()
    } else {
        out
    }
}

/// 判断是否目录链接（ref `isDirectoryTarget`：以 `/` 结尾，或末段不含 `.`）。
fn is_directory_target(original: &str, repository_path: &str) -> bool {
    if original.ends_with('/') {
        return true;
    }
    let basename = repository_path
        .rsplit('/')
        .next()
        .unwrap_or(repository_path);
    !basename.contains('.')
}

/// 只对路径做最小转义（ref 用 `encodeURI`，保留 `/` 与 `-._~`）。
fn percent_encode(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.bytes() {
        let keep = byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'/' | b'-' | b'_' | b'.' | b'~' | b'!' | b'*' | b'\'' | b'(' | b')'
            );
        if keep {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# Changelog

## [0.2.0] - 2026-02-01

### New Features

- Added thing

## [0.1.0] - 2026-01-01

- First release
";

    #[test]
    fn parses_entries_newest_first() {
        let entries = parse(SAMPLE);
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert_eq!(entries[0].version(), "0.2.0");
        assert_eq!(entries[1].version(), "0.1.0");
        assert!(
            entries[0].content.starts_with("## [0.2.0]"),
            "content 应含标题行：{:?}",
            entries[0].content
        );
        assert!(entries[0].content.contains("Added thing"));
    }

    #[test]
    fn header_without_version_resets_the_entry() {
        // ref `:157-166`：`## ` 行解析不出版本 ⇒ 重置，该段不入任何条目。
        let text = "## 0.1.0\n\n- one\n\n## Unreleased\n\n- skipped\n\n## 0.0.9\n\n- nine\n";
        let entries = parse(text);
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert_eq!(entries[0].version(), "0.1.0");
        assert_eq!(entries[1].version(), "0.0.9");
        assert!(
            !entries
                .iter()
                .any(|entry| entry.content.contains("skipped")),
            "Unreleased 段不应进入任何条目"
        );
    }

    #[test]
    fn missing_file_yields_no_entries() {
        assert!(parse("").is_empty());
        assert!(parse("# Changelog\n\nNo versions here.\n").is_empty());
    }

    #[test]
    fn render_orders_oldest_first_and_joins_with_blank_line() {
        let entries = parse(SAMPLE);
        let rendered = render_all(&entries);
        let old = rendered.find("0.1.0").expect("有 0.1.0");
        let new = rendered.find("0.2.0").expect("有 0.2.0");
        assert!(old < new, "应按旧→新排列：{rendered}");
        assert!(rendered.contains("\n\n"), "条目之间应空一行");
    }

    fn entry(major: u64, minor: u64, patch: u64) -> ChangelogEntry {
        ChangelogEntry {
            major,
            minor,
            patch,
            content: String::new(),
        }
    }

    #[test]
    fn relative_links_become_pinned_github_urls() {
        let version = entry(0, 2, 0);
        let out = normalize_links("see [docs](docs/cli.md) for more", &version);
        assert_eq!(
            out,
            "see [docs](https://github.com/earendil-works/pi/blob/v0.2.0/packages/coding-agent/docs/cli.md) for more"
        );
    }

    #[test]
    fn directory_links_use_tree_and_keep_trailing_slash_semantics() {
        let version = entry(1, 0, 0);
        let out = normalize_links("[dir](src/foo/)", &version);
        assert!(
            out.contains("/tree/v1.0.0/packages/coding-agent/src/foo"),
            "目录链接应走 /tree/：{out}"
        );
        // 末段不含 `.` 也当目录（ref `isDirectoryTarget`）。
        let out = normalize_links("[dir](src/bar)", &version);
        assert!(out.contains("/tree/v1.0.0/"), "{out}");
    }

    #[test]
    fn absolute_and_fragment_links_are_left_alone() {
        let version = entry(0, 1, 0);
        assert_eq!(
            normalize_links("[a](https://example.com/x)", &version),
            "[a](https://example.com/x)"
        );
        assert_eq!(normalize_links("[a](#section)", &version), "[a](#section)");
        assert_eq!(normalize_links("[a](//cdn/x)", &version), "[a](//cdn/x)");
    }

    #[test]
    fn floating_branch_refs_are_pinned_to_the_tag() {
        let version = entry(0, 3, 0);
        let out = normalize_links(
            "[f](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/a.md)",
            &version,
        );
        assert!(out.contains("/blob/v0.3.0/"), "应固定到 tag：{out}");
        assert!(!out.contains("/blob/main/"), "{out}");
    }

    #[test]
    fn legacy_repo_names_are_rewritten() {
        let version = entry(0, 1, 0);
        let out = normalize_links("[f](https://github.com/badlogic/pi-mono/x.md)", &version);
        assert!(
            out.contains("github.com/earendil-works/pi/"),
            "旧仓库名应换成新仓库：{out}"
        );
    }

    #[test]
    fn escaping_paths_above_the_root_is_refused() {
        let version = entry(0, 1, 0);
        let out = normalize_links("[f](../../../etc/passwd)", &version);
        assert_eq!(
            out, "[f](../../../etc/passwd)",
            "越界路径应原样保留（ref `resolveRepositoryPath` 返回 undefined）"
        );
    }

    #[test]
    fn link_titles_are_preserved() {
        let version = entry(0, 1, 0);
        let out = normalize_links("[f](docs/a.md \"Title\")", &version);
        assert!(out.ends_with("\"Title\")"), "标题应保留：{out}");
    }
}
