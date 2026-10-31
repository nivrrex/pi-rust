//! skills 的发现、解析、注入与诊断。
//!
//! 依据 `ref/pi-0.87.1/packages/coding-agent/src/core/skills.ts`：
//! - 候选文件名只有 `SKILL.md`（`:72`）；目录里命中即加载并**立即返回**（`:195-221`）；
//! - 否则遍历：跳过 `.` 开头（`:224-226`）与 `node_modules`（`:229-231`），目录递归且
//!   `includeRootFiles=false`（`:256`），**根层** `.md` 仅在 `includeRootFiles` 为真时收（`:262-264`）；
//! - 读失败 ⇒ warning 诊断且不加载（`:287-291`）；解析失败仅声明式记 warning（`:296-302`）；
//!   非声明式且无 description ⇒ 静默丢弃（`:306-308`）；**缺 description ⇒ 不加载**（`:330-332`）；
//! - name = frontmatter.name 缺省用父目录名（`:320-321`）；`:341` `disable-model-invocation === true`；
//! - 发现顺序 `<agentDir>/skills` → `<cwd>/.pi/skills` → 显式路径（`:452-455`、`:477-503`）；
//!   realpath 去重（`:425-430`）；同名**保留先者**并记 collision 诊断（`:433-448`）；
//! - 注入：`formatSkillsForPrompt`（`:355-383`）+ `escapeXml`（`:385-392`），段位置见
//!   `system-prompt.ts:165-169`（需 `read` 或 `bash` 在选中工具内）。
//!
//! **已知限制（见 notes/decisions.md）**：① 本模块自带**最小 YAML 子集**解析（`key: value`、
//! 布尔、去引号；多行/嵌套/列表等复杂 YAML 不支持），因为不允许新增依赖，而 ref 用 `yaml.parse`；
//! ② 不含 `.gitignore`/`.ignore`/`.fdignore` 规则（ref 用 `ignore` 包）；③ 项目目录
//! **一律视为未受信任**（ref `resource-loader.ts:1029/1043` 需 `isProjectTrusted()`，pi-rs 无该机制）。
//!
//! 本模块只做 I/O 边界与纯转换，不写 stderr；诊断随结果返回，由调用方输出。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// name 上限（ref `skills.ts:11`）。
pub const MAX_NAME_LENGTH: usize = 64;
/// description 上限（ref `skills.ts:14`）。
pub const MAX_DESCRIPTION_LENGTH: usize = 1024;
/// 声明式技能文件名（ref `skills.ts:72`）。
pub const SKILL_FILE_NAME: &str = "SKILL.md";

/// 解析出的 frontmatter（仍需按需取值，故保留原始键值）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
    pub disable_model_invocation: bool,
}

/// 一个技能（ref `skills.ts:74-81`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub file_path: String,
    pub base_dir: String,
    pub disable_model_invocation: bool,
}

/// 诊断种类（ref `ResourceDiagnostic.type`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticKind {
    Warning,
    Collision,
}

/// 一条诊断（`collision` 只在 `DiagnosticKind::Collision` 时有值，形如 `(winner, loser)`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    pub message: String,
    pub path: String,
    pub collision: Option<(String, String)>,
}

/// 加载结果（ref `skills.ts:83-86`）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillsResult {
    pub skills: Vec<Skill>,
    pub diagnostics: Vec<Diagnostic>,
}

/// ref `frontmatter.ts:9-27` 的换行归一 + BOM 去除 + 围栏定位。
///
/// 返回 `(frontmatter 文本, body)`；无围栏时前者为 `None`。
#[must_use]
pub fn extract_frontmatter(content: &str) -> (Option<String>, String) {
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    let normalized = normalized
        .strip_prefix('\u{feff}')
        .unwrap_or(&normalized)
        .to_string();
    if !normalized.starts_with("---") {
        return (None, normalized);
    }
    match normalized[3..].find("\n---") {
        Some(offset) => {
            let end_index = 3 + offset;
            let yaml = normalized[4..end_index].to_string();
            let body = normalized[end_index + 4..].trim().to_string();
            (Some(yaml), body)
        }
        None => (None, normalized),
    }
}

