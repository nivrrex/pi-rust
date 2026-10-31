//! P3-14：系统提示模板的渲染校验。
//!
//! 复现 `scripts/verify-system-prompt.mjs` 对 golden/S23 的检查，但用 Rust 实现。
//!
//! **必须说清的循环性**：模板 `spec/system-prompt.template.txt` 本身是**从 golden 提取**的
//! JSON 序列化 system 消息，且 `{{PROJECT_CONTEXT}}` 的内容也是**从同一条 golden 里抠出来**
//! 再回填的。所以这个测试证明的是"模板 + 该段落 = golden"，**不证明**项目上下文与
//! cwd/docs/工具段是怎么组装出来的——那部分尚未实现。

use pi_core::system_prompt::{assemble, render, SystemPromptInput};

fn workspace() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate 在工作区根下")
        .to_path_buf()
}

/// 从请求体里取出"系统提示正文"——两种 provider 的形状不同：
/// - OpenAI 系：`messages[0]` 的 `content`，role 可能是 `system` 或 `developer`（compat 开关）；
/// - Anthropic 系：顶层 `system` 字段，是文本块数组。
fn request_system_text(scenario: &str) -> Option<String> {
    let path = workspace()
        .join("golden")
        .join(scenario)
        .join("requests.jsonl");
    // **跳过摘要请求**：S13d（提示词前压缩检查）的第一个请求就是摘要请求，
    // 它的 system 是 SUMMARIZATION_SYSTEM_PROMPT，不是场景的 system sections。
    // 两侧用同一条跳过规则，所以比较仍然有意义。
    let line = std::fs::read_to_string(&path)
        .ok()?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .find(|l| !l.contains("You are a context summarization assistant"))?
        .to_string();
    let entry: serde_json::Value = serde_json::from_str(&line).ok()?;
    let body: serde_json::Value = serde_json::from_str(entry["body"].as_str()?).ok()?;
    if let Some(blocks) = body.get("system").and_then(|s| s.as_array()) {
        let text: String = blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("");
        return Some(text);
    }
    let first = body.get("messages")?.as_array()?.first()?.clone();
    let role = first.get("role")?.as_str()?;
    if role != "system" && role != "developer" {
        return None;
    }
    Some(first.get("content")?.as_str()?.to_string())
}

fn golden_system_json(scenario: &str) -> String {
    let path = workspace()
        .join("golden")
        .join(scenario)
        .join("requests.jsonl");
    let line = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("读不到 {}: {e}", path.display()))
        .lines()
        .filter(|l| !l.trim().is_empty())
        // 与另一侧同规则：跳过摘要请求（S13d 的首个请求是摘要请求）。
        .find(|l| !l.contains("You are a context summarization assistant"))
        .expect("有非摘要请求记录")
        .to_string();
    let entry: serde_json::Value = serde_json::from_str(&line).expect("记录是 JSON");
    let body: serde_json::Value =
        serde_json::from_str(entry["body"].as_str().expect("body 是字符串"))
            .expect("请求体是 JSON");
    serde_json::to_string(&body["messages"][0]).expect("可序列化")
}

/// 取出 `content` 字段（系统提示正文）。
fn content_of(system_json: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(system_json).expect("是 JSON");
    value["content"]
        .as_str()
        .expect("content 是字符串")
        .to_string()
}

#[test]
fn missing_values_render_as_empty_strings() {
    // TS 侧"值为空就不填"——缺省一律空串。
    let rendered = render(
        "a{{CWD}}b{{PI_PACKAGE_DIR}}c{{TOOLS_JSON}}d{{PROJECT_CONTEXT}}e",
        &SystemPromptInput::default(),
    );
    assert_eq!(rendered, "abcde");
}

#[test]
fn values_are_substituted_globally() {
    let rendered = render(
        "{{CWD}}/{{CWD}}",
        &SystemPromptInput {
            cwd: "/work".to_string(),
            ..SystemPromptInput::default()
        },
    );
    assert_eq!(rendered, "/work//work");
}

