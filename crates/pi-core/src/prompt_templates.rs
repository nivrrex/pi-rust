//! prompt template 的发现、加载、`/名字 参数` 展开与参数替换。
//!
//! 依据 `ref/pi-0.87.1/packages/coding-agent/src/core/prompt-templates.ts`：
//! - 替换 `substitute_args` 照 `:71-103`（五种形态：`${N:-默认}`、`${@:-默认}`/`${ARGUMENTS:-默认}`、
//!   `${@:start}`/`${@:start:len}`、`$ARGUMENTS`/`$@`、`$N`）；
//! - 分词 `parse_command_args` 照 `:25-56`（手写引号感知；引号不保留；无转义；空引号不产参数）；
//! - 加载 `load_template_from_file` 照 `:105-154`（读/解析失败 ⇒ warning + null；名字 = 文件名去 `.md`）、
//!   `load_templates_from_dir` 照 `:159-198`（**只扫本层** `.md`，符号链接指向文件才算，readdir 出错 ⇒ 空）、
//!   `load_prompt_templates` 照 `:222-298`（`<agentDir>/prompts` 先、`<cwd>/.pi/prompts` 后、显式路径最后；
//!   **只 push 不去重** ⇒ 展开时 `find` 先到先得）；
//! - 展开 `expand_prompt_template` 照 `:304-320`（整串 `^\/([^\s]+)(?:\s+([\s\S]*))?$`；未命中原样返回）。
//!
//! **已知限制**：与 skills 同型——项目目录 `<cwd>/.pi/prompts` 需要项目受信任（ref `resource-loader.ts:1029/1043`），
//! pi-rs 无信任机制 ⇒ 一律视为未受信任、不加载（见 `notes/decisions.md`）。本模块不写 stderr，诊断随结果返回。

use std::path::{Path, PathBuf};

/// 一个 prompt template（ref `:12-19`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptTemplate {
    /// 命令名（= 文件名去掉 `.md`，ref `:129`）。
    pub name: String,
    /// 模板正文（frontmatter 之后的部分）。
    pub content: String,
    /// 描述：frontmatter.description 优先，否则取正文**首个非空行**，超过 60 字符截断并加 `...`
    /// （ref `:131-140`）。`get_commands` 的 `description` 字段用它。
    pub description: String,
    pub file_path: String,
}

/// 诊断（与 skills 同型：warning 文本 + 路径）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub message: String,
    pub path: String,
}

/// 加载结果（ref `:211-214`）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TemplatesResult {
    pub templates: Vec<PromptTemplate>,
    pub diagnostics: Vec<Diagnostic>,
}