/// 去掉 frontmatter 后的正文（ref `frontmatter.ts:40`）。
#[must_use]
pub fn strip_frontmatter(content: &str) -> String {
    extract_frontmatter(content).1
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// 双引号标量的转义（YAML 子集：`\\ \" \n \t \r \0 \/ \uXXXX`）。
fn parse_double_quoted(raw: &str) -> Result<String, String> {
    // 跳过开引号本身（此前把开引号当闭引号 ⇒ 值恒为空）。
    let body = raw.strip_prefix('"').unwrap_or(raw);
    let mut chars = body.char_indices();
    let mut out = String::new();
    let mut closed = false;
    while let Some((_, ch)) = chars.next() {
        match ch {
            '"' => {
                closed = true;
                break;
            }
            '\\' => {
                let Some((_, esc)) = chars.next() else {
                    return Err(
                        "unsupported frontmatter syntax: dangling escape in double-quoted scalar"
                            .to_string(),
                    );
                };
                match esc {
                    '\\' => out.push('\\'),
                    '"' => out.push('"'),
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    '0' => out.push('\0'),
                    '/' => out.push('/'),
                    'u' => {
                        let mut hex = String::new();
                        for _ in 0..4 {
                            let Some((_, h)) = chars.next() else {
                                return Err("unsupported frontmatter syntax: short \\uXXXX escape"
                                    .to_string());
                            };
                            hex.push(h);
                        }
                        let code = u32::from_str_radix(&hex, 16).map_err(|_| {
                            "unsupported frontmatter syntax: bad \\uXXXX escape".to_string()
                        })?;
                        let Some(ch) = char::from_u32(code) else {
                            return Err(
                                "unsupported frontmatter syntax: invalid \\uXXXX code point"
                                    .to_string(),
                            );
                        };
                        out.push(ch);
                    }
                    other => {
                        return Err(format!(
                            "unsupported frontmatter syntax: unknown escape \\{other} in double-quoted scalar"
                        ));
                    }
                }
            }
            other => out.push(other),
        }
    }
    if !closed {
        return Err(
            "unsupported frontmatter syntax: unterminated double-quoted scalar".to_string(),
        );
    }
    Ok(out)
}

/// 单引号标量（YAML 子集：`''` 表示一个 `'`）。
fn parse_single_quoted(raw: &str) -> Result<String, String> {
    let body = &raw[1..];
    let mut out = String::new();
    let mut chars = body.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\'' {
            if chars.peek() == Some(&'\'') {
                chars.next();
                out.push('\'');
            } else {
                return Ok(out);
            }
        } else {
            out.push(ch);
        }
    }
    Err("unsupported frontmatter syntax: unterminated single-quoted scalar".to_string())
}

/// 块标量（`>`、`>-`、`>+`、`|`、`|-`、`|+`）。
fn parse_block_scalar(header: &str, content: &[String]) -> Result<String, String> {
    let folded = header.starts_with('>');
    let chomp = header.chars().nth(1);
    if let Some(c) = chomp {
        if c != '-' && c != '+' {
            return Err(format!(
                "unsupported frontmatter syntax: block scalar indicator `{header}`"
            ));
        }
    }
    let mut body = if folded {
        let mut out = String::new();
        for (i, line) in content.iter().enumerate() {
            if i == 0 {
                out.push_str(line);
                continue;
            }
            if line.is_empty() || content[i - 1].is_empty() {
                out.push('\n');
            } else {
                out.push(' ');
            }
            out.push_str(line);
        }
        out
    } else {
        content.join("\n")
    };
    match chomp {
        // `-`：去掉全部尾部换行。
        Some('-') => {
            while body.ends_with('\n') {
                body.pop();
            }
        }
        // 默认（clip）：恰好一个尾部换行（内容非空时）。
        None => {
            while body.ends_with('\n') {
                body.pop();
            }
            if !body.is_empty() || !content.is_empty() {
                body.push('\n');
            }
        }
        // `+`：保留原样（尾部空行已在 body 里）。
        Some(_) => {}
    }
    Ok(body)
}

