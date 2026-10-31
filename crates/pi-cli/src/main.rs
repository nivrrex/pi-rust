//! pi-cli：`pi-rs` 二进制入口。
//!
//! 规格来源：`ref/pi-0.87.1/packages/coding-agent/src/main.ts` 与 `cli/args.ts`，
//! 抽取结果见 `spec/i-cli.md`。
//!
//! **职责边界**：解析参数（`args.rs`，纯逻辑）、判定模式、准备运行期（`runtime.rs`）、
//! 跑 print 模式（`print_mode.rs`）。遇到不该继续的情况一律**明确报错退出**。
//!
//! **与 TS 的一处有意差异**：TS 把未识别的 `--flag` 交给扩展处理；本项目没有扩展机制，
//! 因此未识别的 `--flag` 与已识别但未实现的 flag 都报错退出，不静默忽略。

#![forbid(unsafe_code)]

// 用**库**里的解析器，不要在 bin 里再声明一份 `mod args;`——两份会是不同的类型
// （`args::Args` 与 `pi_cli::args::Args`），编译器会以类型不匹配的形式指出这一点。
use pi_cli::args::{self, AppMode, Args};
use std::io::IsTerminal;
use std::process::ExitCode;
use std::sync::Arc;

/// 加载内置模型目录（P7-6am/M）。
///
/// ref 的目录存在 `<agentDir>/models-store.json`（`core/models-store.ts:46-60`，形状为
/// `{provider: {models:[…], checkedAt, etag}}`）；也兼容 oracle 导出的 `{provider:[…]}` 形状
/// （如 `pi-official-catalog.json`）。缺失或解析失败都**不致命**：退回"无目录"（即原来的行为）。
fn load_builtin_catalog(agent_dir: &std::path::Path) -> Option<pi_ai::config::BuiltInCatalog> {
    // **必须走 `runtime::load_builtin_catalog`**：它先试编译期内嵌目录，再回落
    // `<agentDir>/models-store.json` / `pi-official-catalog.json`。
    // 此前这里只读文件、**不含内嵌目录** ⇒ 只放 `auth.json` 的机器（pi 盒子）上
    // 目录为空：`resolve_with_catalog` 报 "models.json 中没有名为 openrouter 的 provider"，
    // 交互模式还会因为"无模型"把 `/model` 退化成硬编码的 mock-model。
    pi_cli::runtime::load_builtin_catalog(agent_dir)
}

/// `-c`/`--resume` 的 cwd 过滤（ref `session-manager.ts:1791-1796`）：
/// 只有自定义会话目录才按 header `cwd` 过滤；默认分区目录里取最新一份，不看 header。
fn continue_cwd_filter<'a>(
    paths: &pi_cli::runtime::Paths,
    cwd: &'a std::path::Path,
) -> Option<&'a std::path::Path> {
    pi_cli::session_writer::should_filter_by_cwd(&paths.session_dir, cwd, &paths.agent_dir)
        .then_some(cwd)
}

/// 构建时刻（UTC，`YYYY-MM-DDTHH:MM:SSZ`）——由 `build.rs` 注入的 epoch 换算。
fn build_stamp() -> String {
    let epoch: u64 = env!("PI_BUILD_EPOCH").parse().unwrap_or(0);
    let days = epoch / 86_400;
    let seconds = epoch % 86_400;
    // 民用日期换算（Howard Hinnant 的 days-from-civil 逆算法），避免引入时间库。
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        seconds / 3600,
        (seconds % 3600) / 60,
        seconds % 60
    )
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let parsed = args::parse(&argv);

    if parsed.has_errors() {
        // 解析期诊断的打印格式照 `main.ts:608-615`：`Error: <message>` / `Warning: <message>`
        // （TS 对任一 error 诊断 `process.exit(1)`；W1(b) 实测缺值 strerr =
        // `Error: --tui-mode requires regular or fullscreen`）。
        for diagnostic in &parsed.diagnostics {
            let prefix = if diagnostic.is_error {
                "Error"
            } else {
                "Warning"
            };
            eprintln!("{prefix}: {}", diagnostic.message);
        }
        return ExitCode::from(1);
    }
    if parsed.help {
        print_help();
        return ExitCode::SUCCESS;
    }
    if parsed.version {
        println!("pi-rs {}", pi_core::VERSION);
        // P8d/B2：构建信息（提交号 + 构建时刻）。冒烟脚本用它确认"跑的是不是最新二进制"。
        println!("built {} commit {}", build_stamp(), env!("PI_BUILD_COMMIT"));
        return ExitCode::SUCCESS;
    }
    // `--tui-mode` 的取值校验（ref `cli/args.ts:213-219`；**运行期文案以 W1(d) 实测为准**：
    // TS 对非法值输出 `Error: Invalid TUI mode "<v>". Valid values: regular, fullscreen` 并 rc=1）。
    if let Some(mode) = parsed.tui_mode.as_deref() {
        if mode != "regular" && mode != "fullscreen" {
            eprintln!("Error: Invalid TUI mode \"{mode}\". Valid values: regular, fullscreen");
            return ExitCode::from(1);
        }
    }
    if let Some(code) = reject_unsupported(&parsed) {
        return code;
    }
    // `--list-models [search]`：列出可用模型后退出（ref `cli/list-models.ts`）。
    if let Some(search) = parsed.list_models.clone() {
        return run_list_models(&parsed, &search);
    }

    let mode = parsed.app_mode(
        std::io::stdin().is_terminal(),
        std::io::stdout().is_terminal(),
    );
    match mode {
        AppMode::PrintText => run_print(&parsed, false),
        AppMode::PrintJson => run_print(&parsed, true),
        AppMode::Rpc => run_rpc(&parsed),
        // 交互模式（P7-3/W2）：`--tui-mode fullscreen` 本批仍拒绝；`regular`（默认）在 TTY 下进最小 TUI。
        AppMode::Interactive => {
            if parsed.tui_mode.as_deref() == Some("fullscreen") {
                eprintln!("pi-rs: --tui-mode fullscreen 尚未实现（本批只做 regular 的最小 TUI）");
                return ExitCode::from(1);
            }
            if !std::io::stdin().is_terminal() {
                eprintln!("pi-rs: 模式 interactive 需要 TTY（stdin 不是终端）；用 -p/--print 或 --mode rpc");
                return ExitCode::from(1);
            }
            let home = pi_cli::runtime::home_from(&|key| std::env::var(key).ok());
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let paths = pi_cli::runtime::resolve_paths(
                &|key| std::env::var(key).ok(),
                &home,
                parsed.session_dir.as_deref(),
                &cwd,
            );
            // 启动屏的 `[Context]` 段：复用 P6-1 的发现规则，只显示路径。
            let discovered =
                pi_core::project_context::load_project_context_files(&cwd, &paths.agent_dir);
            let mut state = pi_tui::TuiState::new(
                pi_core::VERSION,
                cwd.to_string_lossy().to_string(),
                "0.0%/0 (auto)",
            );
            // `/login` 的 provider 列表必须在没有可用模型（parts=None）时也可用，
            // 故直接写到 state 上（`run()` 里若 parts 存在会用同一份覆盖，等价）。
            state.agent_dir = paths.agent_dir.clone();
            // `/login` 候选 = `models.json` 的 provider ∪ 内置目录的 provider（ref 遍历的
            // 是含内置的运行时 registry；零配置的新机器也要能用 `/login`）。
            state.login_providers = pi_cli::runtime::login_provider_ids(&paths.agent_dir);
            state.context_files = discovered
                .files
                .iter()
                .map(|(path, _)| path.clone())
                .collect();
            // 状态区右侧的模型名：取 settings.json 的 `defaultModel`（run-tui 会写入 `mock-model`）。
            // 状态区右侧的模型段：ref `components/footer.ts:170-171` 取**已解析模型的 id**，
            // 缺省显示 `no-model`（不是取 settings 的 defaultModel）。TUI 路径尚未解析模型
            // （P7-4 W3 才接入 agent loop），这里先用 settings 的 defaultModel 近似，缺失则 `no-model`。
            // 主题：ref `settings-manager.ts:782-791` 的 `theme` 键（缺省 dark）。
            // 值含 `/` 时上游视为**自定义主题文件路径**（`getTheme()` 返 undefined）。
            state.theme_name = load_settings(&paths)
                .ok()
                .and_then(|settings| settings.theme)
                .filter(|name| pi_tui::theme::BUILTIN.contains(&name.as_str()))
                .unwrap_or_else(|| pi_tui::theme::DEFAULT_NAME.to_string());
            state.model = load_settings(&paths)
                .ok()
                .and_then(|settings| settings.default_model.clone())
                .unwrap_or_else(|| "unknown".to_string());
            // W3-2：为 TUI 装配 agent 部件（与 print 路径同一批函数：settings 解析模型 →
            // models.json 校验/resolve → provider → 系统提示 → LoopConfig）。
            let agent_parts = match build_tui_agent_parts(&parsed, &paths, &cwd, &mut state) {
                Ok(parts) => parts,
                Err(code) => return code,
            };
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    eprintln!("pi-rs: 启动 TUI 失败：{error}");
                    return ExitCode::from(1);
                }
            };
            // `/login` 成功后用它重建部件（ref `completeProviderAuthentication` 重建会话）：
            // 既让"无模型启动"的当次会话能对话，也刷新 `available_models` —— `/model` 与 `Ctrl+P`
            // 的候选取自这里；此前只在 `agent_parts.is_none()` 时才给 rebuild，于是**已有模型**时
            // 登录新 provider 后 `/model` 里看不到它的模型（真机反馈）。
            let rebuild: Option<pi_cli::tui::RebuildParts> = {
                let parsed_clone = parsed.clone();
                let paths_clone = paths.clone();
                let cwd_clone = cwd.clone();
                Some(Box::new(move || {
                    let mut scratch = pi_tui::TuiState::new(
                        env!("CARGO_PKG_VERSION"),
                        cwd_clone.to_string_lossy().to_string(),
                        "",
                    );
                    scratch.agent_dir = paths_clone.agent_dir.clone();
                    match build_tui_agent_parts(
                        &parsed_clone,
                        &paths_clone,
                        &cwd_clone,
                        &mut scratch,
                    ) {
                        Ok(Some(parts)) => Some((parts, scratch.model.clone())),
                        Ok(None) | Err(_) => None,
                    }
                }))
            };
            match runtime.block_on(async { pi_cli::tui::run(state, agent_parts, rebuild) }) {
                Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
                Err(error) => {
                    eprintln!("pi-rs: TUI 运行失败：{error}");
                    ExitCode::from(1)
                }
            }
        }
    }
}

