//! 系统提示的模板渲染。
//!
//! 段落的**唯一规格**是 TS 源码，逐段来源见 `spec/system-prompt-sections.md`
//! （原先的 `spec/system-prompt.template.txt` 是 golden 挖空快照，已删除）。
//!
//! **范围说明**：这里只做**替换**。这些值本身是怎么组装出来的（cwd 段、docs 段的
//! README 路径、工具 JSON、项目上下文文件）尚未实现——见
//! `notes/p2-reconcile.md` 的未完成项。因此本模块能保证"给定输入，渲染结果与 TS 一致"，
//! 不能保证"输入本身与 TS 一致"。

/// 模板里使用的占位符。
pub const PLACEHOLDERS: [&str; 4] = [
    "{{CWD}}",
    "{{PI_PACKAGE_DIR}}",
    "{{TOOLS_JSON}}",
    "{{PROJECT_CONTEXT}}",
];

/// 渲染所需的输入；缺省一律用空串（与 TS 侧"值为空就不填"的行为一致）。
#[derive(Debug, Clone, Default)]
pub struct SystemPromptInput {
    /// 工作目录段。
    pub cwd: String,
    /// pi 包目录。
    pub pi_package_dir: String,
    /// 工具声明（已经序列化成 JSON 文本）。
    pub tools_json: String,
    /// 项目上下文块（含 `<project_context>` 标签的整段）。
    pub project_context: String,
}

/// section 的拼接顺序。
///
/// **已验证**：`preamble`、`tools`、`rules`、`docs`、`cwd` 的相对顺序由 S01/S23/S24/S25-openai 的
/// `sections` 与请求体正文逐字节比对确认；`project_context` 与 `skills` 的先后由 **S23b**
/// （两者并存的场景）确认——**project_context 在前、skills 在后**。
///
/// 注：这条顺序此前是猜的（把 skills 排在 project_context 之前），S23b 录制出来后**推翻**了它。
pub const SECTION_ORDER: [&str; 7] = [
    "preamble",
    "tools",
    "rules",
    "docs",
    "project_context",
    "skills",
    "cwd",
];