/// 取一个标量值；块标量会吞掉后续行并把 `idx` 推到块尾。
fn parse_scalar_value(
    raw_value: &str,
    lines: &[String],
    idx: &mut usize,
    key_indent: usize,
    key: &str,
) -> Result<String, String> {
    let trimmed = raw_value.trim();
    let is_block_header = matches!(trimmed, ">" | ">-" | ">+" | "|" | "|-" | "|+");
    if is_block_header {
        let mut content: Vec<String> = Vec::new();
        let mut block_indent: Option<usize> = None;
        let mut cursor = *idx + 1;
        while cursor < lines.len() {
            let line = &lines[cursor];
            if line.trim().is_empty() {
                content.push(String::new());
                cursor += 1;
                continue;
            }
            let ind = indent_of(line);
            if ind <= key_indent {
                break;
            }
            let base = *block_indent.get_or_insert(ind);
            content.push(line[base.min(line.len())..].to_string());
            cursor += 1;
        }
        while content.last().map(|l| l.is_empty()).unwrap_or(false) {
            content.pop();
        }
        *idx = cursor.saturating_sub(1);
        return parse_block_scalar(trimmed, &content);
    }
    if trimmed.starts_with('"') {
        return parse_double_quoted(trimmed);
    }
    if trimmed.starts_with('\'') {
        return parse_single_quoted(trimmed);
    }
    // 其余形态：本实现只保证简单标量。遇到可能的复杂 YAML 一律拒绝，
    // 绝不静默给出可能错误的值（见 notes/decisions.md 的已知限制）。
    if trimmed.starts_with('&')
        || trimmed.starts_with('*')
        || trimmed.starts_with('!')
        || trimmed.starts_with('[')
        || trimmed.starts_with('{')
        || trimmed.contains(" #")
        || trimmed.contains(": ")
    {
        return Err(format!(
            "unsupported frontmatter syntax for `{key}`: {trimmed}"
        ));
    }
    // 简单标量不得跨行（下一行更缩进 ⇒ 多行 plain 标量 ⇒ 拒绝）。
    if let Some(next) = lines.get(*idx + 1) {
        if !next.trim().is_empty() && indent_of(next) > key_indent {
            return Err(format!(
                "unsupported frontmatter syntax for `{key}`: multi-line plain scalar"
            ));
        }
    }
    Ok(trimmed.to_string())
}

/// **受限 YAML 子集**解析（见模块文档的已知限制①）。
///
/// 支持：顶层 `key: value` 简单标量、`"…"`（含 `\\ \" \n \t \r \0 \/ \uXXXX`）、
/// `'…'`（`''` 表示一个单引号）、块标量 `>`/`>-`/`>+`/`|`/`|-`/`|+`（含缩进续行）。
/// 其余形态（锚点/别名/标签/flow 集合/注释/多行 plain 标量）返回 `Err`，
/// 由调用方转成 warning 且**不加载**该技能——不静默给出可能错误的值。
pub fn parse_frontmatter_minimal(yaml: &str) -> Result<Frontmatter, String> {
    let mut out = Frontmatter::default();
    let lines: Vec<String> = yaml.lines().map(|line| line.to_string()).collect();
    let mut i = 0usize;
    while i < lines.len() {
        let line = lines[i].clone();
        let trimmed = line.trim();
        if trimmed.is_empty() {
            i += 1;
            continue;
        }
        if trimmed.starts_with('#') {
            i += 1;
            continue;
        }
        let Some(colon) = line.find(':') else {
            i += 1;
            continue;
        };
        let key = line[..colon]
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        let raw_value = &line[colon + 1..];
        let key_indent = indent_of(&line);
        let value = parse_scalar_value(raw_value, &lines, &mut i, key_indent, &key)?;
        if key == "name" {
            out.name = Some(value);
        } else if key == "description" {
            out.description = Some(value);
        } else if key == "disable-model-invocation" {
            let lowered = value.trim().to_ascii_lowercase();
            if lowered != "true" && lowered != "false" {
                return Err(format!(
                    "unsupported frontmatter syntax for `disable-model-invocation`: {value}"
                ));
            }
            out.disable_model_invocation = lowered == "true";
        }
        i += 1;
    }
    Ok(out)
}

/// 解析 SKILL.md 的 frontmatter（ref `skills.ts:293-302`）。
pub fn parse_skill_frontmatter(content: &str) -> Result<Frontmatter, String> {
    let (yaml, _) = extract_frontmatter(content);
    match yaml {
        Some(yaml) => parse_frontmatter_minimal(&yaml),
        None => Ok(Frontmatter::default()),
    }
}