/// **非循环验证**：会话 entry 的 `sections` 按固定顺序拼接，必须等于请求体里的 system 正文。
///
/// 与上面那条不同——这里的输入（sections）取自**会话文件**，输出比对的是**请求体**，
/// 不存在"从输出里抠出来回填"的循环。
#[test]
fn sections_assemble_into_the_request_system_content() {
    // 覆盖**所有**有 sections 的场景，而不是挑几个——非循环验证的价值就在于覆盖面。
    let golden = workspace().join("golden");
    let mut checked = 0;
    let mut scenarios: Vec<String> = std::fs::read_dir(&golden)
        .expect("有 golden 目录")
        .filter_map(|entry| {
            entry
                .ok()
                .map(|e| e.file_name().to_string_lossy().to_string())
        })
        .filter(|name| !name.starts_with('_'))
        .collect();
    scenarios.sort();
    for scenario in scenarios {
        let session_path = golden.join(&scenario).join("session.jsonl");
        let requests_path = golden.join(&scenario).join("requests.jsonl");
        // 有些场景（如纯 rpc/X 系列）没有请求记录，跳过后再计数。
        let has_requests = std::fs::read_to_string(&requests_path)
            .map(|text| !text.trim().is_empty())
            .unwrap_or(false);
        if !session_path.is_file() || !has_requests {
            continue;
        }
        let session = std::fs::read_to_string(
            workspace()
                .join("golden")
                .join(&scenario)
                .join("session.jsonl"),
        )
        .unwrap_or_else(|e| panic!("{scenario} 会话不可读：{e}"));
        let mut sections = std::collections::BTreeMap::new();
        for line in session.lines().filter(|l| !l.trim().is_empty()) {
            let entry: serde_json::Value = serde_json::from_str(line).expect("行是 JSON");
            let Some(message) = entry.get("message") else {
                continue;
            };
            if message.get("role").and_then(serde_json::Value::as_str) != Some("system") {
                continue;
            }
            let Some(map) = message
                .get("sections")
                .and_then(serde_json::Value::as_object)
            else {
                continue;
            };
            for (key, value) in map {
                if let Some(text) = value.as_str() {
                    sections.insert(key.clone(), text.to_string());
                }
            }
        }
        if sections.is_empty() {
            // 少数场景（如纯 X 系列）可能没有 sections，跳过并计数。
            continue;
        }

        let assembled = assemble(&sections);
        let Some(golden_content) = request_system_text(&scenario) else {
            continue;
        };
        assert_eq!(
            assembled, golden_content,
            "{scenario}: sections 拼接结果必须等于请求体的 system 正文"
        );
        checked += 1;
    }
    assert!(
        checked >= 30,
        "应当覆盖 30 个以上有 sections 的场景，实际 {checked}（S09 无请求记录，属预期排除）"
    );
}

fn golden_sections(scenario: &str) -> serde_json::Map<String, serde_json::Value> {
    let path = workspace()
        .join("golden")
        .join(scenario)
        .join("session.jsonl");
    for line in std::fs::read_to_string(&path)
        .expect("会话可读")
        .lines()
        .filter(|l| !l.trim().is_empty())
    {
        let entry: serde_json::Value = serde_json::from_str(line).expect("行是 JSON");
        let Some(message) = entry.get("message") else {
            continue;
        };
        if message.get("role").and_then(serde_json::Value::as_str) != Some("system") {
            continue;
        }
        if let Some(map) = message
            .get("sections")
            .and_then(serde_json::Value::as_object)
        {
            return map.clone();
        }
    }
    panic!("{scenario}: 没有 system 的 sections");
}

#[test]
fn preamble_matches_golden() {
    assert_eq!(
        pi_core::system_prompt::PREAMBLE,
        golden_sections("S23")["preamble"]
            .as_str()
            .expect("是字符串")
    );
}