/// 分词（ref `:25-56`）：引号只切换状态且不保留；引号态外按空白切分；空片段不产参数。
#[must_use]
pub fn parse_command_args(args_string: &str) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_quote: Option<char> = None;
    for ch in args_string.chars() {
        if let Some(quote) = in_quote {
            if ch == quote {
                in_quote = None;
            } else {
                current.push(ch);
            }
        } else if ch == '"' || ch == '\'' {
            in_quote = Some(ch);
        } else if ch.is_whitespace() {
            if !current.is_empty() {
                args.push(std::mem::take(&mut current));
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}

fn digits_at(chars: &[char], start: usize) -> Option<(usize, String)> {
    let mut end = start;
    while end < chars.len() && chars[end].is_ascii_digit() {
        end += 1;
    }
    if end == start {
        None
    } else {
        Some((end - start, chars[start..end].iter().collect()))
    }
}

/// `$N` / `$ARGUMENTS` / `$@`（ref 正则的第三个分支）。
fn parse_simple(rest: &[char], args: &[String], all_args: &str) -> Option<(usize, String)> {
    let text: String = rest.iter().take(9).collect();
    if text.starts_with("ARGUMENTS") {
        return Some((9, all_args.to_string()));
    }
    if !rest.is_empty() && rest[0] == '@' {
        return Some((1, all_args.to_string()));
    }
    if let Some((len, digits)) = digits_at(rest, 0) {
        let index = digits.parse::<usize>().unwrap_or(0).saturating_sub(1);
        return Some((len, args.get(index).cloned().unwrap_or_default()));
    }
    None
}

/// `${…}` 两种形态（ref 正则的前两个分支）。
fn parse_braced(rest: &[char], args: &[String], all_args: &str) -> Option<(usize, String)> {
    // 分支一：`${(\d+|ARGUMENTS|@):-([^}]*)}`
    let target_len = if rest.len() >= 9 && rest.iter().take(9).collect::<String>() == "ARGUMENTS" {
        Some(9)
    } else if !rest.is_empty() && rest[0] == '@' {
        Some(1)
    } else {
        digits_at(rest, 0).map(|(len, _)| len)
    };
    if let Some(len) = target_len {
        let has_default = rest.len() > len + 2 && rest[len] == ':' && rest[len + 1] == '-';
        if has_default {
            let tail = &rest[len + 2..];
            if let Some(end) = tail.iter().position(|c| *c == '}') {
                let default: String = tail[..end].iter().collect();
                let target: String = rest[..len].iter().collect();
                let value = if target == "@" || target == "ARGUMENTS" {
                    all_args.to_string()
                } else {
                    args.get(target.parse::<usize>().unwrap_or(0).saturating_sub(1))
                        .cloned()
                        .unwrap_or_default()
                };
                let replacement = if value.is_empty() { default } else { value };
                return Some((len + 2 + end + 1, replacement));
            }
        }
    }
    // 分支二：`${@:(\d+)(?::(\d+))?}`
    if !rest.is_empty() && rest[0] == '@' && rest.len() > 1 && rest[1] == ':' {
        let (len1, start_text) = digits_at(rest, 2)?;
        let mut cursor = 2 + len1;
        let mut length_text: Option<String> = None;
        if cursor < rest.len() && rest[cursor] == ':' {
            if let Some((len2, text)) = digits_at(rest, cursor + 1) {
                length_text = Some(text);
                cursor += 1 + len2;
            }
        }
        if cursor < rest.len() && rest[cursor] == '}' {
            // ref `:81-84`：`start = parseInt(start)-1; if (start < 0) start = 0;`
            let start = start_text.parse::<usize>().unwrap_or(1).saturating_sub(1);
            let slice: Vec<String> = match &length_text {
                Some(text) => {
                    let length = text.parse::<usize>().unwrap_or(0);
                    args.iter()
                        .skip(start)
                        .take(length)
                        .cloned()
                        .collect::<Vec<_>>()
                }
                None => args.iter().skip(start).cloned().collect::<Vec<_>>(),
            };
            return Some((cursor + 1, slice.join(" ")));
        }
    }
    None
}

/// 参数替换（ref `:71-103`）。
#[must_use]
pub fn substitute_args(content: &str, args: &[String]) -> String {
    let all_args = args.join(" ");
    let chars: Vec<char> = content.chars().collect();
    let mut out = String::new();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] != '$' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let rest = &chars[i + 1..];
        if !rest.is_empty() && rest[0] == '{' {
            if let Some((consumed, replacement)) = parse_braced(&rest[1..], args, &all_args) {
                out.push_str(&replacement);
                i += 2 + consumed;
                continue;
            }
        }
        if let Some((consumed, replacement)) = parse_simple(rest, args, &all_args) {
            out.push_str(&replacement);
            i += 1 + consumed;
            continue;
        }
        out.push('$');
        i += 1;
    }
    out
}

/// 从单个文件加载（ref `:105-154`）：读/解析失败 ⇒ warning + 不加载；名字 = 文件名去 `.md`。
#[must_use]
pub fn load_template_from_file(file_path: &Path) -> (Option<PromptTemplate>, Vec<Diagnostic>) {
    let path_string = file_path.to_string_lossy().to_string();
    let Ok(bytes) = std::fs::read(file_path) else {
        return (
            None,
            vec![Diagnostic {
                message: "failed to read prompt template file".to_string(),
                path: path_string,
            }],
        );
    };
    let content = String::from_utf8_lossy(&bytes).to_string();
    // ref `:119-140`：frontmatter 解析 + 描述推导（正文首个非空行、60 字符截断 + `...`）。
    let parsed = crate::skills::parse_skill_frontmatter(&content).ok();
    let body = crate::skills::strip_frontmatter(&content);
    let mut description = parsed
        .as_ref()
        .and_then(|frontmatter| frontmatter.description.clone())
        .filter(|value| !value.is_empty())
        .unwrap_or_default();
    if description.is_empty() {
        if let Some(first) = body.split('\n').find(|line| !line.trim().is_empty()) {
            let truncated: String = first.chars().take(60).collect();
            description = if first.chars().count() > 60 {
                format!("{truncated}...")
            } else {
                truncated
            };
        }
    }
    let name = file_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let name = name.strip_suffix(".md").unwrap_or(&name).to_string();
    (
        Some(PromptTemplate {
            name,
            description,
            content: body,
            file_path: path_string,
        }),
        Vec::new(),
    )
}

/// 扫一个目录（ref `:159-198`）：**只扫本层**、只收 `.md` 文件；readdir 出错 ⇒ 空。
#[must_use]
pub fn load_templates_from_dir(dir: &Path) -> TemplatesResult {
    let mut result = TemplatesResult::default();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return result;
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .collect();
    paths.sort();
    for path in paths {
        let is_md = path
            .file_name()
            .map(|name| name.to_string_lossy().ends_with(".md"))
            .unwrap_or(false);
        if !is_md || !path.is_file() {
            continue;
        }
        let (template, diagnostics) = load_template_from_file(&path);
        if let Some(template) = template {
            result.templates.push(template);
        }
        result.diagnostics.extend(diagnostics);
    }
    result
}