/// 按固定顺序把各 section 用空行拼成系统提示正文。
///
/// 缺的 section 直接跳过（不留空行）——S01/S25-openai 没有 `project_context`，
/// 拼接结果仍与 golden 逐字节一致，说明缺席就是跳过。
pub fn assemble(sections: &std::collections::BTreeMap<String, String>) -> String {
    SECTION_ORDER
        .iter()
        .filter_map(|name| sections.get(*name))
        .filter(|value| !value.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// 渲染模板：逐个占位符做**全局**替换，末尾换行按 TS 的 `trimEnd()` 处理。
///
/// 注意 `{{PROJECT_CONTEXT}}` 在模板里带尾随空格，替换时要把它一并吃掉——
/// 否则字节对不上（TS 侧用的是同一份模板，也有这个问题，靠 `trimEnd` 兜住整行末尾）。
pub fn render(template: &str, input: &SystemPromptInput) -> String {
    let mut out = template.trim_end().to_string();
    for placeholder in PLACEHOLDERS {
        let value = match placeholder {
            "{{CWD}}" => &input.cwd,
            "{{PI_PACKAGE_DIR}}" => &input.pi_package_dir,
            "{{TOOLS_JSON}}" => &input.tools_json,
            "{{PROJECT_CONTEXT}}" => &input.project_context,
            other => unreachable!("未定义的占位符 {other}"),
        };
        out = out.replace(placeholder, value);
    }
    out
}
/// `preamble` section 的固定文本（与 golden 逐字节一致）。
pub const PREAMBLE: &str = "You are an expert coding assistant operating inside pi, a coding agent harness. You help users by reading files, executing commands, editing code, and writing new files.";

/// `tools` section 的结尾句（`<tools>` 段的固定收尾）。
pub const TOOLS_SECTION_TAIL: &str = "In addition to the tools above, you may have access to other custom tools depending on the project.";

/// `cwd` section 的正文格式：`<cwd>\n{cwd}\n</cwd>`。
pub fn cwd_section(cwd: &str) -> String {
    format!("<cwd>\n{cwd}\n</cwd>")
}

/// 由工具的 `(name, snippet)` 列表拼出 `tools` section。
///
/// 每行是 `- {name}: {snippet}`；这些 snippet 来自各工具的
/// `*ToolSystemPromptContribution.snippet`（`spec/e-tools.md` 第 1 节）。
pub fn tools_section<S: AsRef<str>>(tools: &[(S, S)]) -> String {
    let mut out = String::from("<tools>\n");
    for (name, snippet) in tools {
        out.push_str(&format!("- {}: {}\n", name.as_ref(), snippet.as_ref()));
    }
    out.push('\n');
    out.push_str(TOOLS_SECTION_TAIL);
    out.push_str("\n</tools>");
    out
}

/// 四个内置工具的 snippet，与 golden 的 `tools` 段逐字一致。
pub fn builtin_tool_snippets() -> Vec<(String, String)> {
    vec![
        ("read".to_string(), "Read file contents".to_string()),
        ("bash".to_string(), "Execute bash commands (ls, grep, find, etc.)".to_string()),
        ("edit".to_string(), "Make precise file edits with exact text replacement, including multiple disjoint edits in one call".to_string()),
        ("write".to_string(), "Create or overwrite files".to_string()),
    ]
}

/// `rules` section 的固定文本。
///
/// **已验证**：32 个 golden 场景的 `rules` 段指纹完全一致（`ea783a8f`），是常量。
pub const RULES: &str = "<rules>\n- Use bash for file operations like ls, rg, find\n- Use read to examine files instead of cat or sed.\n- You can inspect PI_* environment variables for current model and session details.\n- Use edit for precise changes (edits[].oldText must match exactly)\n- When changing multiple separate locations in one file, use one edit call with multiple entries in edits[] instead of multiple edit calls\n- Each edits[].oldText is matched against the original file, not after earlier edits are applied. Do not emit overlapping or nested edits. Merge nearby changes into one edit.\n- Keep edits[].oldText as small as possible while still being unique in the file. Do not pad with large unchanged regions.\n- Use write only for new files or complete rewrites.\n- Be concise in your responses\n- Show file paths clearly when working with files\n</rules>";

/// `docs` section 的模板；`{{PI_PACKAGE_DIR}}` 由运行时包路径替换。
pub const DOCS_TEMPLATE: &str = "<docs>\nPi documentation (read only when the user asks about pi itself, its SDK, extensions, themes, skills, or TUI):\n- Main documentation: {{PI_PACKAGE_DIR}}/README.md\n- Additional docs: {{PI_PACKAGE_DIR}}/docs\n- Examples: {{PI_PACKAGE_DIR}}/examples (extensions, custom tools, SDK)\n- When reading pi docs or examples, resolve docs/... under Additional docs and examples/... under Examples, not the current working directory\n- When asked about: extensions (docs/extensions.md, examples/extensions/), themes (docs/themes.md), skills (docs/skills.md), prompt templates (docs/prompt-templates.md), TUI components (docs/tui.md), keybindings (docs/keybindings.md), SDK integrations (docs/sdk.md), custom providers (docs/custom-provider.md), adding models (docs/models.md), pi packages (docs/packages.md), environment variables (docs/environment-variables.md)\n- When working on pi topics, read the docs and examples, and follow .md cross-references before implementing\n- Always read pi .md files completely and follow links to related docs (e.g., tui.md for TUI API details)\n</docs>";

/// 生成 `docs` section。
pub fn docs_section(pi_package_dir: &str) -> String {
    DOCS_TEMPLATE.replace("{{PI_PACKAGE_DIR}}", pi_package_dir)
}

/// `project_context` section 的固定引导句。
pub const PROJECT_CONTEXT_HEADER: &str = "Project-specific instructions and guidelines:";

/// 一条项目指令文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectInstructionFile {
    /// 文件的绝对路径（写进 `path` 属性）。
    pub path: String,
    /// 文件内容原文（末尾换行会保留）。
    pub content: String,
}

/// 生成 `project_context` section。
///
/// 格式（对照 golden/S23 的段落逐字节）：
/// `<project_context>` + 引导句 + 每条文件一个
/// `<project_instructions path="…">\n{content}</project_instructions>` 块，
/// 块之间空行分隔，最后以 `</project_context>` 收尾。
///
/// **未验证**：文件的**发现顺序与搜索范围**（哪些文件名、从 cwd 往上找几层）。
/// 这里按调用方给的顺序输出，顺序由调用方负责。
pub fn project_context_section(files: &[ProjectInstructionFile]) -> String {
    if files.is_empty() {
        return String::new();
    }
    let mut out = String::from("<project_context>\n");
    out.push_str(PROJECT_CONTEXT_HEADER);
    for file in files {
        out.push_str("\n\n<project_instructions path=\"");
        out.push_str(&file.path);
        out.push_str("\">\n");
        out.push_str(&file.content);
        out.push_str("</project_instructions>");
    }
    out.push_str("\n</project_context>");
    out
}

/// `skills` section 的固定引导文本（`<available_skills>` 之前的部分）。
///
/// **已验证**：S24 的段落里包路径未出现，前缀是常量；`<location>` 里的路径是运行时的
/// skill 文件位置。
pub const SKILLS_HEADER: &str = "<skills>\nThe following skills provide specialized instructions for specific tasks.\nUse the read tool to load a skill's file when the task matches its description.\nWhen a skill file references a relative path, resolve it against the skill directory (parent of SKILL.md / dirname of the path) and use that absolute path in tool commands.\n\n";

/// 一个可用 skill。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableSkill {
    /// skill 名。
    pub name: String,
    /// 描述（取自 SKILL.md 的 frontmatter）。
    pub description: String,
    /// `SKILL.md` 的绝对路径。
    pub location: String,
}