#[test]
fn cwd_section_matches_golden() {
    let sections = golden_sections("S23");
    let golden = sections["cwd"].as_str().expect("是字符串").to_string();
    // golden 里的路径被归一化成了 `<TMP>/work`，取最后一段比对格式。
    let inner = golden
        .strip_prefix("<cwd>\n")
        .and_then(|rest| rest.strip_suffix("\n</cwd>"))
        .expect("格式为 <cwd>…</cwd>");
    assert_eq!(
        pi_core::system_prompt::cwd_section(inner),
        golden,
        "cwd 段落格式必须与 golden 逐字节一致"
    );
}

#[test]
fn tools_section_matches_golden() {
    let sections = golden_sections("S23");
    let golden = sections["tools"].as_str().expect("是字符串");
    let snippets = pi_core::system_prompt::builtin_tool_snippets();
    let names: Vec<String> = snippets.iter().map(|(name, _)| name.clone()).collect();
    assert_eq!(names, vec!["read", "bash", "edit", "write"], "顺序固定");
    assert_eq!(
        pi_core::system_prompt::tools_section(&snippets),
        golden,
        "tools 段落必须与 golden 逐字节一致"
    );
}

#[test]
/// **部分循环**：`preamble`/`tools`/`cwd` 用真实生产者产出，
/// 但 `rules`/`docs`/`project_context` **取自 golden**（该测试的注释原本就写明了这一点）。
fn assembled_sections_from_real_producers_match_golden() {
    // 用**生产者函数**产出的三個 section + golden 里其余 section，整体拼出来要等于请求体正文。
    let sections = golden_sections("S23");
    let mut produced = std::collections::BTreeMap::new();
    produced.insert(
        "preamble".to_string(),
        pi_core::system_prompt::PREAMBLE.to_string(),
    );
    produced.insert(
        "tools".to_string(),
        pi_core::system_prompt::tools_section(&pi_core::system_prompt::builtin_tool_snippets()),
    );
    let cwd_inner = sections["cwd"]
        .as_str()
        .expect("是字符串")
        .strip_prefix("<cwd>\n")
        .and_then(|rest| rest.strip_suffix("\n</cwd>"))
        .expect("格式为 <cwd>…</cwd>")
        .to_string();
    produced.insert(
        "cwd".to_string(),
        pi_core::system_prompt::cwd_section(&cwd_inner),
    );
    // 其余 section（rules / docs / project_context）还没有生产者，取自 golden。
    for (key, value) in &sections {
        if let Some(text) = value.as_str() {
            produced
                .entry(key.clone())
                .or_insert_with(|| text.to_string());
        }
    }
    assert_eq!(
        assemble(&produced),
        content_of(&golden_system_json("S23")),
        "三个真实生产者 + 其余 golden 段落，整体必须等于请求体正文"
    );
}

#[test]
fn rules_section_is_a_constant_across_scenarios() {
    let a = golden_sections("S23");
    let b = golden_sections("S01");
    let golden = a["rules"].as_str().expect("是字符串");
    assert_eq!(pi_core::system_prompt::RULES, golden);
    // 常量性：换个场景也应当是同一份文本。
    assert_eq!(
        golden,
        b["rules"].as_str().expect("是字符串"),
        "rules 段在所有场景里都是同一份文本"
    );
}