/// rpc 模式：stdin 收命令、stdout 发响应与事件。〔`spec/h-rpc.md`〕
///
/// **运行时语义**：`prompt` 会**阻塞**到该次运行结束（期间把事件逐行写到 stdout），
/// 这与 TS 的异步派发不同——TS 的 `prompt` 响应先回、事件随后到；本实现把响应写在
/// 运行**之后**。差异已在 notes 记录，未按 TS 形状对齐。
/// 启动时的模型解析（B-b 接线）：按 ref `model-resolver.ts:622-717` 的五步回落。
///
/// 数据来源全部是文件读：
/// - `models.json`：可用模型列表（`providers.<id>.models[].id`）与 provider 级 `apiKey`；
/// - `auth.json`：已存凭据（键为 provider 名）；
/// - 环境变量：`<PROVIDER>_API_KEY`（大写、非字母数字转 `_`），以及 `settings.enabledModels` 作 scoped。
fn resolve_startup_model(
    parsed: &Args,
    settings: &pi_cli::settings::Settings,
    paths: &pi_cli::runtime::Paths,
) -> Result<(String, String), String> {
    use pi_cli::settings::{resolve_initial_model, ModelRef};

    let models_path = paths.agent_dir.join("models.json");
    let models_value: serde_json::Value = std::fs::read_to_string(&models_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| serde_json::json!({}));

    let mut available: Vec<ModelRef> = Vec::new();
    let mut provider_api_key: Vec<String> = Vec::new();
    if let Some(providers) = models_value.get("providers").and_then(|v| v.as_object()) {
        for (provider, value) in providers {
            if value
                .get("apiKey")
                .and_then(|v| v.as_str())
                .is_some_and(|key| !key.trim().is_empty())
            {
                provider_api_key.push(provider.clone());
            }
            if let Some(models) = value.get("models").and_then(|v| v.as_array()) {
                for model in models {
                    if let Some(id) = model.get("id").and_then(|v| v.as_str()) {
                        available.push(ModelRef {
                            provider: provider.clone(),
                            id: id.to_string(),
                        });
                    }
                }
            }
        }
    }

    // 没有 models.json（只放 auth.json 的机器，例如 pi 盒子）时，候选来自**内置目录**：
    // 只加已认证 provider 的模型（未认证的选了也起不来）。
    if let Some(catalog) = pi_cli::runtime::load_builtin_catalog(&paths.agent_dir) {
        let authenticated = pi_cli::list_models::authenticated_providers(&paths.agent_dir, None);
        for (provider, model) in catalog.entries() {
            if authenticated.contains(provider) {
                available.push(ModelRef {
                    provider: provider.to_string(),
                    id: model.id.clone(),
                });
            }
        }
    }

    let auth_providers: Vec<String> = std::fs::read_to_string(paths.agent_dir.join("auth.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|value| value.as_object().map(|map| map.keys().cloned().collect()))
        .unwrap_or_default();

    let has_auth = |provider: &str| -> bool {
        if provider_api_key.iter().any(|item| item == provider)
            || auth_providers.iter().any(|item| item == provider)
        {
            return true;
        }
        let env_name = format!(
            "{}_API_KEY",
            provider
                .chars()
                .map(|ch| if ch.is_ascii_alphanumeric() {
                    ch.to_ascii_uppercase()
                } else {
                    '_'
                })
                .collect::<String>()
        );
        std::env::var(&env_name).is_ok_and(|value| !value.trim().is_empty())
    };

    let scoped: Vec<ModelRef> = settings
        .enabled_models
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .filter_map(|entry| {
            let (provider, rest) = entry.split_once('/')?;
            Some(ModelRef {
                provider: provider.to_string(),
                id: rest.to_string(),
            })
        })
        .collect();

    let resolved = resolve_initial_model(
        parsed.provider.as_deref(),
        parsed.model.as_deref(),
        settings,
        &scoped,
        &available,
        parsed.continue_session || parsed.session.is_some(),
        &has_auth,
    )?;
    Ok((resolved.provider, resolved.id))
}

fn run_rpc(parsed: &Args) -> ExitCode {
    use std::io::BufRead;

    let home = pi_cli::runtime::home_from(&|key| std::env::var(key).ok());
    let cwd_for_paths = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let paths = pi_cli::runtime::resolve_paths(
        &|key| std::env::var(key).ok(),
        &home,
        parsed.session_dir.as_deref(),
        &cwd_for_paths,
    );
    let settings = match load_settings(&paths) {
        Ok(settings) => settings,
        Err(code) => return code,
    };
    let (provider_name, model_id) = match resolve_startup_model(parsed, &settings, &paths) {
        Ok(pair) => pair,
        Err(error) => {
            eprintln!("pi-rs: {error}");
            return ExitCode::from(1);
        }
    };
    let (provider_name, model_id) = (provider_name.as_str(), model_id.as_str());
    let models_path = paths.agent_dir.join("models.json");
    // 没有 models.json 是**正常状态**（只放 auth.json 的机器，例如 pi 盒子）：
    // 按空配置处理，模型从内置目录解析（`resolve_with_catalog` 支持只靠目录）。
    let config = match std::fs::read_to_string(&models_path) {
        Ok(text) => match serde_json::from_str::<pi_ai::config::ModelsConfig>(&text) {
            Ok(config) => config,
            Err(error) => {
                eprintln!("pi-rs: {} 不可用：{error}", models_path.display());
                return ExitCode::from(1);
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            pi_ai::config::ModelsConfig::default()
        }
        Err(error) => {
            eprintln!("pi-rs: {} 不可用：{error}", models_path.display());
            return ExitCode::from(1);
        }
    };
    // ref `core/model-config.ts:305-312`：`Check` 失败就**整份**拒绝（不是"用到哪个才校验哪个"）。
    if let Err(error) = config.validate() {
        eprintln!("pi-rs: {} 不可用：{error}", models_path.display());
        return ExitCode::from(1);
    }
    // P7-6am/M：先尝试加载内置模型目录，供 `resolve_with_catalog` 补齐 api/baseUrl。
    let catalog = load_builtin_catalog(&paths.agent_dir);
    let resolved = match config.resolve_with_catalog(provider_name, model_id, catalog.as_ref()) {
        Ok(resolved) => resolved,
        Err(error) => {
            eprintln!("pi-rs: {error}");
            return ExitCode::from(1);
        }
    };
    let provider = match pi_cli::runtime::build_provider(&resolved, parsed.api_key.clone()) {
        Ok(provider) => provider,
        Err(error) => {
            eprintln!("pi-rs: {error}");
            return ExitCode::from(1);
        }
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("pi-rs: 起不了运行时：{error}");
            return ExitCode::from(1);
        }
    };
    // 共享状态：后台运行线程与主线程（读 stdin）都从这里取东西。
    let rpc_cwd = std::env::current_dir().unwrap_or_default();
    // `--continue`/`--resume`：把最近的会话**读回来当上下文**并续写同一个文件。
    //
    // 以前 rpc 路径**完全没有**处理这两个开关（处理只在 print 路径，见本文件后面的
    // `newest_session_file` 一段），于是 `--continue` 对 rpc 是无效的：种子会话不会被
    // 当成当前会话，`get_tree` 看到的是新建的空会话（S28 暴露的）。
    // **启动时**固定"当前会话文件"（见 `RpcShared::loaded_session_path` 的说明）：
    // 必须在任何会话文件被创建之前取，否则 `newest_session_file` 会挑到本进程新建的那份。
    let mut rpc_loaded_session: Option<std::path::PathBuf> = None;
    let (rpc_writer, rpc_transcript, rpc_system_written) = if parsed.no_session {
        (None, Vec::new(), false)
    } else if parsed.continue_session || parsed.resume {
        let Some(path) = pi_cli::session_writer::newest_session_file(
            &paths.session_dir,
            continue_cwd_filter(&paths, &cwd_for_paths),
        ) else {
            eprintln!(
                "pi-rs: 找不到可续写的会话文件（目录：{}）",
                paths.session_dir.display()
            );
            return ExitCode::from(1);
        };
        rpc_loaded_session = Some(path.clone());
        let file = match pi_core::session_io::SessionFile::read(&path) {
            Ok(file) => file,
            Err(error) => {
                eprintln!("pi-rs: 读会话文件失败：{error}");
                return ExitCode::from(1);
            }
        };
        // **压缩后必须用投影重建上下文**（与 print 路径同一规则、也与 ref 一致）：
        // 父链走法（`transcript()`）不会应用 `compaction`/`context_edit` 的投影规则，
        // 于是 `--continue` 续写会带着压缩前的原始消息、还丢掉压缩摘要 ⇒ 请求体多一条
        // （golden/S20c 的 C 层：golden 6 条、pi-rs 7 条）。`System` 仍按既有约定单独算。
        let transcript = match pi_core::agent::projection::build_session_projection(
            &file.entries,
            pi_core::agent::projection::LeafSelector::Unspecified,
        ) {
            Ok(projection) => projection
                .messages
                .into_iter()
                .filter(|message| !matches!(message, pi_ai::message::Message::System { .. }))
                .collect(),
            Err(error) => {
                eprintln!("pi-rs: 读会话消息失败：{error}");
                return ExitCode::from(1);
            }
        };
        // 文件里已经有 system 条目吗？（`transcript()` 会把它过滤掉，所以要直接看条目。）
        // 有→`system_written=true`，本轮就不再写第二条（golden 里续写只有一条 system）。
        let has_system = file.has_system_message();
        // ref `sdk.ts:196`：会话里没有 `thinking_level_change` 时**补一条**。
        // rpc 路径此前完全不写它（print 路径有），导致 S28 的 `get_tree.leafId` 与 TS 差一条。
        let had_thinking_level = file
            .entries
            .iter()
            .any(|entry| entry.kind() == "thinking_level_change");
        match pi_cli::session_writer::SessionWriter::open(&path) {
            Ok(mut writer) => {
                if !had_thinking_level {
                    let level = if resolved.reasoning { "medium" } else { "off" };
                    if let Err(error) = writer.append_thinking_level_change(level) {
                        eprintln!("pi-rs: 写 thinking_level_change 失败：{error}");
                        return ExitCode::from(1);
                    }
                }
                (Some(writer), transcript, has_system)
            }
            Err(error) => {
                eprintln!("pi-rs: 打开会话文件失败：{error}");
                return ExitCode::from(1);
            }
        }
    } else {
        (None, Vec::new(), false)
    };
    // rpc 与 print 用同一份系统提示（此前 rpc 只传 `--system-prompt` 开关值，默认为空）。
    let (rpc_sections, rpc_system_text, rpc_skills, rpc_templates) =
        assemble_system_prompt(parsed, &rpc_cwd, &paths.agent_dir);
    let rpc_compaction_settings = settings.compaction_settings();
    let shared = std::sync::Arc::new(pi_cli::rpc_session::RpcShared {
        provider,
        tools: pi_cli::print_mode::default_tools(parsed),
        cwd: rpc_cwd,
        settings: rpc_compaction_settings,
        context_window: resolved.context_window,
        model: resolved.model_id.clone(),
        skills: rpc_skills,
        agent_dir: paths.agent_dir.clone(),
        prompt_templates: rpc_templates,
        sections: rpc_sections.clone(),
        system_sections: rpc_sections.into_iter().collect(),
        api: resolved.api.clone(),
        reasoning: resolved.reasoning,
        model_json: serde_json::json!({
            "id": resolved.model_id.clone(),
            "name": resolved.name.clone().unwrap_or_else(|| resolved.model_id.clone()),
            "api": resolved.api.clone(),
            "provider": resolved.provider_name.clone(),
            "baseUrl": resolved.base_url.clone(),
            "reasoning": resolved.reasoning,
            "input": resolved.input_supported.clone(),
            "cost": {
                "input": rate_json(resolved.rates.input),
                "output": rate_json(resolved.rates.output),
                "cacheRead": rate_json(resolved.rates.cache_read),
                "cacheWrite": rate_json(resolved.rates.cache_write),
            },
            "contextWindow": resolved.context_window,
            "maxTokens": resolved.max_tokens,
        }),
        level: std::sync::Mutex::new(if resolved.reasoning { "medium" } else { "off" }.to_string()),
        system_written: std::sync::Mutex::new(rpc_system_written),
        // 会话 id/文件名在**启动时**就确定（TS 亦如此）；`--no-session` 时不预生成。
        precomputed_session: if parsed.no_session {
            None
        } else {
            Some(pi_cli::session_writer::precompute_session(
                &paths.session_dir,
            ))
        },
        // `--continue` 且原会话里没有 system 时，本轮写的那条 system 要 announce
        // （print 路径同样的规则；新建会话的首轮不 announce）。
        announce_system: std::sync::atomic::AtomicBool::new(
            (parsed.continue_session || parsed.resume) && !rpc_system_written,
        ),
        system: Some(rpc_system_text),
        max_tokens: resolved.max_tokens,
        provider_name: provider_name.to_string(),
        model_id: model_id.to_string(),
        steering: std::sync::Arc::new(pi_core::agent::PendingQueue::new()),
        follow_up: std::sync::Arc::new(pi_core::agent::PendingQueue::new()),
        cancel: std::sync::Mutex::new(tokio_util::sync::CancellationToken::new()),
        // 续写时先装入既有 transcript；`written` 相应置成它的长度（既有条目已在盘上）。
        written: std::sync::Mutex::new(rpc_transcript.len()),
        transcript: std::sync::Mutex::new(rpc_transcript),
        session: std::sync::Mutex::new(rpc_writer),
        busy: std::sync::atomic::AtomicBool::new(false),
        out: std::sync::Mutex::new(Box::new(std::io::stdout())),
        // 安全阀 2s：正常情况这段只持续到工具的首条进度上报（微秒级）；工具卡死时不让命令响应陪等。
        section: pi_core::agent::section::SectionGate::new(std::time::Duration::from_secs(2)),
        runtime: std::sync::Arc::new(runtime),
        session_dir: if parsed.no_session {
            None
        } else {
            Some(paths.session_dir.clone())
        },
        loaded_session_path: rpc_loaded_session,
    });
    let session = pi_cli::rpc_session::RpcSession::new(shared);

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        session.handle(&line);
    }
    ExitCode::SUCCESS
}

/// 费率 → JSON 数值。
///
/// **整数值要发成整数**：`serde_json` 会把 `f64` 的 `1.0` 渲染成 `1.0`，
/// 而 TS 的 `models.json` 里是整数 `1`——JSON 文本上二者不同，比较器按文本比，
/// S16 的 `get_state`/`set_model` 因此判为不同。
fn rate_json(value: f64) -> serde_json::Value {
    if value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 {
        serde_json::json!(value as i64)
    } else {
        serde_json::json!(value)
    }
}

/// 组装系统提示：sections + 按 `text.ts:15-21` 拼接后的正文。
///
/// **rpc 与 print 必须用同一份**——此前 rpc 只传了 `--system-prompt` 开关值（默认为空），
/// 于是 rpc 场景的会话文件里**根本没有 system 条目**（S09a/S09b/S10/S11 恒少 1 条），
/// 而且发给模型的请求也缺了系统提示。
///
/// `PI_PACKAGE_DIR` 与 TS 同义（`config.ts:389-393`）：差分时 runner 指向 TS 包目录，
/// 这样 docs 段的三个路径才能与 golden 一致。
/// `assemble_system_prompt` 的返回：段落、拼接后的正文、已加载技能、已加载 prompt template。
/// （用别名避免 `clippy::type_complexity`。）
type AssembledSystemPrompt = (
    Vec<(String, String)>,
    String,
    Vec<pi_core::skills::Skill>,
    Vec<pi_core::prompt_templates::PromptTemplate>,
);

fn assemble_system_prompt(
    parsed: &Args,
    cwd: &std::path::Path,
    agent_dir: &std::path::Path,
) -> AssembledSystemPrompt {
    let package_dir = std::env::var("PI_PACKAGE_DIR").unwrap_or_default();
    let selected_tools = if parsed.no_tools {
        Vec::new()
    } else {
        pi_cli::print_mode::default_tools(parsed)
            .iter()
            .map(|tool| tool.name().to_string())
            .collect::<Vec<String>>()
    };
    // 技能：`--no-skills` 时跳过发现（ref `resource-loader.ts:468-470`）；诊断逐条打 stderr
    // （前缀照 `main.ts:98-104`），有 error 诊断则退出码 1（`main.ts:896-905`）。
    let loaded_skills_result = if parsed.no_skills {
        pi_core::skills::SkillsResult::default()
    } else {
        pi_core::skills::load_skills(cwd, agent_dir, &parsed.skill_paths)
    };
    for diagnostic in &loaded_skills_result.diagnostics {
        eprintln!("Warning: {}", diagnostic.message);
    }
    let loaded_skills = loaded_skills_result.skills.clone();
    // prompt template（ref `prompt-templates.ts:222-298`）：`-np` 时跳过发现；
    // 项目目录 `<cwd>/.pi/prompts` 与 skills 同因不受信任而不加载。
    let loaded_templates_result = pi_core::prompt_templates::load_prompt_templates(
        cwd,
        agent_dir,
        &parsed.prompt_template_paths,
        !parsed.no_prompt_templates,
    );
    for diagnostic in &loaded_templates_result.diagnostics {
        eprintln!("Warning: {}", diagnostic.message);
    }
    let loaded_templates = loaded_templates_result.templates.clone();
    // ref `skills.ts:356`：`disableModelInvocation` 的技能不进 system 提示
    // （仍留在 `loaded_skills` 里供 `/skill:` 显式调用）。
    let available_skills: Vec<pi_core::system_prompt::AvailableSkill> = loaded_skills_result
        .skills
        .iter()
        .filter(|skill| !skill.disable_model_invocation)
        .map(|skill| pi_core::system_prompt::AvailableSkill {
            name: skill.name.clone(),
            description: skill.description.clone(),
            location: skill.file_path.clone(),
        })
        .collect();
    let sections = pi_core::system_prompt::build_sections(&pi_core::system_prompt::SectionsInput {
        cwd: cwd.to_string_lossy().to_string(),
        package_dir,
        selected_tools,
        tool_prompts: pi_core::system_prompt::builtin_tool_prompts(),
        prompt_guidelines: Vec::new(),
        append_system_prompt: parsed.append_system_prompt.join("\n\n"),
        // 项目指令文件（AGENTS.md 等）的发现规则：ref `core/resource-loader.ts:71-157`。
        // 读失败告警由本模块统一输出到 stderr（与 ref 的 console.error 同通道；
        // rpc 协议走 stdout，stderr 是独立诊断通道，见 rpc-client.ts:101-104）。
        context_files: {
            // `--no-context-files` / `-nc` ⇒ 不加载（ref `resource-loader.ts:515-521`）。
            if parsed.no_context_files {
                Vec::new()
            } else {
                let discovered =
                    pi_core::project_context::load_project_context_files(cwd, agent_dir);
                for warning in &discovered.warnings {
                    eprintln!("{warning}");
                }
                discovered.files
            }
        },
        skills: available_skills,
        custom_prompt: parsed.system_prompt.clone(),
    });
    let text = pi_core::system_prompt::render_system_text(&sections);
    (sections, text, loaded_skills, loaded_templates)
}

/// `--list-models [search]`：打印可用模型表后退出 0（ref `cli/list-models.ts:37-115`）。
///
/// 数据源是 `<agentDir>/models.json` 与内置目录的并集；没有可用模型时输出上游同一段
/// "No models available" 提示（`core/auth-guidance.ts:16-20`）。
fn run_list_models(parsed: &Args, search: &str) -> ExitCode {
    let home = pi_cli::runtime::home_from(&|key| std::env::var(key).ok());
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let paths = pi_cli::runtime::resolve_paths(
        &|key| std::env::var(key).ok(),
        &home,
        parsed.session_dir.as_deref(),
        &cwd,
    );
    let models_json = std::fs::read_to_string(paths.agent_dir.join("models.json")).ok();
    let catalog = pi_cli::runtime::load_builtin_catalog(&paths.agent_dir);
    // 可用 provider：`auth.json` 里有凭据的、`models.json` 里配了 `apiKey` 的、
    // 或对应环境变量已设置的（ref 的 `getAvailable` 只返回已认证 provider 的模型）。
    let mut authenticated: std::collections::BTreeSet<String> =
        std::fs::read_to_string(paths.agent_dir.join("auth.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|value| value.as_object().cloned())
            .map(|object| object.keys().cloned().collect())
            .unwrap_or_default();
    if let Some(text) = models_json.as_deref() {
        if let Ok(config) = serde_json::from_str::<pi_ai::config::ModelsConfig>(text) {
            for (name, raw) in &config.providers {
                let has_key = raw
                    .get("apiKey")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|key| !key.trim().is_empty());
                let api = raw
                    .get("api")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                let api_env = if api == "anthropic-messages" {
                    Some("ANTHROPIC_API_KEY")
                } else if api == "openai-completions" || api == "openai-responses" {
                    Some("OPENAI_API_KEY")
                } else {
                    None
                };
                let env_ready = api_env.is_some_and(|name| std::env::var(name).is_ok());
                if has_key || env_ready {
                    authenticated.insert(name.clone());
                }
            }
        }
    }
    let rows = pi_cli::list_models::collect_models(
        models_json.as_deref(),
        catalog.as_ref(),
        &authenticated,
    );
    if rows.is_empty() {
        println!(
            "No models available. Use /login to log into a provider via OAuth or API key. See:"
        );
        println!(
            "  {}/docs/providers.md",
            docs_dir(&paths.agent_dir).display()
        );
        println!("  {}/docs/models.md", docs_dir(&paths.agent_dir).display());
        return ExitCode::SUCCESS;
    }
    let filtered = pi_cli::list_models::filter_models(rows, search);
    if filtered.is_empty() {
        println!("No models matching \"{search}\"");
        return ExitCode::SUCCESS;
    }
    println!("{}", pi_cli::list_models::render_table(&filtered));
    ExitCode::SUCCESS
}

/// print 模式（text / json）。返回进程退出码。〔print-mode.ts:143-161〕
fn run_print(parsed: &Args, json: bool) -> ExitCode {
    let prompt = parsed.messages.join(" ");
    let cwd = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(error) => {
            eprintln!("pi-rs: 取不到当前目录：{error}");
            return ExitCode::from(1);
        }
    };
    let home = pi_cli::runtime::home_from(&|key| std::env::var(key).ok());
    let paths = pi_cli::runtime::resolve_paths(
        &|key| std::env::var(key).ok(),
        &home,
        parsed.session_dir.as_deref(),
        &cwd,
    );

    // 模型解析顺序：命令行 → settings.json 的 defaultProvider/defaultModel →
    // （内置默认表与"第一个可用模型"未实现，报错而不是猜）。
    let settings = match load_settings(&paths) {
        Ok(settings) => settings,
        Err(code) => return code,
    };
    let (provider_name, model_id) = match resolve_startup_model(parsed, &settings, &paths) {
        Ok(pair) => pair,
        Err(_error) => {
            print_no_model_error(&paths.agent_dir);
            return ExitCode::from(1);
        }
    };
    let (provider_name, model_id) = (provider_name.as_str(), model_id.as_str());

    let models_path = paths.agent_dir.join("models.json");
    let config = match std::fs::read_to_string(&models_path) {
        Ok(text) => match serde_json::from_str::<pi_ai::config::ModelsConfig>(&text) {
            Ok(config) => {
                // ref `core/model-config.ts:305-312`：整份校验（同 rpc 路径）。
                if let Err(error) = config.validate() {
                    eprintln!("pi-rs: {} 不可用：{error}", models_path.display());
                    return ExitCode::from(1);
                }
                config
            }
            Err(error) => {
                eprintln!("pi-rs: {} 解析失败：{error}", models_path.display());
                return ExitCode::from(1);
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            pi_ai::config::ModelsConfig::default()
        }
        Err(error) => {
            eprintln!("pi-rs: 读不到 {}：{error}", models_path.display());
            return ExitCode::from(1);
        }
    };
    // P7-6am/M：先尝试加载内置模型目录，供 `resolve_with_catalog` 补齐 api/baseUrl。
    let catalog = load_builtin_catalog(&paths.agent_dir);
    let resolved = match config.resolve_with_catalog(provider_name, model_id, catalog.as_ref()) {
        Ok(resolved) => resolved,
        Err(error) => {
            eprintln!("pi-rs: {error}");
            return ExitCode::from(1);
        }
    };
    let provider = match pi_cli::runtime::build_provider(&resolved, parsed.api_key.clone()) {
        Ok(provider) => provider,
        Err(error) => {
            eprintln!("pi-rs: {error}");
            return ExitCode::from(1);
        }
    };

    // 系统提示：按 `system-prompt.ts` 的规则组装，再按 `text.ts:15-21` 拼接。
    //
    // **`PI_PACKAGE_DIR` 与 TS 同义**（`config.ts:389-393`）：差分时 runner 会把它指向
    // TS 包目录，这样 docs 段里的三个路径才能与 golden 一致（golden 记的是绝对路径）。
    // 生产环境应指向 pi-rs 自己的包目录——这一步**尚未实现**，见 notes 的已知缺口。
    let (sections, system_text, loaded_skills, loaded_templates) =
        assemble_system_prompt(parsed, &cwd, &paths.agent_dir);

    let run_cwd = cwd.clone();
    // json 模式必须**实时**输出（外部要能在运行中等到事件）；text 模式不需要。
    let sink: Arc<dyn pi_core::agent::EventSink> = if json {
        Arc::new(pi_cli::print_mode::StdoutJsonSink)
    } else {
        Arc::new(pi_cli::print_mode::NoopEventSink)
    };
    let mut written: usize = 0;
    // `--continue`/`--resume` 时从已有会话重建的消息；非继续时为空。
    let mut initial_messages: Vec<pi_ai::message::Message> = Vec::new();
    // `--continue` 且会话里原先**没有 system** 时，本轮会补写一条 system 条目——
    // 它是本轮的待处理消息，循环要 announce 它（golden/S17 的 B 层有两对消息事件）。
    let mut announce_system = false;
    // 推理模型才发这两项（golden/S16 实测；S01 等非推理场景不出现）：
    // system 的角色用 `developer`（`openai-completions.ts:1225`），
    // 思考等级下发成 `reasoning_effort`（`:925-928`）。
    let instruction_role = if resolved.reasoning {
        Some("developer".to_string())
    } else {
        None
    };
    let reasoning_effort = if resolved.reasoning {
        Some(if resolved.reasoning { "medium" } else { "off" }.to_string())
    } else {
        None
    };
    let session_writer = if parsed.no_session {
        None
    } else if parsed.continue_session || parsed.resume {
        // **续写**：找会话目录里**最新**的 `.jsonl`，从它重建上下文，并**续写同一文件**。
        // golden/S14 实测：续写后会话里只有**一条** `session`、没有重复的 `model_change`，
        // 且 `system` 只出现一次——所以 `open` 不写 header/system 条目。
        let Some(path) = pi_cli::session_writer::newest_session_file(
            &paths.session_dir,
            continue_cwd_filter(&paths, &cwd),
        ) else {
            eprintln!(
                "pi-rs: 找不到可续写的会话文件（目录：{}）",
                paths.session_dir.display()
            );
            return ExitCode::from(1);
        };
        let had_thinking_level = match pi_core::session_io::SessionFile::read(&path) {
            Ok(file) => {
                // ref `sdk.ts:196`：会话里**没有** `thinking_level_change` 时才补一条。
                let had = file
                    .entries
                    .iter()
                    .any(|entry| entry.kind() == "thinking_level_change");
                // 会话里**没有** system 消息时，工具声明需要补一条（`agent-loop.ts` 的 declareToolChanges）。
                let had_system = file.entries.iter().any(|entry| {
                    matches!(
                        entry,
                        pi_core::session::SessionEntry::Message {
                            message: pi_ai::message::Message::System { .. },
                            ..
                        }
                    )
                });
                // **压缩后必须用投影重建上下文**（ref 一律走 `buildSessionProjection()`）：
                // 父链走法（`transcript()`）不会应用 `compaction`/`context_edit` 的投影规则，
                // 压缩后的那一轮会带着压缩前的原始消息再发一次（golden/S19 第 3 条请求即此差异）。
                // `System` 仍按既有约定单独计（`written = 1 + initial_messages.len()`）。
                initial_messages = match pi_core::agent::projection::build_session_projection(
                    &file.entries,
                    pi_core::agent::projection::LeafSelector::Unspecified,
                ) {
                    Ok(projection) => projection
                        .messages
                        .into_iter()
                        .filter(|message| {
                            !matches!(message, pi_ai::message::Message::System { .. })
                        })
                        .collect(),
                    // **不再静默丢消息**：条目读不回就明确失败（以前 `.ok()` 会让它无声消失）。
                    Err(error) => {
                        eprintln!("pi-rs: 读会话消息失败：{error}");
                        return ExitCode::from(1);
                    }
                };
                Some((had, had_system))
            }
            Err(error) => {
                eprintln!("pi-rs: 读会话失败（{}）：{error}", path.display());
                return ExitCode::from(1);
            }
        };
        // 已加载的那几条消息**已经在文件里**（外加一条 system），所以 `written` 从它们起算。
        written = 1 + initial_messages.len();
        match pi_cli::session_writer::SessionWriter::open(&path) {
            Ok(mut writer) => {
                // **json 模式仍要打印 header 行**：golden/S14 的事件流里，第二阶段的开头
                // 又是一条 `{"type":"session",…}`（第 15 行）。会话**文件**里不重复写，
                // 但**事件流**里每次运行都要打印一次——我 `open` 分支漏了这句。
                if json {
                    println!("{}", writer.header_line());
                }

                // **提示词前的压缩检查**（ref `agent-session.ts:1693-1698`：发提示词前先用
                // `skipAbortedCheck=false` 查一次，"接住被中止的响应"——被中止的回合不会走到回合后的
                // 检查）。这里与回合后那次**共用同一份判定与执行**（`overflow_outcome` 的
                // `skip_aborted_check` 参数 + `perform_auto_compaction`），不另写一套。
                // 这里的 `writer` 就是刚打开的续写会话（不必再经 `session_writer`）。
                if let Ok(file) = pi_core::session_io::SessionFile::read(writer.path()) {
                    let assistant = file.entries.iter().rev().find_map(|entry| {
                        if let pi_core::session::SessionEntry::Message {
                            message: pi_ai::message::Message::Assistant(assistant),
                            ..
                        } = entry
                        {
                            Some((**assistant).clone())
                        } else {
                            None
                        }
                    });
                    if let Some(assistant) = assistant {
                        let config = pi_cli::auto_compaction::CompactionConfig {
                            settings: settings.compaction_settings(),
                            context_window: resolved.context_window,
                            model: resolved.model_id.clone(),
                            model_provider: provider_name.to_string(),
                            model_max_tokens: resolved.max_tokens,
                            model_reasoning: resolved.reasoning,
                            thinking_level: Some(if resolved.reasoning {
                                "medium".to_string()
                            } else {
                                "off".to_string()
                            }),
                        };
                        if let Some(plan) = pi_cli::auto_compaction::auto_compaction_plan(
                            &file.entries,
                            &config,
                            &assistant,
                            false,
                        ) {
                            match tokio::runtime::Builder::new_current_thread()
                                .enable_all()
                                .build()
                            {
                                Ok(runtime) => {
                                    // `compaction_start` 在摘要调用之前发（ref `:2769` 早于 `:2828`）。
                                    println!(
                                        "{}",
                                        serde_json::json!({
                                            "type": "compaction_start",
                                            "reason": plan.reason,
                                        })
                                    );
                                    match runtime.block_on(
                                        pi_cli::auto_compaction::perform_auto_compaction(
                                            &file.entries,
                                            &config,
                                            None,
                                            Some(plan.reason),
                                            Some(&pi_cli::print_mode::StdoutJsonSink),
                                            provider.as_ref(),
                                            &mut writer,
                                        ),
                                    ) {
                                        Ok(Some(run)) => {
                                            // **压缩后的上下文要交给本轮循环**（ref 压缩后用投影重建 `agent.state.messages`）：
                                            // 否则提示词请求里仍是压缩前的原始消息、还丢掉压缩摘要（golden/S13D 的 C 层即此）。
                                            initial_messages = run.messages.clone();
                                            println!(
                                                "{}",
                                                serde_json::json!({
                                                    "type": "compaction_end",
                                                    "reason": plan.reason,
                                                    "result": run.result_event,
                                                    "aborted": false,
                                                    "willRetry": plan.will_retry,
                                                })
                                            );
                                        }
                                        Ok(None) => {}
                                        Err(error) => eprintln!("pi-rs: 提示词前压缩失败：{error}"),
                                    }
                                }
                                Err(error) => {
                                    eprintln!("pi-rs: 起不了运行时（提示词前压缩）：{error}")
                                }
                            }
                        }
                    }
                }
                if let Some((had_thinking, had_system)) = had_thinking_level {
                    if !had_thinking {
                        if let Err(error) = writer.append_thinking_level_change("off") {
                            eprintln!("pi-rs: 写 thinking_level_change 失败：{error}");
                        }
                    }
                    if !had_system {
                        announce_system = true;
                        let tools = pi_cli::print_mode::default_tools(parsed);
                        let tools_added = pi_core::agent::tools_added_json(&tools);
                        let tools_added_raw = pi_core::agent::tools_added_raw(&tools);
                        if let Err(error) =
                            writer.append_system_message(&sections, &tools_added, &tools_added_raw)
                        {
                            eprintln!("pi-rs: 写 system 消息失败：{error}");
                        }
                    }
                }
                Some(writer)
            }
            Err(error) => {
                eprintln!("pi-rs: 打开会话失败（{}）：{error}", path.display());
                return ExitCode::from(1);
            }
        }
    } else {
        // 初始思考等级：**推理模型 `medium`、其余 `off`**
        // （`defaults.ts:3` 的 `DEFAULT_THINKING_LEVEL` + `agent-session.ts:2325` 的
        // `clampThinkingLevel(model, level)`——非推理模型被夹成 `off`）。
        // golden 实测：S01/S09a（非推理）`off`、S16（reasoning: true）`medium`。
        let thinking_level = if resolved.reasoning { "medium" } else { "off" };
        match pi_cli::session_writer::SessionWriter::create(
            &paths.session_dir,
            &run_cwd,
            provider_name,
            model_id,
            thinking_level,
        ) {
            Ok(mut writer) => {
                if json {
                    println!("{}", writer.header_line());
                }
                // **`written` 是"真的写了几条"**：没有 `--system-prompt` 时 system 条目
                // 仍然按组装好的 sections 写（golden 的 system 条目是
                // `content: ""` + `sections` + `toolsAdded`）。
                let tools = pi_cli::print_mode::default_tools(parsed);
                let tools_added = pi_core::agent::tools_added_json(&tools);
                let tools_added_raw = pi_core::agent::tools_added_raw(&tools);
                match writer.append_system_message(&sections, &tools_added, &tools_added_raw) {
                    Ok(()) => written += 1,
                    Err(error) => eprintln!("pi-rs: 写 system 消息失败：{error}"),
                }
                Some(writer)
            }
            Err(error) => {
                eprintln!("pi-rs: 建会话失败：{error}");
                None
            }
        }
    };

    // P5-3c：自动压缩参数（没有会话文件时不接——没有可压缩的上下文）。
    let compaction = session_writer
        .as_ref()
        .map(|_| pi_cli::auto_compaction::CompactionConfig {
            settings: settings.compaction_settings(),
            context_window: resolved.context_window,
            model: resolved.model_id.clone(),
            model_provider: provider_name.to_string(),
            model_max_tokens: resolved.max_tokens,
            model_reasoning: resolved.reasoning,
            thinking_level: Some(if resolved.reasoning {
                "medium".to_string()
            } else {
                "off".to_string()
            }),
        });
    let run = pi_cli::print_mode::PrintRun {
        provider,
        model: resolved.model_id.clone(),
        api: resolved.api.clone(),
        provider_name: provider_name.to_string(),
        system: Some(system_text),
        system_sections: sections.iter().cloned().collect(),
        writer: session_writer,
        written,
        initial_messages,
        compaction,
        announce_system,
        instruction_role,
        reasoning_effort,
        prompt: {
            // ref `agent-session.ts:1797-1821`：`/skill:name args` 展开；未命中 ⇒ 原样；
            // 读失败 ⇒ 按 ref `:1812-1819` 报错（`emitError`）后原样放行。
            let expanded = match pi_core::skills::expand_skill_command(&prompt, &loaded_skills) {
                Ok(Some(text)) => text,
                Ok(None) => prompt.clone(),
                Err((path, message)) => {
                    eprintln!("Error: {message} ({path})");
                    prompt.clone()
                }
            };
            // ref `agent-session.ts:1646-1651`：**先 `/skill:`、后模板**。
            pi_core::prompt_templates::expand_prompt_template(&expanded, &loaded_templates)
        },
        max_completion_tokens: resolved.max_tokens,
        cwd,
        tools: pi_cli::print_mode::default_tools(parsed),
        output: if json {
            pi_cli::print_mode::PrintOutput::Json
        } else {
            pi_cli::print_mode::PrintOutput::Text
        },
        sink: sink.clone(),
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("pi-rs: 起不了运行时：{error}");
            return ExitCode::from(1);
        }
    };
    // 会话文件在**运行之前**建好：json 模式的**事件流首行**就是会话头
    // （golden/S01 的 `events.jsonl` 第 1 行是 `{"type":"session",…}`），
    // 所以必须先把 header 打出去，再让循环开始发事件。
    let (stdout, stderr, code, _messages) = runtime.block_on(run.execute());
    if !json {
        // json 模式的正文已由 sink 逐事件写出（实时），此处不再重复输出。
        print!("{stdout}");
    }
    for line in stderr {
        eprintln!("{line}");
    }
    ExitCode::from(code)
}

/// 读 `settings.json`：文件缺失按"没有设置"处理，解析失败则报错退出。
/// W3-2：给 TUI 装配 `AgentParts`（与 print 路径同一批函数，避免两套装配漂移）。
///
/// 失败时返回 `Err(退出码)`（调用方直接 `return`）。副作用：把状态行的模型段设为**已解析模型的 id**
/// （ref `components/footer.ts:170-171` 取 `state.model?.id`，而不是 settings 的 `defaultModel`）。
/// print/json/rpc 模式下无可用模型时的报错：逐字对齐 ref `core/auth-guidance.ts:16-20`
/// （`No models available. … See:` + 两行 docs 路径），退出码 1。
fn print_no_model_error(agent_dir: &std::path::Path) {
    let docs = agent_dir.join("docs");
    eprintln!("No models available. Use /login to log into a provider via OAuth or API key. See:");
    eprintln!("  {}", docs.join("providers.md").display());
    eprintln!("  {}", docs.join("models.md").display());
}

/// 无可用模型时的启动提示，逐字对齐 ref `core/auth-guidance.ts:16-20` 的
/// `formatNoModelsAvailableMessage()`（TS 侧前缀 `Warning: ` 由调用方加）。
fn push_no_models_notice(state: &mut pi_tui::TuiState, agent_dir: &std::path::Path) {
    // ref `core/auth-guidance.ts:8-20` 的 `formatNoModelsAvailableMessage()` 是**一条**
    // 多行消息（`… See:\n  providers.md\n  models.md`），TS 侧只产生一个块 ⇒ 只跟一个空行。
    let docs = docs_dir(agent_dir);
    let text = format!(
        "Warning: No models available. Use /login to log into a provider via OAuth or API key. See:\n  {}\n  {}",
        docs.join("providers.md").display(),
        docs.join("models.md").display()
    );
    state.transcript.push(pi_tui::TranscriptItem::Notice(text));
}

/// 文档目录（ref `config.ts:445-447`：`getPackageDir()/docs`）。
///
/// pi-rs 是**单二进制**，没有安装包目录，所以按下面顺序回落：
/// 1. `PI_DOCS_DIR`（显式配置）；
/// 2. 可执行文件旁的 `docs/`（自解压/绿色部署时可用）；
/// 3. 公开仓库的 docs 目录（本仓库不随二进制分发 docs 页面，见 LIMITATIONS.md）。
fn docs_dir(agent_dir: &std::path::Path) -> std::path::PathBuf {
    if let Some(dir) = std::env::var_os("PI_DOCS_DIR") {
        return std::path::PathBuf::from(dir);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let candidate = parent.join("docs");
            if candidate.is_dir() {
                return candidate;
            }
        }
    }
    let _ = agent_dir;
    std::path::PathBuf::from("https://github.com/nivrrex/pi-rust/tree/main/docs")
}