/// 汇总来源（ref `:222-298`）：`<agentDir>/prompts` → `<cwd>/.pi/prompts`（项目不受信任 ⇒ 跳过）→ 显式路径。
#[must_use]
pub fn load_prompt_templates(
    cwd: &Path,
    agent_dir: &Path,
    template_paths: &[String],
    include_defaults: bool,
) -> TemplatesResult {
    let mut result = TemplatesResult::default();
    if include_defaults {
        // 项目目录 `<cwd>/.pi/prompts` 需要项目受信任（ref `:1029/1043`）⇒ pi-rs 一律不加载。
        let global = load_templates_from_dir(&agent_dir.join("prompts"));
        result.templates.extend(global.templates);
        result.diagnostics.extend(global.diagnostics);
    }
    for raw in template_paths {
        let candidate = Path::new(raw);
        let resolved = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            cwd.join(candidate)
        };
        if !resolved.exists() {
            result.diagnostics.push(Diagnostic {
                message: "prompt template path does not exist".to_string(),
                path: resolved.to_string_lossy().to_string(),
            });
            continue;
        }
        if resolved.is_dir() {
            let loaded = load_templates_from_dir(&resolved);
            result.templates.extend(loaded.templates);
            result.diagnostics.extend(loaded.diagnostics);
        } else if resolved.is_file() {
            let (template, diagnostics) = load_template_from_file(&resolved);
            if let Some(template) = template {
                result.templates.push(template);
            }
            result.diagnostics.extend(diagnostics);
        } else {
            result.diagnostics.push(Diagnostic {
                message: "prompt template path is not a markdown file".to_string(),
                path: resolved.to_string_lossy().to_string(),
            });
        }
    }
    result
}

/// `/名字 参数` 展开（ref `:304-320`）：整串匹配；未命中 ⇒ 原样返回。
#[must_use]
pub fn expand_prompt_template(text: &str, templates: &[PromptTemplate]) -> String {
    if !text.starts_with('/') {
        return text.to_string();
    }
    let rest = &text[1..];
    let name_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    if name_end == 0 {
        return text.to_string();
    }
    let name = &rest[..name_end];
    let args_string = if name_end < rest.len() {
        &rest[name_end..]
    } else {
        ""
    };
    let Some(template) = templates.iter().find(|template| template.name == name) else {
        return text.to_string();
    };
    let args = parse_command_args(args_string);
    substitute_args(&template.content, &args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitution_matches_the_reference() {
        let args = vec!["甲".to_string(), "乙".to_string(), "丙".to_string()];
        assert_eq!(substitute_args("$1|$2|$9", &args), "甲|乙|");
        assert_eq!(substitute_args("$ARGUMENTS|$@", &args), "甲 乙 丙|甲 乙 丙");
        assert_eq!(substitute_args("${1:-默认}", &args), "甲");
        assert_eq!(substitute_args("${9:-默认}", &args), "默认");
        assert_eq!(substitute_args("${@:-没有}", &args), "甲 乙 丙");
        assert_eq!(substitute_args("${@:2}", &args), "乙 丙");
        assert_eq!(substitute_args("${@:2:1}", &args), "乙");
        // ref 的分支二只认 ； 不是该形态 ⇒ 原样保留
        assert_eq!(substitute_args("${0:1}", &args), "${0:1}");
        assert_eq!(substitute_args("${@:0:1}", &args), "甲"); // start<=0 当 1
    }

    #[test]
    fn tokenization_and_expansion_match_the_reference() {
        assert_eq!(
            parse_command_args("a \"b c\" 'd e' f"),
            vec!["a", "b c", "d e", "f"]
        );
        assert_eq!(parse_command_args("\"\""), Vec::<String>::new());
        let templates = vec![PromptTemplate {
            name: "my-tpl".to_string(),
            description: "描述".to_string(),
            content: "参数一是 $1；全部是 $ARGUMENTS；\n".to_string(),
            file_path: "<X>".to_string(),
        }];
        assert_eq!(
            expand_prompt_template("/my-tpl 甲 乙 丙", &templates),
            "参数一是 甲；全部是 甲 乙 丙；\n"
        );
        assert_eq!(expand_prompt_template("/nope x", &templates), "/nope x");
        assert_eq!(expand_prompt_template("hello", &templates), "hello");
        assert_eq!(expand_prompt_template("/ a", &templates), "/ a");
    }
}