/// `docs` 段：把三个路径替换成 `<pkg>/{README.md,docs,examples}`。
///
/// **取值来源**：包目录从 `PI_PACKAGE_DIR`（配置，与生产同源）取；**不再从 golden 输出里反推**。
/// 结构断言（合成包目录）**无条件执行**；与 golden 的逐字节比对在 `PI_PACKAGE_DIR` 已设置时执行——
/// 因为 golden 是用录制时的包目录产生的，包目录不同则不可比（这是环境依赖，不是判据放宽）。
#[test]
fn docs_section_substitutes_the_package_dir() {
    let synthetic = "/tmp/pi-pkg-structural-test";
    let produced = pi_core::system_prompt::docs_section(synthetic);
    // `docs_section` 返回的是**已包裹**的文本：`<docs>\n…\n</docs>`。
    let mut lines = produced.lines();
    assert_eq!(lines.next(), Some("<docs>"), "首行是包裹标签");
    assert!(
        lines
            .next()
            .unwrap_or("")
            .starts_with("Pi documentation (read only"),
        "第二行是固定文本"
    );
    assert_eq!(produced.lines().last(), Some("</docs>"), "末行是包裹标签");
    for suffix in ["README.md", "docs", "examples"] {
        assert!(
            produced.contains(&format!("{synthetic}/{suffix}")),
            "<pkg>/{suffix} 必须被替换进去"
        );
    }
    assert_eq!(
        produced.lines().count(),
        10,
        "docs 段 = 2 行包裹 + 8 行正文（TS `system-prompt.ts:153-160`）"
    );
    assert!(!produced.contains("{{"), "不应残留占位符");

    let pkg = match std::env::var("PI_PACKAGE_DIR") {
        Ok(value) => value,
        Err(_) => {
            eprintln!(
                "注意：未设置 PI_PACKAGE_DIR，跳过与 golden 的逐字节比对（结构断言已执行）。\
                 用 `PI_PACKAGE_DIR=<TS 包目录> cargo test -p pi-core` 可补上这一层。"
            );
            return;
        }
    };
    let sections = golden_sections("S23");
    let golden = sections["docs"].as_str().expect("是字符串");
    assert_eq!(
        pi_core::system_prompt::docs_section(&pkg),
        golden,
        "docs 段替换包路径后必须逐字节一致"
    );
}

/// **格式验证（值取自 golden）**：源文件路径与内容都从 golden 段落里抠出来再回填，
/// 因此证明的是**格式与拼接**，不证明发现顺序与搜索范围（那两个属 P6，尚未实现）。
#[test]
fn project_context_section_reproduces_the_golden_paragraph() {
    use pi_core::system_prompt::{project_context_section, ProjectInstructionFile};

    let sections = golden_sections("S23");
    let golden = sections["project_context"].as_str().expect("是字符串");

    // 从 golden 段落里把「路径 + 内容」抽出来，再用生产者重建——
    // 这验证的是**格式与拼接**，不验证文件的发现顺序与搜索范围（那两个仍未实现）。
    let mut files = Vec::new();
    let mut rest = golden;
    while let Some(start) = rest.find("<project_instructions path=\"") {
        let after = &rest[start + "<project_instructions path=\"".len()..];
        let path_end = after.find("\">\n").expect("有路径结束");
        let path = after[..path_end].to_string();
        let body_start = path_end + "\">\n".len();
        let body_end = after[body_start..]
            .find("</project_instructions>")
            .expect("有块结束")
            + body_start;
        files.push(ProjectInstructionFile {
            path,
            content: after[body_start..body_end].to_string(),
        });
        rest = &after[body_end + "</project_instructions>".len()..];
    }
    assert_eq!(files.len(), 2, "S23 有两份项目指令文件");
    assert_eq!(
        project_context_section(&files),
        golden,
        "project_context 段的格式必须与 golden 逐字节一致"
    );
}

#[test]
fn empty_project_context_renders_as_empty_string() {
    assert_eq!(
        pi_core::system_prompt::project_context_section(&[]),
        "",
        "没有项目指令文件时不产出该段（S01 就是这种情况）"
    );
}

/// **格式验证（值取自 golden）**：`location` 从 golden 段落里抠出来再回填；
/// skills 的**发现**属 P6，尚未实现。
#[test]
fn skills_section_reproduces_the_golden_paragraph() {
    use pi_core::system_prompt::{skills_section, AvailableSkill};

    let sections = golden_sections("S24");
    let golden = sections["skills"].as_str().expect("是字符串");
    let skills = vec![AvailableSkill {
        name: "review-helper".to_string(),
        description: "用于演示 skill 展开的示例技能".to_string(),
        location: golden
            .split("<location>")
            .nth(1)
            .and_then(|rest| rest.split("</location>").next())
            .expect("有 location")
            .to_string(),
    }];
    // golden 里的 `<TMP>` 是录制后替换的占位符；实现按 ref `skills.ts:385-392` 会做
    // XML 转义（`<` ⇒ `&lt;`），故比较前先把占位符转义还原，避免把占位符差异误判为格式差异。
    let rendered = skills_section(&skills)
        .replace("&lt;TMP&gt;", "<TMP>")
        .replace("&lt;TMP/&gt;", "<TMP/>");
    assert_eq!(rendered, golden, "skills 段的格式必须与 golden 逐字节一致");
}

