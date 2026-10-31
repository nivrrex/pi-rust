//! 命令行参数解析（纯逻辑，可单测）。
//!
//! 规则逐条照 `packages/coding-agent/src/cli/args.ts`；出处写在各分支旁。
//!
//! **与 TS 的一处有意差异**：TS 把未识别的 `--flag` 收进 `unknownFlags` 交给扩展处理
//! （`args.ts:237-250`）。本项目没有扩展机制，所以这些 flag **不能静默丢掉**——解析器
//! 照样收进 `unknown_flags`，由 `main` 决定报错。同理，解析到但本项目未实现的 flag
//! （skills/themes/扩展/模板）收进 `unsupported`，也由 `main` 报错。
//! 这两条对应 RUNBOOK-4 的"命令行未知参数必须报错，不得静默忽略"。

use std::collections::BTreeMap;

/// 三种应用模式。〔args.ts:11-11〕
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Text,
    Json,
    Rpc,
}

impl Mode {
    /// 从字符串解析；非法值返回 `None`。
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "text" => Some(Mode::Text),
            "json" => Some(Mode::Json),
            "rpc" => Some(Mode::Rpc),
            other => {
                let _ = other;
                None
            }
        }
    }
}

/// 解析期的诊断。〔args.ts:13-58〕
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub is_error: bool,
    pub message: String,
}

/// 解析结果。只列本项目范围内的字段；未支持但已识别的 flag 进 `unsupported`。
#[derive(Debug, Clone, Default)]
pub struct Args {
    pub messages: Vec<String>,
    pub file_args: Vec<String>,
    pub mode: Option<Mode>,
    pub print: bool,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub models: Vec<String>,
    pub api_key: Option<String>,
    pub thinking: Option<String>,
    pub system_prompt: Option<String>,
    pub append_system_prompt: Vec<String>,
    pub no_session: bool,
    pub continue_session: bool,
    pub resume: bool,
    pub session: Option<String>,
    pub session_id: Option<String>,
    pub session_dir: Option<String>,
    pub name: Option<String>,
    pub no_tools: bool,
    pub no_builtin_tools: bool,
    pub tools: Vec<String>,
    pub exclude_tools: Vec<String>,
    pub offline: bool,
    pub verbose: bool,
    pub help: bool,
    pub version: bool,
    pub list_models: Option<String>,
    /// 未识别的 `--flag`（值可能是字符串或"仅出现"）。〔args.ts:237-250〕
    pub unknown_flags: BTreeMap<String, Option<String>>,
    /// 已识别但本项目未实现的 flag，供 `main` 明确报错而不是静默忽略。
    pub unsupported: Vec<String>,
    /// `--no-context-files` / `-nc`：置真时不加载项目指令文件（ref `resource-loader.ts:515-521`）。
    pub no_context_files: bool,
    /// `--no-skills` / `-ns`：置真时不做技能发现（ref `cli/args.ts:198-199`、`resource-loader.ts:468-470`）。
    pub no_skills: bool,
    /// `--skill <path>`（可重复）：显式技能路径（ref `cli/args.ts:181`）。
    pub skill_paths: Vec<String>,
    /// `--no-prompt-templates` / `-np`：置真时不做模板发现（ref `cli/args.ts:200-201`）。
    pub no_prompt_templates: bool,
    /// `--tui-mode <mode>`：`regular`（默认）或 `fullscreen`（ref `cli/args.ts:50/213-219/325`）。
    ///
    /// 取值合法时记在 `Some`；缺值 ⇒ 记 error 诊断（文案照 `cli/args.ts:219`）。
    /// **消费点只有交互模式**（ref 的读取点全在 `modes/interactive/`）⇒ print/json/rpc 仅放行不消费。
    pub tui_mode: Option<String>,
    /// `--prompt-template <path>`（可重复）：显式模板路径（ref `cli/args.ts:184`）。
    pub prompt_template_paths: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Args {
    /// 是否有错误级诊断。
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.is_error)
    }

    /// 应用模式判定。〔main.ts:111-126〕
    ///
    /// 优先级：`--mode rpc` → rpc；`--mode json` → json；`--print` **或** stdin/stdout
    /// 非 TTY → print（text/json 由 `toPrintOutputMode` 再定）；否则交互。
    pub fn app_mode(&self, stdin_is_tty: bool, stdout_is_tty: bool) -> AppMode {
        match self.mode {
            Some(Mode::Rpc) => AppMode::Rpc,
            Some(Mode::Json) => AppMode::PrintJson,
            Some(Mode::Text) | None => {
                if self.print || !stdin_is_tty || !stdout_is_tty {
                    match self.mode {
                        Some(Mode::Json) => AppMode::PrintJson,
                        Some(Mode::Text) | Some(Mode::Rpc) | None => AppMode::PrintText,
                    }
                } else {
                    AppMode::Interactive
                }
            }
        }
    }
}

/// 最终要跑的模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    PrintText,
    PrintJson,
    Rpc,
    Interactive,
}