fn build_tui_agent_parts(
    parsed: &Args,
    paths: &pi_cli::runtime::Paths,
    cwd: &std::path::Path,
    state: &mut pi_tui::TuiState,
) -> Result<Option<std::sync::Arc<pi_cli::tui::AgentParts>>, ExitCode> {
    let settings = load_settings(paths)?;
    let (provider_name, model_id) = match resolve_startup_model(parsed, &settings, paths) {
        Ok(pair) => pair,
        Err(error) => {
            // ref 允许 `model === undefined` 进入交互模式（footer 显示 `no-model`），
            // 由用户在里面 `/login` 或 `/model` 补齐；这里**不再直接退出**，也**不打印
            // 额外 stderr**（TS 只把提示写进转录区，见 A-b 对拍）。
            state.model = "unknown".to_string();
            // ref `core/auth-guidance.ts:16-20`：`No models available. Use /login …`
            let _ = error;
            push_no_models_notice(state, &paths.agent_dir);
            return Ok(None);
        }
    };
    let models_path = paths.agent_dir.join("models.json");
    // 同上：没有 models.json 时按空配置处理（模型从内置目录解析）。
    let models_config = match std::fs::read_to_string(&models_path) {
        Ok(text) => match serde_json::from_str::<pi_ai::config::ModelsConfig>(&text) {
            Ok(config) => config,
            Err(error) => {
                eprintln!("pi-rs: {} 不可用：{error}", models_path.display());
                return Err(ExitCode::from(1));
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            pi_ai::config::ModelsConfig::default()
        }
        Err(error) => {
            eprintln!("pi-rs: {} 不可用：{error}", models_path.display());
            return Err(ExitCode::from(1));
        }
    };
    if let Err(error) = models_config.validate() {
        eprintln!("pi-rs: {} 不可用：{error}", models_path.display());
        return Err(ExitCode::from(1));
    }
    // P7-6am/M：rpc 路径同样使用内置目录补齐。
    let catalog = load_builtin_catalog(&paths.agent_dir);
    let resolved = match models_config.resolve_with_catalog(
        provider_name.as_str(),
        model_id.as_str(),
        catalog.as_ref(),
    ) {
        Ok(resolved) => resolved,
        Err(error) => {
            eprintln!(
                "pi-rs[tui-parts]: {provider_name}/{model_id} catalog={} err={error}",
                catalog.is_some()
            );
            return Err(ExitCode::from(1));
        }
    };
    let provider = match pi_cli::runtime::build_provider(&resolved, parsed.api_key.clone()) {
        Ok(provider) => provider,
        Err(error) => {
            eprintln!("pi-rs: {error}");
            // 没有可用凭据：ref 的交互模式允许 model 为空、由用户 `/login` 补齐，
            // 因此这里**不退出**，进 TUI 并把报错作为状态行显示。
            state.model = "unknown".to_string();
            let _ = error;
            push_no_models_notice(state, &paths.agent_dir);
            return Ok(None);
        }
    };
    let (sections, system_text, _skills, _templates) =
        assemble_system_prompt(parsed, cwd, &paths.agent_dir);
    // 无 usage 时 footer 百分比用系统消息的 token 估算（见 `tui.rs` 的 `TuiEventSink`）。
    //
    // **必须按 TS 会话里 system 条目的形态构造**：`content` 为空串，正文放在 `sections`/`toolsAdded`
    // （ref `compaction.ts:320-336` 的 `estimateTokens` 对 system 分支正是把 `sections` 与 `toolsAdded`
    // 各自的长度计入；只算 `content` 会显著偏小 —— 实测 T05 曾因此得 0.5% 而 golden 是 1.1%）。
    let system_message = {
        let tools_for_sections = pi_cli::print_mode::default_tools(parsed);
        let tools_added_for_sections = pi_core::agent::tools_added_json(&tools_for_sections);
        let mut sections_map = serde_json::Map::new();
        for (name, body) in &sections {
            sections_map.insert(name.clone(), serde_json::Value::String(body.clone()));
        }
        let mut extra = serde_json::Map::new();
        extra.insert(
            "sections".to_string(),
            serde_json::Value::Object(sections_map),
        );
        if !tools_added_for_sections.is_empty() {
            extra.insert(
                "toolsAdded".to_string(),
                serde_json::Value::Array(tools_added_for_sections),
            );
        }
        pi_ai::message::Message::System {
            content: pi_ai::UserContent::Text(String::new()),
            timestamp: 0,
            extra,
        }
    };
    state.model = resolved.model_id.clone();
    // TUI 会话落盘（W4 会话层）：与 print 路径同一批调用（`SessionWriter::create` + `append_system_message`）。
    // `--continue/-c` / `--resume`：**续写最近一个会话**（而不是新建），并把历史消息读出来
    // （ref `sdk.ts` 的 continue 语义；此前 TUI 路径忽略这两个开关，导致续写会话是空屏、`/tree` 无历史）。
    let resumed_path = if parsed.continue_session || parsed.resume {
        pi_cli::session_writer::newest_session_file(
            &paths.session_dir,
            continue_cwd_filter(paths, cwd),
        )
    } else {
        None
    };
    let initial_messages: Vec<pi_ai::message::Message> = resumed_path
        .as_ref()
        .and_then(|path| pi_core::session_io::SessionFile::read(path).ok())
        .map(|file| {
            file.entries
                .iter()
                .filter_map(|entry| match entry {
                    pi_core::session::SessionEntry::Message { message, .. } => {
                        Some(message.clone())
                    }
                    other => {
                        let _ = other;
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let writer = if parsed.no_session {
        None
    } else if let Some(path) = resumed_path.as_ref() {
        match pi_cli::session_writer::SessionWriter::open(path) {
            Ok(writer) => Some(std::sync::Arc::new(std::sync::Mutex::new(writer))),
            Err(error) => {
                eprintln!("pi-rs: 续写会话失败：{error}");
                None
            }
        }
    } else {
        let thinking_level = if resolved.reasoning { "medium" } else { "off" };
        let tools = pi_cli::print_mode::default_tools(parsed);
        let tools_added = pi_core::agent::tools_added_json(&tools);
        let tools_added_raw = pi_core::agent::tools_added_raw(&tools);
        match pi_cli::session_writer::SessionWriter::create(
            &paths.session_dir,
            cwd,
            provider_name.as_str(),
            resolved.model_id.as_str(),
            thinking_level,
        ) {
            Ok(mut writer) => {
                match writer.append_system_message(&sections, &tools_added, &tools_added_raw) {
                    Ok(()) => Some(std::sync::Arc::new(std::sync::Mutex::new(writer))),
                    Err(error) => {
                        eprintln!("pi-rs: 写 system 消息失败：{error}");
                        Some(std::sync::Arc::new(std::sync::Mutex::new(writer)))
                    }
                }
            }
            Err(error) => {
                eprintln!("pi-rs: 建会话失败：{error}");
                None
            }
        }
    };
    // P7-7f M：该 provider 下的可选模型（`models.json` 顺序）与当前模型状态。
    // 可切换模型列表：**跨 provider**。ref 用 `settings.enabledModels`（形如
    // `provider/modelId`；模型 id 自身可能含 `/` ⇒ 按**第一个** `/` 切分）。
    // 没有该设置时回落到"当前 provider 下的全部模型"。
    // `-c`/`/resume`：从会话里取**最后一个** `thinking_level_change` 的档位（ref 的
    // `sessionManager` 读回 `thinkingLevel`）。
    let initial_thinking_level: Option<String> = resumed_path
        .as_ref()
        .and_then(|path| pi_core::session_io::SessionFile::read(path).ok())
        .and_then(|file| {
            file.entries.iter().rev().find_map(|entry| match entry {
                pi_core::session::SessionEntry::ThinkingLevelChange { thinking_level, .. } => {
                    Some(thinking_level.clone())
                }
                _other => None,
            })
        })
        .or_else(|| {
            // ref `sdk.ts:230-254` 的取值顺序：会话里的 `thinking_level_change` →
            // ref `sdk.ts:230-254` 的取值顺序：会话里的 `thinking_level_change` →
            // 每模型覆盖（`modelThinkingLevels["provider/modelId"]`）→
            // `settings.defaultThinkingLevel` → `DEFAULT_THINKING_LEVEL`。
            let per_model = settings
                .model_thinking_levels
                .as_ref()
                .and_then(|map| map.get(&format!("{provider_name}/{model_id}")).cloned());
            per_model.or_else(|| settings.default_thinking_level.clone())
        });
    let available_models: Vec<pi_cli::tui::ModelChoice> = {
        let enabled: Vec<pi_cli::tui::ModelChoice> = settings
            .enabled_models
            .clone()
            .unwrap_or_default()
            .iter()
            .filter_map(|entry| entry.split_once('/'))
            .map(|(provider, model)| pi_cli::tui::ModelChoice {
                provider: provider.to_string(),
                id: model.to_string(),
            })
            .collect();
        if !enabled.is_empty() {
            // ref `core/model-resolver.ts:282-382`：启动时把 `enabledModels` 解析成
            // scope，**匹配不到的项被丢弃并打 Warning**（`No models match pattern "<p>"`），
            // 而不是留到运行时再跳过。这里做同样的过滤 + 告警。
            // scope 里的一条 `provider/model` 可能来自内置目录（用户常见写法：只写
            // `openrouter/free` 而不在 models.json 里声明该 provider）。此前 `known` 只取
            // models.json，目录里的条目匹配不上就被当"无效项"丢掉 —— 真机上表现为
            // `Model scope:` 有 openrouter/free，但 `/model` 里一个 openrouter 模型都没有。
            let mut known: std::collections::BTreeSet<(String, String)> = models_config
                .providers
                .iter()
                .flat_map(|(provider, value)| {
                    value
                        .get("models")
                        .and_then(|models| models.as_array())
                        .map(|models| {
                            models
                                .iter()
                                .filter_map(|model| model.get("id").and_then(|id| id.as_str()))
                                .map(|id| (provider.clone(), id.to_string()))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default()
                })
                .collect();
            if let Some(catalog) = catalog.as_ref() {
                for (provider, model) in catalog.entries() {
                    known.insert((provider.to_string(), model.id.clone()));
                }
            }
            let (kept, warnings) = filter_available_models(&enabled, &known);
            for warning in &warnings {
                eprintln!("Warning: {warning}");
            }
            kept
        } else {
            // ref 的 `/model` 列表来自运行时 registry：`models.json` 里配置的 provider +
            // **已认证 provider 的内置目录模型**。此前只取"当前 provider 在 models.json 里的模型"，
            // 于是 `/login` 登录一个没写进 models.json 的 provider 后，`/model` 里看不到任何模型
            // （真机：登录 openrouter 后只剩 mock-model）。
            let models_json_text =
                std::fs::read_to_string(paths.agent_dir.join("models.json")).ok();
            let authenticated = pi_cli::list_models::authenticated_providers(
                &paths.agent_dir,
                models_json_text.as_deref(),
            );
            let rows = pi_cli::list_models::collect_models(
                models_json_text.as_deref(),
                catalog.as_ref(),
                &authenticated,
            );
            let mut fallback = pi_cli::list_models::to_model_choices(&rows);
            if fallback.is_empty() {
                fallback.push(pi_cli::tui::ModelChoice {
                    provider: provider_name.to_string(),
                    id: resolved.model_id.clone(),
                });
            }
            fallback
        }
    };
    Ok(Some(std::sync::Arc::new(pi_cli::tui::AgentParts {
        resumed_path,
        initial_messages,
        agent_dir: paths.agent_dir.clone(),
        cwd: cwd.to_string_lossy().to_string(),
        session_dir: paths.session_dir.clone(),
        available_models,
        provider_names: {
            let mut names: Vec<String> = models_config.providers.keys().cloned().collect();
            names.sort();
            names
        },
        // `/login` 列表 = `models.json` ∪ 内置目录（ref 遍历运行时 registry）；
        // 与启动时写进 `TuiState` 的那份必须同源，否则 `parts` 存在时会把它覆盖成"只有 models.json"。
        login_providers: pi_cli::runtime::login_provider_ids(&paths.agent_dir),
        scoped_models_configured: settings
            .enabled_models
            .as_ref()
            .is_some_and(|models| !models.is_empty()),
        initial_thinking_level,
        supports_thinking: resolved.reasoning,
        model_state: std::sync::Arc::new(std::sync::Mutex::new(pi_cli::tui::CurrentModelState {
            model_id: resolved.model_id.clone(),
            provider_name: provider_name.to_string(),
            provider: std::sync::Arc::clone(&provider),
            context_window: resolved.context_window,
        })),
        provider,
        tools: pi_cli::print_mode::default_tools(parsed),
        config: pi_core::agent::LoopConfig {
            model: resolved.model_id.clone(),
            api: resolved.api.clone(),
            provider: provider_name.to_string(),
            system: Some(system_text),
            system_sections: sections.iter().cloned().collect(),
            max_completion_tokens: resolved.max_tokens,
            ..pi_core::agent::LoopConfig::default()
        },
        context_window: resolved.context_window,
        system_message,
        writer,
    })))
}

/// `enabledModels` → 可切换模型 scope 的过滤（ref `core/model-resolver.ts:282-382`）。
///
/// 匹配不到 `models.json` 里实际存在的 `(provider, id)` 的项**丢弃**并返回告警文案
/// （调用方以 `Warning: ` 前缀打到 stderr，与 ref 的 `console.warn` 文案一致）。
/// 纯函数，便于单测：不读文件、不写 stderr。
fn filter_available_models(
    enabled: &[pi_cli::tui::ModelChoice],
    known: &std::collections::BTreeSet<(String, String)>,
) -> (Vec<pi_cli::tui::ModelChoice>, Vec<String>) {
    let mut kept: Vec<pi_cli::tui::ModelChoice> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    for choice in enabled {
        if known.contains(&(choice.provider.clone(), choice.id.clone())) {
            kept.push(choice.clone());
        } else {
            warnings.push(format!(
                "No models match pattern \"{}/{}\"",
                choice.provider, choice.id
            ));
        }
    }
    (kept, warnings)
}

fn load_settings(paths: &pi_cli::runtime::Paths) -> Result<pi_cli::settings::Settings, ExitCode> {
    let path = paths.agent_dir.join("settings.json");
    match std::fs::read_to_string(&path) {
        Ok(text) => match pi_cli::settings::parse(&text) {
            Ok(settings) => Ok(settings),
            Err(error) => {
                eprintln!("pi-rs: {} 解析失败：{error}", path.display());
                Err(ExitCode::from(1))
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(pi_cli::settings::Settings::default())
        }
        Err(error) => {
            eprintln!("pi-rs: 读不到 {}：{error}", path.display());
            Err(ExitCode::from(1))
        }
    }
}

/// 未识别或未实现的参数一律拒绝；返回 `Some(退出码)` 表示已拒绝。
fn reject_unsupported(parsed: &Args) -> Option<ExitCode> {
    let mut rejected: Vec<String> = Vec::new();
    for (name, value) in &parsed.unknown_flags {
        rejected.push(match value {
            Some(value) => format!("--{name}={value}"),
            None => format!("--{name}"),
        });
    }
    rejected.extend(parsed.unsupported.iter().cloned());
    if rejected.is_empty() {
        return None;
    }
    eprintln!(
        "pi-rs: 不支持的参数：{}（本项目没有扩展/主题/技能机制，参数不会被静默忽略）",
        rejected.join(", ")
    );
    Some(ExitCode::from(1))
}

fn print_help() {
    println!("pi-rs {}\n", pi_core::VERSION);
    println!("AI coding assistant with read, bash, edit, write tools\n");
    println!("Usage:\n  pi-rs [options] [--] [@files...] [messages...]\n");
    println!("Options:");
    println!(r#"  --provider <name>              Provider name (default: google)"#);
    println!(
        r#"  --model <pattern>              Model pattern or ID (supports "provider/id" and optional ":<thinking>")"#
    );
    println!(r#"  --api-key <key>                API key (defaults to env vars)"#);
    println!(
        r#"  --system-prompt <text>         System prompt (default: coding assistant prompt)"#
    );
    println!(
        r#"  --append-system-prompt <text>  Append text or file contents to the system prompt (can be used multiple times)"#
    );
    println!(r#"  --mode <mode>                  Output mode: text (default), json, or rpc"#);
    println!(r#"  --print, -p                    Non-interactive mode: process prompt and exit"#);
    println!(r#"  --continue, -c                 Continue previous session"#);
    println!(r#"  --resume, -r                   Select a session to resume"#);
    println!(r#"  --session <path|id>            Use specific session file or partial UUID"#);
    println!(
        r#"  --session-id <id>              Use exact project session ID, creating it if missing"#
    );
    println!(r#"  --session-dir <dir>            Directory for session storage and lookup"#);
    println!(r#"  --no-session                   Don't save session (ephemeral)"#);
    println!(r#"  --name, -n <name>              Set session display name"#);
    println!(
        r#"  --models <patterns>            Comma-separated model patterns for Ctrl+P cycling"#
    );
    println!(
        r#"  --no-tools, -nt                Disable all tools by default (built-in and extension)"#
    );
    println!(
        r#"  --no-builtin-tools, -nbt       Disable built-in tools by default but keep extension/custom tools enabled"#
    );
    println!(
        r#"  --tools, -t <tools>            Comma-separated allowlist of tool names to enable"#
    );
    println!(
        r#"  --exclude-tools, -xt <tools>   Comma-separated denylist of tool names to disable"#
    );
    println!(
        r#"  --thinking <level>             Set thinking level: off, minimal, low, medium, high, xhigh, max"#
    );
    println!(
        r#"  --no-context-files, -nc        Disable AGENTS.md and CLAUDE.md discovery and loading"#
    );
    println!(
        r#"  --list-models [search]         List available models (with optional fuzzy search)"#
    );
    println!(
        r#"  --verbose                      Force verbose startup (accepted; this build always shows the full startup banner)"#
    );
    println!(r#"  --tui-mode <mode>              TUI mode: regular (default) or fullscreen"#);
    println!(
        r#"  --offline                      Disable startup network operations (same as PI_OFFLINE=1)"#
    );
    println!(r#"  --help, -h                     Show this help"#);
    println!(r#"  --version, -v                  Show version number"#);
    println!("\nBuilt-in tools:");
    println!("  read                      Read a file");
    println!("  bash                      Run a bash command");
    println!("  edit                      Exact string replacement");
    println!("  write                     Create or overwrite a file");
    println!("  grep / find / ls          Read-only search (opt-in)");
    println!("  powershell                Run PowerShell (Windows only)");
    println!("\nNot implemented in pi-rs (see LIMITATIONS.md):");
    println!(
        r#"  --fork <path|id>               Fork specific session file or partial UUID into a new session"#
    );
    println!(
        r#"  --extension, -e <path>         Load an extension file (can be used multiple times)"#
    );
    println!(
        r#"  --no-extensions, -ne           Disable extension discovery (explicit -e paths still work)"#
    );
    println!(
        r#"  --skill <path>                 Load a skill file or directory (can be used multiple times)"#
    );
    println!(r#"  --no-skills, -ns               Disable skills discovery and loading"#);
    println!(
        r#"  --prompt-template <path>       Load a prompt template file or directory (can be used multiple times)"#
    );
    println!(r#"  --no-prompt-templates, -np     Disable prompt template discovery and loading"#);
    println!(
        r#"  --theme <path>                 Load a theme file or directory (can be used multiple times)"#
    );
    println!(r#"  --use-theme <name[/name]>      Set the initial interactive theme for this run"#);
    println!(r#"  --no-themes                    Disable theme discovery and loading"#);
    println!(r#"  --export <file>                Export session file to HTML and exit"#);
    println!(r#"  --approve, -a                  Trust project-local files for this run"#);
    println!(r#"  --no-approve, -na              Ignore project-local files for this run"#);
    println!(
        r#"  --                             End option parsing; treat remaining arguments as messages/files"#
    );
    println!();
}

#[cfg(test)]
mod enabled_models_tests {
    use super::filter_available_models;
    use pi_cli::tui::ModelChoice;
    use std::collections::BTreeSet;

    fn choice(provider: &str, id: &str) -> ModelChoice {
        ModelChoice {
            provider: provider.to_string(),
            id: id.to_string(),
        }
    }

    fn known(items: &[(&str, &str)]) -> BTreeSet<(String, String)> {
        items
            .iter()
            .map(|(provider, id)| ((*provider).to_string(), (*id).to_string()))
            .collect()
    }

    /// 有效项保持顺序；无效项被丢弃并给出与 ref 同文案的告警。
    #[test]
    fn drops_unmatched_and_warns_with_ref_text() {
        let enabled = vec![
            choice("openrouter", "openrouter/free"),
            choice("command-code", "inclusionai/ling-3.1-flash:free"),
            choice("opencode-go", "deepseek-v4.1-flash"),
        ];
        let known = known(&[
            ("openrouter", "openrouter/free"),
            ("command-code", "deepseek/deepseek-v4.1-flash"),
            ("opencode-go", "deepseek-v4.1-flash"),
        ]);
        let (kept, warnings) = filter_available_models(&enabled, &known);
        assert_eq!(
            kept,
            vec![
                choice("openrouter", "openrouter/free"),
                choice("opencode-go", "deepseek-v4.1-flash"),
            ]
        );
        assert_eq!(
            warnings,
            vec![
                "No models match pattern \"command-code/inclusionai/ling-3.1-flash:free\""
                    .to_string()
            ]
        );
    }

    /// 模型 id 自身含 `/`（`provider/id/with/slashes`）时按完整 id 比对，不能二次切分。
    #[test]
    fn keeps_ids_containing_slashes() {
        let enabled = vec![choice("command-code", "deepseek/deepseek-v4.1-flash")];
        let known = known(&[("command-code", "deepseek/deepseek-v4.1-flash")]);
        let (kept, warnings) = filter_available_models(&enabled, &known);
        assert_eq!(kept, enabled);
        assert!(warnings.is_empty());
    }

    /// 同名 id 在不同 provider 下不能互相匹配（`(provider, id)` 是二元组）。
    #[test]
    fn provider_is_part_of_the_key() {
        let enabled = vec![choice("opencode-go", "deepseek-v4.1-flash")];
        let known = known(&[("command-code", "deepseek-v4.1-flash")]);
        let (kept, warnings) = filter_available_models(&enabled, &known);
        assert!(kept.is_empty());
        assert_eq!(
            warnings,
            vec!["No models match pattern \"opencode-go/deepseek-v4.1-flash\"".to_string()]
        );
    }

    #[test]
    fn empty_input_yields_empty_output() {
        let (kept, warnings) = filter_available_models(&[], &known(&[("a", "b")]));
        assert!(kept.is_empty());
        assert!(warnings.is_empty());
    }
}

#[cfg(test)]
mod catalog_only_tests {
    use super::load_builtin_catalog;
    use pi_ai::config::ModelsConfig;

    /// 只放 `auth.json` 的机器（没有 `models.json`、没有 `models-store.json`）
    /// 也必须拿得到模型目录；否则启动直接报
    /// "models.json 中没有名为 openrouter 的 provider"。
    #[test]
    fn embedded_catalog_available_without_any_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let catalog = load_builtin_catalog(dir.path()).expect("内嵌目录应可用");
        assert!(
            catalog
                .entries()
                .any(|(provider, _)| provider == "openrouter"),
            "内嵌目录应包含 openrouter"
        );
    }

    /// 空 `models.json` + 目录 ⇒ 仍能解析出模型（目录分支）。
    #[test]
    fn empty_models_config_resolves_from_catalog() {
        let dir = tempfile::tempdir().expect("tempdir");
        let catalog = load_builtin_catalog(dir.path()).expect("内嵌目录应可用");
        let (provider, model) = ("openrouter".to_string(), "moonshotai/kimi-k2.6".to_string());
        let resolved = ModelsConfig::default()
            .resolve_with_catalog(&provider, &model, Some(&catalog))
            .expect("空 models.json 也应能解析目录里的模型");
        println!(
            "resolved {}/{} ctx={} api={} base={}",
            resolved.provider_name,
            resolved.model_id,
            resolved.context_window,
            resolved.api,
            resolved.base_url
        );
        assert_eq!(resolved.provider_name, provider);
        assert_eq!(resolved.model_id, model);
    }
}