#[test]
fn empty_skills_render_as_empty_string() {
    assert_eq!(pi_core::system_prompt::skills_section(&[]), "");
}

/// E3b：`skills` 与 `project_context` 并存时的先后顺序（S23b 是为此专门录的场景）。
///
/// 这条顺序此前是**猜的**（把 skills 排在 project_context 之前），S23b 推翻了这个猜测。
#[test]
fn project_context_precedes_skills_when_both_present() {
    let sections = golden_sections("S23b");
    for key in ["project_context", "skills"] {
        assert!(
            sections.contains_key(key),
            "S23b 必须同时含 {key}（否则这条测试失去意义）"
        );
    }
    let order = pi_core::system_prompt::SECTION_ORDER;
    let place = |name: &str| order.iter().position(|k| *k == name).expect("在顺序表里");
    assert!(
        place("project_context") < place("skills"),
        "S23b 证实 project_context 排在 skills 之前：{order:?}"
    );
}

// —— P3.3 重做：装配规则（值取自 ref，测试与 golden 比对）——

#[test]
fn rules_section_matches_golden_without_taking_golden_as_input() {
    use pi_core::system_prompt::{builtin_tool_prompts, rules_section};
    // 输入**全部来自代码**：工具顺序是 `system-prompt.ts:48` 的默认顺序，
    // 规则文本是各工具定义里的 guidelines（出处见 `ToolPrompt` 的注释）。
    // golden 只作为**期望**，不作输入——这是与旧测试的关键区别。
    let prompts = builtin_tool_prompts();
    let names: Vec<String> = prompts.iter().map(|tool| tool.name.clone()).collect();
    let produced = rules_section(&names, &prompts, &[]);

    let golden = golden_sections("S01");
    let golden_rules = golden["rules"].as_str().expect("golden 有 rules 段");
    // golden 存的是**包好标签**的段（`<rules>\n…\n</rules>`，见 system-prompt.ts:171-174）。
    assert_eq!(
        format!("<rules>\n{produced}\n</rules>"),
        golden_rules,
        "rules 段的文本与顺序必须与 golden 一致"
    );
}

#[test]
fn tools_section_ordering_comes_from_the_code_not_golden() {
    use pi_core::system_prompt::{builtin_tool_prompts, tools_section};
    let prompts = builtin_tool_prompts();
    let pairs: Vec<(String, String)> = prompts
        .iter()
        .map(|tool| (tool.name.clone(), tool.snippet.clone()))
        .collect();
    assert_eq!(
        pairs
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        vec!["read", "bash", "edit", "write"],
        "顺序是 system-prompt.ts:48 的默认值"
    );

    // golden 的 `tools` 段 = 逐条 `- name: snippet` + 那两句固定尾巴。
    let golden = golden_sections("S01");
    let golden_tools = golden["tools"].as_str().expect("golden 有 tools 段");
    let body: Vec<String> = pairs
        .iter()
        .map(|(name, snippet)| format!("- {name}: {snippet}"))
        .collect();
    let expected = format!(
        "{}\n\n{}",
        body.join("\n"),
        pi_core::system_prompt::TOOLS_TAIL
    );
    assert_eq!(
        format!("<tools>\n{expected}\n</tools>"),
        golden_tools,
        "tools 段必须与 golden 逐字节一致"
    );
    let _ = tools_section(&pairs);
}

