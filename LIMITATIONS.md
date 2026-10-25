# 已知限制

本仓库以 pi **0.87.1** 为行为规格重写。下面是**已知**的差异与未实现项，
每条都标了影响面；没有列的差异属于缺陷，请提 issue。

## 已裁定的偏差
| 项 | 说明 |
|---|---|
| `/bug` 提示行 | 上游在不可重试错误后会追加 `If this looks like a pi bug, /bug sends a report to the developers.`（`interactive-mode.ts:2096-2114`，每会话一次）。本实现**不显示**该行：复刻它需要重构消息组件的间距模型（会改动全部 TUI golden 的布局基础） |

## 未实现的 api
已支持：`openai-completions`、`openai-responses`、`anthropic-messages`。
未支持：`azure-openai-responses`、`openai-codex-responses`、`mistral-conversations`、
`bedrock-converse-stream`、`google-generative-ai`、`google-vertex`、`pi-messages`。
选中未支持 api 的模型时，状态行会显示 `无法切换模型：…`（不会静默失败）。

## 未实现的功能
| 项 | 影响 |
|---|---|
| 扩展机制（pi extensions） | 第三方扩展不加载；`get_commands` 不含 extension 来源 |
| 项目级 skills / prompt 模板 | 只加载用户级；项目目录里的自定义资源不生效 |
| 12 类键位动作 | 见上游 `core/keybindings.ts`；缺的动作按无效键处理 |
| `grep` / `find` 工具 | 未实现（`ls` 已实现，放在可选集里，需 `--tools ls`） |
| Windows `powershell` 工具的真机验证 | 与 Windows 二进制一样，只做了交叉编译验证，未在真机跑过 |
| anthropic 的思考预算映射 | `shift+tab` 的档位只映射到 openai 系的 `reasoning_effort` / `reasoning.effort`，Anthropic 的 `thinking.budget_tokens` 未映射 |
| `/thinking` 选中即生效 | 选择器目前只显示当前模型档位，选中不改变状态（用 `shift+tab` 切换） |

## 视觉/交互细节
| 项 | 说明 |
|---|---|
| TUI 差分测试需要上游 pi 0.87.1 | `tools/difftest/` 的场景比对要 `TS_PI_ENTRY` 指向上游 bundle；`cargo test` 不需要 |
| `golden*/` 是录制基线 | 由上游二进制产生，可用 `tools/difftest/` 重录；不是上游源码。`golden/S*/` 的 JSONL 保留了录制时的安装路径（上游 prompt 的 docs 路径，例如 `<PI_TS_INSTALL>/node_modules/…`），**不含凭据**；`golden-tui/*/meta.json` 的路径字段已占位 |
| 退出残留为 500 ms 有界窗口 | 根除需要 `tcflush`（本项目不引入 libc/nix），窗口内到达的终端应答会被丢弃 |
| 剪贴板 | 桌面环境优先 `wl-copy`/`xclip`/`xsel`，否则 OSC 52；终端需支持 OSC 52 才能粘贴到系统剪贴板 |

## 平台
| 平台 | 状态 |
|---|---|
| Linux x86_64（glibc / musl 静态） | 完整支持；musl 为 static-pie，无 OpenSSL，TLS 根证书内置 |
| Windows x86_64 | **实验性**：能编译运行（TUI/print/json/rpc）。`bash` 工具**与上游一致**地要求系统里有 bash（上游 `utils/shell.ts:74-108`：先找 `%ProgramFiles%\Git\bin\bash.exe`，再在 PATH 上找 `bash.exe`，都没有就报错并给出安装指引）；超时用 `taskkill /T /F` 结束进程树；Ctrl+C 由终端按键处理 |
| Windows 的 `powershell` 工具 | **已实现**（`crates/pi-core/src/tools/powershell.rs`）：名字 `powershell`、描述 `Execute a PowerShell command …`、命令前注入 `try { [Console]::OutputEncoding=… }`、解析 `pwsh.exe` → `powershell.exe`、参数 `-NoProfile -NonInteractive -ExecutionPolicy Bypass -Command`；非 Windows 上执行会返回 `The powershell tool is only available on Windows.`（与上游同文案）。**默认激活集仍是 `read/bash/edit/write`**（同上游），需要用 `--tools powershell` 选中 |
| macOS / 其它 | 未验证 |