/// name 校验（ref `skills.ts:92-112`，逐条文案一致）。
#[must_use]
pub fn validate_name(name: &str) -> Vec<String> {
    let mut errors = Vec::new();
    if name.chars().count() > MAX_NAME_LENGTH {
        errors.push(format!(
            "name exceeds {MAX_NAME_LENGTH} characters ({})",
            name.chars().count()
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        errors.push(
            "name contains invalid characters (must be lowercase a-z, 0-9, hyphens only)"
                .to_string(),
        );
    }
    if name.starts_with('-') || name.ends_with('-') {
        errors.push("name must not start or end with a hyphen".to_string());
    }
    if name.contains("--") {
        errors.push("name must not contain consecutive hyphens".to_string());
    }
    errors
}

/// description 校验（ref `skills.ts:117-127`）。
#[must_use]
pub fn validate_description(description: Option<&str>) -> Vec<String> {
    let mut errors = Vec::new();
    let present = description
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if !present {
        errors.push("description is required".to_string());
    } else if let Some(value) = description {
        if value.chars().count() > MAX_DESCRIPTION_LENGTH {
            errors.push(format!(
                "description exceeds {MAX_DESCRIPTION_LENGTH} characters ({})",
                value.chars().count()
            ));
        }
    }
    errors
}

/// 从单个文件加载技能（ref `skills.ts:277-345`）。
#[must_use]
pub fn load_skill_from_file(file_path: &Path) -> (Option<Skill>, Vec<Diagnostic>) {
    let mut diagnostics = Vec::new();
    let path_string = path_to_string(file_path);
    let is_declared = file_path
        .file_name()
        .map(|name| name == SKILL_FILE_NAME)
        .unwrap_or(false);
    let Ok(content) = std::fs::read(file_path) else {
        diagnostics.push(Diagnostic {
            kind: DiagnosticKind::Warning,
            message: "failed to read skill file".to_string(),
            path: path_string,
            collision: None,
        });
        return (None, diagnostics);
    };
    let content = String::from_utf8_lossy(&content).to_string();
    let frontmatter = match parse_skill_frontmatter(&content) {
        Ok(parsed) => parsed,
        Err(error) => {
            if is_declared {
                diagnostics.push(Diagnostic {
                    kind: DiagnosticKind::Warning,
                    message: error,
                    path: path_string,
                    collision: None,
                });
            }
            return (None, diagnostics);
        }
    };
    let description = frontmatter.description.clone();
    let has_description = description
        .as_deref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if !is_declared && !has_description {
        return (None, diagnostics);
    }
    for error in validate_description(description.as_deref()) {
        diagnostics.push(Diagnostic {
            kind: DiagnosticKind::Warning,
            message: error,
            path: path_string.clone(),
            collision: None,
        });
    }
    let parent_dir_name = file_path
        .parent()
        .and_then(|dir| dir.file_name())
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let name = frontmatter
        .name
        .clone()
        .filter(|value| !value.is_empty())
        .unwrap_or(parent_dir_name);
    for error in validate_name(&name) {
        diagnostics.push(Diagnostic {
            kind: DiagnosticKind::Warning,
            message: error,
            path: path_string.clone(),
            collision: None,
        });
    }
    if !has_description {
        return (None, diagnostics);
    }
    let base_dir = file_path.parent().map(path_to_string).unwrap_or_default();
    (
        Some(Skill {
            name,
            description: description.unwrap_or_default(),
            file_path: path_string,
            base_dir,
            disable_model_invocation: frontmatter.disable_model_invocation,
        }),
        diagnostics,
    )
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn is_skipped_entry(name: &str) -> bool {
    name.starts_with('.') || name == "node_modules"
}

/// 扫描一个技能目录（ref `skills.ts:173-275`）。
#[must_use]
pub fn load_skills_from_dir(dir: &Path, include_root_files: bool) -> SkillsResult {
    let mut result = SkillsResult::default();
    if !dir.is_dir() {
        return result;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return result;
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .collect();
    paths.sort();
    // 第一遍：目录里存在 SKILL.md ⇒ 只加载它并立即返回（ref `:195-221`）。
    // 注意：这一遍**不受** `include_root_files` 约束（ref 的守卫只加在第二遍的根层 `.md` 上）。
    let declared = dir.join(SKILL_FILE_NAME);
    if declared.is_file() {
        let (skill, diagnostics) = load_skill_from_file(&declared);
        if let Some(skill) = skill {
            result.skills.push(skill);
        }
        result.diagnostics.extend(diagnostics);
        return result;
    }
    // 第二遍：递归子目录；根层 `.md` 仅在 include_root_files 时收（ref `:223-271`）。
    for path in paths {
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_string()) else {
            continue;
        };
        if is_skipped_entry(&name) {
            continue;
        }
        if path.is_dir() {
            let sub = load_skills_from_dir(&path, false);
            result.skills.extend(sub.skills);
            result.diagnostics.extend(sub.diagnostics);
            continue;
        }
        if !include_root_files || !name.ends_with(".md") {
            continue;
        }
        let (skill, diagnostics) = load_skill_from_file(&path);
        if let Some(skill) = skill {
            result.skills.push(skill);
        }
        result.diagnostics.extend(diagnostics);
    }
    result
}

/// 汇总多个来源（ref `skills.ts:409-509`）。
///
/// `agent_dir/skills`（user）→ `cwd/.pi/skills`（project，见模块文档限制③）→ 显式路径。
#[must_use]
pub fn load_skills(cwd: &Path, agent_dir: &Path, skill_paths: &[String]) -> SkillsResult {
    let mut result = SkillsResult::default();
    let mut real_paths: HashSet<String> = HashSet::new();
    let mut names: Vec<String> = Vec::new();
    let mut collisions: Vec<Diagnostic> = Vec::new();

    let add = |incoming: SkillsResult,
               result: &mut SkillsResult,
               real_paths: &mut HashSet<String>,
               names: &mut Vec<String>,
               collisions: &mut Vec<Diagnostic>| {
        result.diagnostics.extend(incoming.diagnostics);
        for skill in incoming.skills {
            let real = std::fs::canonicalize(&skill.file_path)
                .map(|path| path_to_string(&path))
                .unwrap_or_else(|_| skill.file_path.clone());
            if !real_paths.insert(real) {
                continue; // 同一文件（含符号链接）静默跳过（ref `:425-430`）
            }
            if names.iter().any(|name| name == &skill.name) {
                let winner = result
                    .skills
                    .iter()
                    .find(|existing| existing.name == skill.name)
                    .map(|existing| existing.file_path.clone())
                    .unwrap_or_default();
                collisions.push(Diagnostic {
                    kind: DiagnosticKind::Collision,
                    message: format!("name \"{}\" collision", skill.name),
                    path: skill.file_path.clone(),
                    collision: Some((winner, skill.file_path.clone())),
                });
            } else {
                names.push(skill.name.clone());
                result.skills.push(skill);
            }
        }
    };

    add(
        load_skills_from_dir(&agent_dir.join("skills"), true),
        &mut result,
        &mut real_paths,
        &mut names,
        &mut collisions,
    );
    // 项目目录：pi-rs 无信任机制 ⇒ 一律视为未受信任，不加载（模块文档限制③）。
    for raw in skill_paths {
        let candidate = Path::new(raw);
        let resolved = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            cwd.join(candidate)
        };
        if !resolved.exists() {
            result.diagnostics.push(Diagnostic {
                kind: DiagnosticKind::Warning,
                message: "skill path does not exist".to_string(),
                path: path_to_string(&resolved),
                collision: None,
            });
            continue;
        }
        if resolved.is_dir() {
            add(
                load_skills_from_dir(&resolved, true),
                &mut result,
                &mut real_paths,
                &mut names,
                &mut collisions,
            );
        } else if resolved.is_file() && path_to_string(&resolved).ends_with(".md") {
            let (skill, diagnostics) = load_skill_from_file(&resolved);
            add(
                SkillsResult {
                    skills: skill.into_iter().collect(),
                    diagnostics,
                },
                &mut result,
                &mut real_paths,
                &mut names,
                &mut collisions,
            );
        } else {
            result.diagnostics.push(Diagnostic {
                kind: DiagnosticKind::Warning,
                message: "skill path is not a markdown file".to_string(),
                path: path_to_string(&resolved),
                collision: None,
            });
        }
    }
    result.diagnostics.extend(collisions);
    result
}

/// `escapeXml`（ref `skills.ts:385-392`，替换顺序一致）。
#[must_use]
pub fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// `formatSkillsForPrompt`（ref `skills.ts:355-383`，逐行照抄）。
#[must_use]
pub fn format_skills_for_prompt(skills: &[Skill], file_read_tool: &str) -> String {
    let visible: Vec<&Skill> = skills
        .iter()
        .filter(|skill| !skill.disable_model_invocation)
        .collect();
    if visible.is_empty() {
        return String::new();
    }
    let mut lines = vec![
        String::new(),
        String::new(),
        "The following skills provide specialized instructions for specific tasks.".to_string(),
        if file_read_tool == "read" {
            "Use the read tool to load a skill's file when the task matches its description."
                .to_string()
        } else {
            "Use bash to load a skill's file when the task matches its description.".to_string()
        },
        "When a skill file references a relative path, resolve it against the skill directory (parent of SKILL.md / dirname of the path) and use that absolute path in tool commands.".to_string(),
        String::new(),
        "<available_skills>".to_string(),
    ];
    for skill in &visible {
        lines.push("  <skill>".to_string());
        lines.push(format!("    <name>{}</name>", escape_xml(&skill.name)));
        lines.push(format!(
            "    <description>{}</description>",
            escape_xml(&skill.description)
        ));
        lines.push(format!(
            "    <location>{}</location>",
            escape_xml(&skill.file_path)
        ));
        lines.push("  </skill>".to_string());
    }
    lines.push("</available_skills>".to_string());
    lines.join("\n")
}

/// `/skill:name args` 展开（ref `agent-session.ts:1797-1821`）。
///
/// 未命中或读失败 ⇒ 原样返回（`Err` 表示需要由调用方 `emitError` 的诊断）。
pub fn expand_skill_command(
    text: &str,
    skills: &[Skill],
) -> Result<Option<String>, (String, String)> {
    if !text.starts_with("/skill:") {
        return Ok(None);
    }
    let (name, args) = match text.find(' ') {
        Some(space) => (&text[7..space], text[space + 1..].trim()),
        None => (&text[7..], ""),
    };
    let Some(skill) = skills.iter().find(|skill| skill.name == name) else {
        return Ok(None); // 未知技能：原样放行（ref `:1805`）
    };
    let Ok(content) = std::fs::read_to_string(&skill.file_path) else {
        return Err((
            skill.file_path.clone(),
            "failed to read skill file".to_string(),
        ));
    };
    let body = strip_frontmatter(&content);
    let block = format!(
        "<skill name=\"{}\" location=\"{}\">\nReferences are relative to {}.\n\n{}\n</skill>",
        skill.name, skill.file_path, skill.base_dir, body
    );
    Ok(Some(if args.is_empty() {
        block
    } else {
        format!("{block}\n\n{args}")
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().expect("父目录")).expect("建目录");
        std::fs::write(path, content).expect("写文件");
    }

    fn skill_md(name: &str, description: &str) -> String {
        format!("---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n")
    }

    #[test]
    fn discovery_and_validation_follow_the_ref() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let agent = tmp.path().join("agent");
        let cwd = tmp.path().join("work");
        write(
            &agent,
            "skills/good-helper/SKILL.md",
            &skill_md("good-helper", "好的技能。"),
        );
        // 缺 description ⇒ 不加载（ref `:330-332`）
        write(
            &agent,
            "skills/broken/SKILL.md",
            "---\nname: broken\n---\n\n# broken\n",
        );
        // name 违规：仍加载但产生 warning（ref `:324-327`）
        write(
            &agent,
            "skills/Bad_Name/SKILL.md",
            &skill_md("Bad_Name", "名字违规。"),
        );
        let result = load_skills(&cwd, &agent, &[]);
        let names: Vec<&str> = result.skills.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"good-helper"),
            "应产出 good-helper：{names:?}"
        );
        assert!(
            !names.contains(&"broken"),
            "缺 description 不得加载：{names:?}"
        );
        assert!(names.contains(&"Bad_Name"), "name 违规仍加载（仅告警）");
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.message.contains("invalid characters")),
            "应有 name 违规告警：{:?}",
            result.diagnostics
        );

        // 同名冲突：user 目录先加载 ⇒ 胜出；显式路径的同名者被记为 collision
        write(
            &agent,
            "skills/dup/SKILL.md",
            &skill_md("dup", "来自 user 目录。"),
        );
        write(
            &cwd,
            ".pi/skills/dup/SKILL.md",
            &skill_md("dup", "来自显式路径。"),
        );
        let result = load_skills(
            &cwd,
            &agent,
            &[cwd.join(".pi/skills/dup").to_string_lossy().to_string()],
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.kind == DiagnosticKind::Collision && d.message.contains("collision")),
            "应有 collision 诊断：{:?}",
            result.diagnostics
        );
        // disable-model-invocation ⇒ 不进 prompt 文本
        write(&agent, "skills/hidden/SKILL.md", "---\nname: hidden\ndescription: 隐藏。\ndisable-model-invocation: true\n---\n\n# hidden\n");
        let result = load_skills(&cwd, &agent, &[]);
        let text = format_skills_for_prompt(&result.skills, "read");
        assert!(text.contains("good-helper"), "可见技能必须进文本");
        assert!(
            !text.contains("hidden"),
            "disable-model-invocation 必须排除"
        );
        assert_eq!(escape_xml("a&<>\"'b"), "a&amp;&lt;&gt;&quot;&apos;b");
    }

    #[test]
    fn skill_command_expansion_matches_the_ref() {
        let skills = vec![Skill {
            name: "helper".to_string(),
            description: "帮助".to_string(),
            file_path: "<X>".to_string(),
            base_dir: "<D>".to_string(),
            disable_model_invocation: false,
        }];
        // 未命中/非命令 ⇒ None（原样放行）
        assert_eq!(expand_skill_command("hello", &skills).expect("无错"), None);
        assert_eq!(
            expand_skill_command("/skill:does-not-exist x", &skills).expect("无错"),
            None
        );
        // 命中但文件读不到 ⇒ Err（调用方 emitError）
        assert!(expand_skill_command("/skill:helper", &skills).is_err());
    }

    /// 用仓库内 fixtures（13 个真实 skill 的 frontmatter 形态，已剔除 `~/.pi` 路径）
    /// 对照 node `yaml.parse` 的期望值：`description` 必须**逐字相同**。
    #[test]
    fn frontmatter_subset_matches_yaml_parse_on_real_shapes() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/skills-frontmatter");
        let expected_path = root.join("expected.json");
        let expected: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&expected_path).expect("读期望值"))
                .expect("期望值是 JSON");
        let mut checked = 0usize;
        for entry in std::fs::read_dir(&root).expect("列目录") {
            let path = entry.expect("目录项").path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let name = path
                .file_name()
                .expect("文件名")
                .to_string_lossy()
                .to_string();
            let content = std::fs::read_to_string(&path).expect("读 fixture");
            let parsed = parse_skill_frontmatter(&content)
                .unwrap_or_else(|e| panic!("{name} 应被支持，实际 Err({e})"));
            let want = expected
                .get(&name)
                .unwrap_or_else(|| panic!("{name} 缺期望值"));
            let want_name = want["name"].as_str();
            let want_desc = want["description"].as_str().expect("期望值含 description");
            assert_eq!(parsed.name.as_deref(), want_name, "{name}: name 不一致");
            assert_eq!(
                parsed.description.as_deref(),
                Some(want_desc),
                "{name}: description 必须与 yaml.parse 逐字相同"
            );
            assert_eq!(
                parsed.disable_model_invocation,
                want["disable"].as_bool().unwrap_or(false),
                "{name}: disable-model-invocation 不一致"
            );
            checked += 1;
        }
        assert_eq!(checked, 13, "fixtures 数应为 13，实际 {checked}");
    }

    /// 不支持的形态必须返回 `Err`（不静默给出可能错误的值）。
    #[test]
    fn unsupported_frontmatter_forms_are_rejected() {
        for (label, content) in [
            ("锚点", "---\nname: x\ndescription: &d 锚点描述\n---\n"),
            ("别名", "---\nname: x\ndescription: *d\n---\n"),
            ("flow 序列", "---\nname: x\ndescription: [a, b]\n---\n"),
            ("未知转义", "---\nname: x\ndescription: \"a\\qb\"\n---\n"),
            ("多行 plain", "---\nname: x\ndescription: 甲\n  乙\n---\n"),
            ("带注释", "---\nname: x\ndescription: 甲 # 注释\n---\n"),
        ] {
            assert!(
                parse_skill_frontmatter(content).is_err(),
                "{label} 必须被拒绝"
            );
        }
        // 支持形态的负向对照：同样内容去掉不支持的部分必须 Ok
        assert!(parse_skill_frontmatter("---\nname: x\ndescription: 锚点描述\n---\n").is_ok());
    }
}