/// 生成 `skills` section；没有 skill 时返回空串（整段跳过）。
///
/// 格式对照 golden/S24：固定引导文本 + `<available_skills>` +
/// 每个 skill 一个 `<skill>` 块（两空格缩进）+ `</available_skills></skills>`。
pub fn skills_section(skills: &[AvailableSkill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let converted: Vec<crate::skills::Skill> = skills
        .iter()
        .map(|skill| crate::skills::Skill {
            name: skill.name.clone(),
            description: skill.description.clone(),
            file_path: skill.location.clone(),
            base_dir: String::new(),
            disable_model_invocation: false,
        })
        .collect();
    let inner = crate::skills::format_skills_for_prompt(&converted, "read");
    if inner.trim().is_empty() {
        return String::new();
    }
    format!("<skills>\n{}\n</skills>", inner.trim())
}

/// 工具在系统提示里的声明：`systemPromptContribution` 的 `snippet` 与 `guidelines`。
///
/// **值全部取自 ref 的源码**（不是 golden），出处逐条标注：
/// - `read`：`core/tools/read.ts:21-22`
/// - `bash`：`core/tools/bash.ts:44-45`
/// - `edit`：`core/tools/edit.ts:44,46-49`
/// - `write`：`core/tools/write.ts:17-18`
///
/// 顺序即 `selectedTools` 的默认顺序（`system-prompt.ts:48` 的
/// `["read", "bash", "edit", "write"]`），**不排序**。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPrompt {
    pub name: String,
    pub snippet: String,
    pub guidelines: Vec<String>,
}

/// 四个内置工具的提示声明（顺序固定）。
pub fn builtin_tool_prompts() -> Vec<ToolPrompt> {
    vec![
        ToolPrompt {
            name: "read".to_string(),
            snippet: "Read file contents".to_string(),
            guidelines: vec!["Use read to examine files instead of cat or sed.".to_string()],
        },
        ToolPrompt {
            name: "bash".to_string(),
            snippet: "Execute bash commands (ls, grep, find, etc.)".to_string(),
            guidelines: vec![
                "You can inspect PI_* environment variables for current model and session details."
                    .to_string(),
            ],
        },
        ToolPrompt {
            name: "edit".to_string(),
            snippet: "Make precise file edits with exact text replacement, including multiple disjoint edits in one call".to_string(),
            guidelines: vec![
                "Use edit for precise changes (edits[].oldText must match exactly)".to_string(),
                "When changing multiple separate locations in one file, use one edit call with multiple entries in edits[] instead of multiple edit calls".to_string(),
                "Each edits[].oldText is matched against the original file, not after earlier edits are applied. Do not emit overlapping or nested edits. Merge nearby changes into one edit.".to_string(),
                "Keep edits[].oldText as small as possible while still being unique in the file. Do not pad with large unchanged regions.".to_string(),
            ],
        },
        ToolPrompt {
            name: "write".to_string(),
            snippet: "Create or overwrite files".to_string(),
            guidelines: vec!["Use write only for new files or complete rewrites.".to_string()],
        },
    ]
}

/// `tools` 段末尾那句固定尾巴。出处：`system-prompt.ts:152`。
pub const TOOLS_TAIL: &str =
    "In addition to the tools above, you may have access to other custom tools depending on the project.";

/// `rules` 段末尾的两条固定规则。出处：`system-prompt.ts:115-116`。
pub const FIXED_RULES: [&str; 2] = [
    "Be concise in your responses",
    "Show file paths clearly when working with files",
];