/// 解析参数。
pub fn parse(args: &[String]) -> Args {
    let mut result = Args::default();
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].clone();
        index += 1;

        // `--` 之后全部按位置参数处理，然后停止解析。〔args.ts:82-90〕
        if arg == "--" {
            while index < args.len() {
                let rest = args[index].clone();
                index += 1;
                push_positional(&mut result, &rest);
            }
            break;
        }

        if let Some(file) = arg.strip_prefix('@') {
            result.file_args.push(file.to_string()); // 去掉 `@` 前缀
            continue;
        }
        if arg.starts_with("--") {
            if !handle_long_flag(&mut result, &arg, args, &mut index) {
                // 未识别：带 `=` 按 key=value，否则若下一个参数既不以 `-` 也不以 `@` 开头
                // 就当值吃掉，否则记 true。〔args.ts:237-250〕
                match arg.find('=') {
                    Some(eq) => {
                        result
                            .unknown_flags
                            .insert(arg[2..eq].to_string(), Some(arg[eq + 1..].to_string()));
                    }
                    None => {
                        let flag = arg.strip_prefix("--").unwrap_or_default().to_string();
                        let next = args.get(index);
                        match next {
                            Some(value) if !value.starts_with('-') && !value.starts_with('@') => {
                                result.unknown_flags.insert(flag, Some(value.clone()));
                                index += 1;
                            }
                            Some(_) | None => {
                                result.unknown_flags.insert(flag, None);
                            }
                        }
                    }
                }
            }
            continue;
        }
        if arg.starts_with('-') {
            // 先认短选项（`-c`、`-nt` 这类），再报未知。〔args.ts:110-156〕
            if handle_short_flag(&mut result, &arg, args, &mut index) {
                continue;
            }
            // 单横线的未知选项直接进 diagnostics。〔args.ts:251-252〕
            result.diagnostics.push(Diagnostic {
                is_error: true,
                message: format!("Unknown option: {arg}"),
            });
            continue;
        }
        result.messages.push(arg);
    }
    result
}

fn push_positional(result: &mut Args, arg: &str) {
    if let Some(rest) = arg.strip_prefix('@') {
        result.file_args.push(rest.to_string());
    } else {
        result.messages.push(arg.to_string());
    }
}

/// 处理短选项；返回 `false` 表示未识别（调用方按"未知单横线选项"报错）。
fn handle_short_flag(result: &mut Args, arg: &str, args: &[String], index: &mut usize) -> bool {
    match arg {
        "-c" => result.continue_session = true,
        "-r" => result.resume = true,
        "-p" => {
            result.print = true;
            if let Some(next) = args.get(*index) {
                if !next.starts_with('@') && (!next.starts_with('-') || next.starts_with("---")) {
                    result.messages.push(next.clone());
                    *index += 1;
                }
            }
        }
        "-n" => match args.get(*index) {
            Some(value) => {
                result.name = Some(value.clone());
                *index += 1;
            }
            None => result.diagnostics.push(Diagnostic {
                is_error: true,
                message: "--name requires a value".to_string(),
            }),
        },
        "-h" => result.help = true,
        "-v" => result.version = true,
        "-nt" => result.no_tools = true,
        "-nbt" => result.no_builtin_tools = true,
        "-t" => {
            if let Some(value) = args.get(*index) {
                result.tools = split_list(value);
                *index += 1;
            }
        }
        "-xt" => {
            if let Some(value) = args.get(*index) {
                result.exclude_tools = split_list(value);
                *index += 1;
            }
        }
        "-a" => result.unsupported.push("--approve".to_string()),
        "-na" => result.unsupported.push("--no-approve".to_string()),
        "-e" => result.unsupported.push("--extension".to_string()),
        "-ne" => result.unsupported.push("--no-extensions".to_string()),
        "-ns" => result.no_skills = true,
        "-np" => result.no_prompt_templates = true,
        "-nc" => result.no_context_files = true,
        _unrecognized => return false,
    }
    true
}