#[test]
fn build_sections_produces_the_source_defined_order_and_matches_the_request_body() {
    use pi_core::system_prompt::{build_sections, builtin_tool_prompts, SectionsInput};
    // 期望顺序取自 **ref 的源码**（`system-prompt.ts:145-171` 的插入顺序），
    // 不从 golden 的 `sections` 键取——serde 会把键排序，顺序在那儿已经丢了。
    let mut input = SectionsInput {
        cwd: "<TMP>/work".to_string(),
        package_dir: "/pkg".to_string(),
        selected_tools: vec![
            "read".to_string(),
            "bash".to_string(),
            "edit".to_string(),
            "write".to_string(),
        ],
        tool_prompts: builtin_tool_prompts(),
        ..SectionsInput::default()
    };
    let names = |input: &SectionsInput| -> Vec<String> {
        build_sections(input)
            .into_iter()
            .map(|(name, _)| name)
            .collect()
    };
    assert_eq!(
        names(&input),
        vec!["preamble", "tools", "rules", "docs", "cwd"],
        "S01 形态：五个段，顺序照源码"
    );

    input.context_files = vec![("AGENTS.md".to_string(), "内容".to_string())];
    assert_eq!(
        names(&input),
        vec![
            "preamble",
            "tools",
            "rules",
            "docs",
            "project_context",
            "cwd"
        ],
        "带项目指令时 project_context 出现在 cwd **之前**"
    );

    // 非循环核对：golden 的**请求体正文**里，各段标签的出现次序必须与上面的顺序一致。
    let body = request_system_text("S23").expect("golden/S23 的请求体里有 system 正文");
    let tags = ["<tools>", "<rules>", "<docs>", "<project_context>", "<cwd>"];
    let positions: Vec<usize> = tags
        .iter()
        .map(|tag| {
            body.find(tag)
                .unwrap_or_else(|| panic!("请求体里应有 {tag}"))
        })
        .collect();
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "请求体里的段次序应与源码顺序一致，实际位置：{positions:?}"
    );
}

#[test]
fn preamble_is_the_only_unwrapped_section() {
    use pi_core::system_prompt::{build_sections, builtin_tool_prompts, SectionsInput};
    let input = SectionsInput {
        cwd: "/w".to_string(),
        package_dir: "/pkg".to_string(),
        selected_tools: vec!["read".to_string(), "bash".to_string()],
        tool_prompts: builtin_tool_prompts(),
        ..SectionsInput::default()
    };
    for (name, content) in build_sections(&input) {
        if name == "preamble" {
            assert!(!content.starts_with("<preamble>"), "preamble 不加标签");
        } else {
            assert!(
                content.starts_with(&format!("<{name}>\n"))
                    && content.ends_with(&format!("\n</{name}>")),
                "{name} 段必须包成 <{name}>…</{name}>：{}",
                &content[..content.len().min(60)]
            );
        }
    }
}

#[test]
fn custom_prompt_keeps_project_context_but_drops_tools_rules_docs() {
    use pi_core::system_prompt::{build_sections, builtin_tool_prompts, SectionsInput};
    let mut input = SectionsInput {
        cwd: "<TMP>/work".to_string(),
        package_dir: "/pkg".to_string(),
        selected_tools: vec!["read".to_string(), "bash".to_string()],
        tool_prompts: builtin_tool_prompts(),
        context_files: vec![("<TMP>/work/AGENTS.md".to_string(), "# 项目规则".to_string())],
        custom_prompt: Some("你是自定义系统提示。".to_string()),
        ..SectionsInput::default()
    };
    let sections = build_sections(&input);
    let names: Vec<String> = sections.iter().map(|(name, _)| name.clone()).collect();
    assert_eq!(
        names,
        vec!["preamble", "project_context", "cwd"],
        "ref `system-prompt.ts:143-161` 只生成自定义 preamble；`:164` 的 project_context 仍在"
    );
    assert_eq!(
        sections[0].1, "你是自定义系统提示。",
        "preamble 必须是自定义文本"
    );
    assert!(
        sections[1].1.contains("# 项目规则"),
        "project_context 必须含指令文件内容：{:?}",
        sections[1].1
    );
    input.custom_prompt = None;
    let names: Vec<String> = build_sections(&input)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(
        names,
        vec![
            "preamble",
            "tools",
            "rules",
            "docs",
            "project_context",
            "cwd"
        ]
    );
}