/// 按 `system-prompt.ts:81-117` 的 `buildRules` 组装 `rules` 段。
///
/// 规则顺序（**逐条照抄**）：
/// 1. **条件性首条**（:101-109）：选中了 `bash` 或 `powershell`，且 `grep`/`find`/`ls` **都没选**时，
///    加一条"用 shell 做文件操作"的规则（三个分支见下）；
/// 2. 按 `selectedTools` 顺序取各工具的 guidelines（:111-113）；
/// 3. `promptGuidelines`（:114）；
/// 4. 两条固定规则（:115-116）。
///
/// 全程**去重**且跳过空串（`addRule`，:88-93）——我第一版漏了这一步与那条条件首条，
/// 由与 golden 比对的 `rules_section_matches_golden_without_taking_golden_as_input` 抓出。
pub fn rules_section(
    selected_tools: &[String],
    prompts: &[ToolPrompt],
    prompt_guidelines: &[String],
) -> String {
    struct Rules {
        items: Vec<String>,
        seen: std::collections::BTreeSet<String>,
    }
    impl Rules {
        fn add(&mut self, rule: &str) {
            let normalized = rule.trim();
            if normalized.is_empty() || !self.seen.insert(normalized.to_string()) {
                return;
            }
            self.items.push(normalized.to_string());
        }
    }

    let has = |name: &str| selected_tools.iter().any(|tool| tool == name);
    let mut rules = Rules {
        items: Vec::new(),
        seen: std::collections::BTreeSet::new(),
    };

    if (has("bash") || has("powershell")) && !has("grep") && !has("find") && !has("ls") {
        if has("bash") && has("powershell") {
            rules.add("Use bash or PowerShell for file operations like listing, searching, and finding files");
        } else if has("powershell") {
            rules.add(
                "Use PowerShell for file operations like listing, searching, and finding files",
            );
        } else {
            rules.add("Use bash for file operations like ls, rg, find");
        }
    }

    for name in selected_tools {
        if let Some(tool) = prompts.iter().find(|tool| &tool.name == name) {
            for rule in &tool.guidelines {
                rules.add(rule);
            }
        }
    }
    for rule in prompt_guidelines {
        rules.add(rule);
    }
    for rule in FIXED_RULES {
        rules.add(rule);
    }
    rules
        .items
        .iter()
        .map(|rule| format!("- {rule}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 装配的输入变量。**每个字段的来源都要能说清**（见 `notes/decisions.md` 的来源表）：
/// 场景/runner 配置、ref 的工具定义、或环境——不得取自被验证的 golden。
#[derive(Debug, Clone, Default)]
pub struct SectionsInput {
    pub cwd: String,
    /// pi 包目录（docs 段的三个路径由它推导）。出处：`config.ts:389` 的 `getPackageDir()`。
    pub package_dir: String,
    /// 选中的工具，**顺序即请求体顺序**。默认 `["read","bash","edit","write"]`（`system-prompt.ts:48`）。
    pub selected_tools: Vec<String>,
    /// 工具的 snippet 与 guidelines（见 [`builtin_tool_prompts`]）。
    pub tool_prompts: Vec<ToolPrompt>,
    /// `promptGuidelines`（本项目暂无来源，留空）。
    pub prompt_guidelines: Vec<String>,
    /// `--append-system-prompt` 的累加结果（多段用 `\n\n` 连接，`agent-session.ts:1379`）。
    pub append_system_prompt: String,
    /// 项目指令文件（`path` 与 `content`）。出处：场景夹具里的 AGENTS.md 等。
    pub context_files: Vec<(String, String)>,
    /// 技能列表。出处：场景夹具。
    pub skills: Vec<AvailableSkill>,
    /// `--system-prompt`（替换 preamble）。
    pub custom_prompt: Option<String>,
}

/// 按 `system-prompt.ts:121-176` 的顺序产出 `(段名, 已包裹的段文本)`。
///
/// 顺序（**这是 P3.3 此前完全缺失的那一层**）：`preamble`（不加标签）→ `tools` → `rules`
/// → `docs` → `addendum`（有才加）→ `project_context`（有才加）→ `skills`（有且选中
/// `read`/`bash` 才加）→ `cwd` → 自定义段。除 `preamble` 外一律包裹成 `<名>\n内容\n</名>`。
pub fn build_sections(input: &SectionsInput) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let preamble = match &input.custom_prompt {
        Some(custom) => custom.clone(),
        None => PREAMBLE.to_string(),
    };
    out.push(("preamble".to_string(), preamble));

    // ref `system-prompt.ts:143-161`：`customPrompt` 存在时**只**用自定义 preamble，
    // `tools`/`rules`/`docs` 三段根本不生成；`addendum`/`project_context`/`skills`/`cwd`
    // 在 if/else **之外**（`:163-170`），仍然生成。
    if input.custom_prompt.is_none() {
        let mut visible: Vec<(String, String)> = Vec::new();
        for name in &input.selected_tools {
            if let Some(tool) = input.tool_prompts.iter().find(|tool| &tool.name == name) {
                if !tool.snippet.is_empty() {
                    visible.push((tool.name.clone(), tool.snippet.clone()));
                }
            }
        }
        let tools_body = if visible.is_empty() {
            "(none)".to_string()
        } else {
            visible
                .iter()
                .map(|(name, snippet)| format!("- {name}: {snippet}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        out.push(("tools".to_string(), format!("{tools_body}\n\n{TOOLS_TAIL}")));

        out.push((
            "rules".to_string(),
            rules_section(
                &input.selected_tools,
                &input.tool_prompts,
                &input.prompt_guidelines,
            ),
        ));
        out.push(("docs".to_string(), docs_body(&input.package_dir)));
    }
    if !input.append_system_prompt.is_empty() {
        out.push(("addendum".to_string(), input.append_system_prompt.clone()));
    }
    if !input.context_files.is_empty() {
        let rendered = input
            .context_files
            .iter()
            .map(|(path, content)| {
                format!(
                    "<project_instructions path=\"{path}\">\n{content}\n</project_instructions>"
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        out.push((
            "project_context".to_string(),
            format!("Project-specific instructions and guidelines:\n\n{rendered}"),
        ));
    }
    // `skills` 只在选中 `read` 或 `bash` 时才加（`system-prompt.ts:165-170`）。
    let skill_reader = ["read", "bash"]
        .into_iter()
        .find(|name| input.selected_tools.iter().any(|tool| tool == name));
    if let Some(reader) = skill_reader {
        if !input.skills.is_empty() {
            // ref `skills.ts:355-383`：措辞按 `fileReadTool` 切换；**外层 `<skills>` 标签由下面的
            // 通用包裹逻辑补**（golden 的 skills 段正是 `<skills>…</skills>`）。
            let converted: Vec<crate::skills::Skill> = input
                .skills
                .iter()
                .map(|skill| crate::skills::Skill {
                    name: skill.name.clone(),
                    description: skill.description.clone(),
                    file_path: skill.location.clone(),
                    base_dir: String::new(),
                    disable_model_invocation: false,
                })
                .collect();
            let text = crate::skills::format_skills_for_prompt(&converted, reader)
                .trim()
                .to_string();
            if !text.is_empty() {
                out.push(("skills".to_string(), text));
            }
        }
    }
    out.push(("cwd".to_string(), input.cwd.replace('\\', "/")));

    let mut wrapped: Vec<(String, String)> = Vec::new();
    for (name, content) in out {
        if name == "preamble" {
            wrapped.push((name, content));
        } else {
            wrapped.push((name.clone(), format!("<{name}>\n{content}\n</{name}>")));
        }
    }
    wrapped
}

/// `docs` 段的正文。**逐字取自** `ref/.../core/system-prompt.ts:153-160`，
/// 三个路径由 `config.ts:440-452` 的 `join(getPackageDir(), …)` 推导。
fn docs_body(package_dir: &str) -> String {
    let dir = package_dir.trim_end_matches('/');
    let template = r#"Pi documentation (read only when the user asks about pi itself, its SDK, extensions, themes, skills, or TUI):
- Main documentation: {dir}/README.md
- Additional docs: {dir}/docs
- Examples: {dir}/examples (extensions, custom tools, SDK)
- When reading pi docs or examples, resolve docs/... under Additional docs and examples/... under Examples, not the current working directory
- When asked about: extensions (docs/extensions.md, examples/extensions/), themes (docs/themes.md), skills (docs/skills.md), prompt templates (docs/prompt-templates.md), TUI components (docs/tui.md), keybindings (docs/keybindings.md), SDK integrations (docs/sdk.md), custom providers (docs/custom-provider.md), adding models (docs/models.md), pi packages (docs/packages.md), environment variables (docs/environment-variables.md)
- When working on pi topics, read the docs and examples, and follow .md cross-references before implementing
- Always read pi .md files completely and follow links to related docs (e.g., tui.md for TUI API details)"#;
    template.replace("{dir}", dir)
}

/// 把各段拼成请求体里的 system 正文。出处：`ai/src/utils/text.ts:15-21`。
///
/// 规则：`content` 在前（本实现里恒为空串，会被过滤掉），随后**按段顺序**取每段文本，
/// 跳过 `null`，**丢掉空串**，最后用 `"\n\n"` 连接。
pub fn render_system_text(sections: &[(String, String)]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (_, text) in sections {
        if !text.is_empty() {
            parts.push(text.clone());
        }
    }
    parts.join("\n\n")
}