/// 处理已识别的长选项；返回 `false` 表示未识别。
fn handle_long_flag(result: &mut Args, arg: &str, args: &[String], index: &mut usize) -> bool {
    // 取 `--name` 与 `--name=value` 两种形式的键与内联值。
    let (key, inline) = match arg.find('=') {
        Some(eq) => (&arg[2..eq], Some(arg[eq + 1..].to_string())),
        None => (&arg[2..], None),
    };
    // 需要值的选项：内联值优先，否则吃掉下一个参数（存在时）。
    macro_rules! value {
        () => {
            match inline.clone() {
                Some(inline_value) => Some(inline_value),
                None => match args.get(*index) {
                    Some(next) => {
                        *index += 1;
                        Some(next.clone())
                    }
                    None => None,
                },
            }
        };
    }
    match key {
        "mode" => match value!() {
            Some(text) => match Mode::parse(&text) {
                Some(mode) => result.mode = Some(mode),
                None => result.diagnostics.push(Diagnostic {
                    is_error: true,
                    message: format!("Invalid mode \"{text}\". Valid values: text, json, rpc"),
                }),
            },
            None => result.diagnostics.push(Diagnostic {
                is_error: true,
                message: "--mode requires text, json, or rpc".to_string(),
            }),
        },
        "print" => {
            result.print = true;
            // `--print` 顺手吃掉紧跟其后的一个位置参数，条件是它不是 `@` 开头，
            // 且不以 `-` 开头或以 `---` 开头。〔args.ts:167-173〕
            if let Some(next) = args.get(*index) {
                if !next.starts_with('@') && (!next.starts_with('-') || next.starts_with("---")) {
                    result.messages.push(next.clone());
                    *index += 1;
                }
            }
        }
        "provider" => result.provider = value!(),
        "model" => result.model = value!(),
        "api-key" => result.api_key = value!(),
        "thinking" => result.thinking = value!(),
        "system-prompt" => result.system_prompt = value!(),
        "append-system-prompt" => {
            if let Some(text) = value!() {
                result.append_system_prompt.push(text);
            }
        }
        "no-session" => result.no_session = true,
        "continue" | "c" => result.continue_session = true,
        "resume" | "r" => result.resume = true,
        "session" => result.session = value!(),
        "session-id" => result.session_id = value!(),
        "session-dir" => result.session_dir = value!(),
        "name" | "n" => match value!() {
            Some(text) => result.name = Some(text),
            None => result.diagnostics.push(Diagnostic {
                is_error: true,
                message: "--name requires a value".to_string(),
            }),
        },
        "models" => {
            if let Some(text) = value!() {
                result.models = split_list(&text);
            }
        }
        "no-tools" | "nt" => result.no_tools = true,
        "no-builtin-tools" | "nbt" => result.no_builtin_tools = true,
        "tools" | "t" => {
            if let Some(text) = value!() {
                result.tools = split_list(&text);
            }
        }
        // `--skill <path>`（可重复；ref `cli/args.ts:181`、`resource-loader.ts:477-503`）。
        "skill" => {
            if let Some(text) = value!() {
                result.skill_paths.push(text);
            }
        }
        // `--tui-mode <mode>`（ref `cli/args.ts:213-219`）：缺值 ⇒ error 诊断；值非法也先记下
        // （**运行期**文案以实测为准：`Error: Invalid TUI mode "<v>". Valid values: regular, fullscreen`）。
        "tui-mode" => match value!() {
            Some(mode) => result.tui_mode = Some(mode),
            None => result.diagnostics.push(Diagnostic {
                is_error: true,
                message: "--tui-mode requires regular or fullscreen".to_string(),
            }),
        },
        // `--prompt-template <path>`（可重复；ref `cli/args.ts:184`）。
        "prompt-template" => {
            if let Some(text) = value!() {
                result.prompt_template_paths.push(text);
            }
        }
        // `--no-prompt-templates` / `-np`：**只放行这两个名字**（布尔，不吃值）。
        "no-prompt-templates" | "np" => {
            if inline.is_some() {
                return false;
            }
            result.no_prompt_templates = true
        }
        // `--no-skills` / `-ns`：**只放行这两个名字**（布尔，不吃值）。
        "no-skills" | "ns" => {
            if inline.is_some() {
                return false;
            }
            result.no_skills = true
        }
        "exclude-tools" | "xt" => {
            if let Some(text) = value!() {
                result.exclude_tools = split_list(&text);
            }
        }
        "offline" => result.offline = true,
        "verbose" => result.verbose = true,
        "help" | "h" => result.help = true,
        "version" | "v" => result.version = true,
        "list-models" => {
            // 可带搜索模式；不带时记空串（表示"列出全部"）。
            let search = match inline.clone() {
                Some(inline_value) => Some(inline_value),
                None => match args.get(*index) {
                    Some(next) if !next.starts_with('-') && !next.starts_with('@') => {
                        *index += 1;
                        Some(next.clone())
                    }
                    Some(_) | None => None,
                },
            };
            result.list_models = Some(search.unwrap_or_default());
        }
        // `--no-context-files` / `-nc`：**只放行这两个名字**（布尔开关，不吃值；带 `=值`
        // 的形式仍走"未识别"）。语义照 ref `resource-loader.ts:515-521`。
        "no-context-files" | "nc" => {
            // ref `args.ts:205` 只匹配裸旗标；带 `=值` 的形式照 ref 不识别。
            if inline.is_some() {
                return false;
            }
            result.no_context_files = true
        }
        // 已识别但本项目未实现：交给 main 明确报错，不静默忽略。
        "extension" | "e" | "no-extensions" | "ne" | "theme" | "no-themes" | "use-theme"
        | "export" | "approve" | "a" | "no-approve" | "na" => {
            // 这几个带值；吃掉值以免它被当成位置参数。
            if matches!(key, "use-theme" | "theme" | "extension") {
                let _ = value!();
            }
            result.unsupported.push(format!("--{key}"));
        }
        _unrecognized => return false,
    }
    true
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}
