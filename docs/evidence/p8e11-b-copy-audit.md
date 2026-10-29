# B：用户可见文案审计（P8e-11）

## 1. `--help`（已对齐）
按 ref `cli/args.ts` 的 `printHelp` 重写（提交 `f90b2d9`）：

| 段 | 处理 |
|---|---|
| 头部 | `pi-rs <version>` + `AI coding assistant with read, bash, edit, write tools`（ref 同句） |
| Usage | `pi-rs [options] [--] [@files...] [messages...]`（ref 同形） |
| Options（27 项已实现） | **逐行照搬 ref 英文原文**，含括号里的默认值 |
| Options（14 项未实现） | 单列 `Not implemented in pi-rs (see LIMITATIONS.md)`，不静默缺失 |
| Built-in tools | 新增英文段（ref 在 `tools.md` 描述，help 里没有对等表） |
| 合计 | 41 个选项与 ref 一一对应 |

## 2. 本批已改为 ref 英文原文的字符串
| 位置 | 原（中文/自写） | 现（ref 原文） | ref 出处 |
|---|---|---|---|
| TUI 无模型启动提示 | `没有可用的模型：…` + 自写提示 | `Warning: No models available. Use /login to log into a provider via OAuth or API key. See:` + 两行 docs 路径 | `core/auth-guidance.ts:16-20` |
| print/json/rpc 无模型 | `pi-rs: 没有可用的模型：…` | `No models available. … See:` + docs 两行 | 同上 |
| `/login` 首屏 | 自写面板 | `Select authentication method:` / `Sign in with an account` / `Sign in with an API key` / `↑↓ navigate  enter select  escape/ctrl+c cancel` | `interactive-mode.ts:354-402` |
| `/logout` 无凭据 | 自写 | `No stored credentials to remove. /logout only removes credentials saved by /login; environment variables and models.json are unaffected.` | `interactive-mode.ts:5834` |
| 登录成功 / 进行中 | 自写中文 | `Logged in to <provider>.` / `Logging in to <name> (api_key).` | `:6003-6040` |

## 3. 仍为中文的字符串（115 处字面量，按类归并）
| 类别 | 例子 | 处置 |
|---|---|---|
| TUI 内部诊断（`eprintln!("pi-rs: 写会话失败…")`、模型切换失败、崩溃恢复信息） | `无法切换模型：{error}`、`pi-rs 崩溃：{message}` | **保留**：这些是 pi-rs 独有路径（ref 无对应文案），已在 `LIMITATIONS.md` 标注"错误信息为中文" |
| 选择器辅助文案（`（无消息）`、`（无位置）`） | 面板占位 | 保留（ref 用 `(no messages)` 类英文文案，属界面细节，**未逐条替换**） |
| selectors 的说明文本（`settings_selector.rs` 里的 description） | 与 ref 结构对应但文本是中文 | **未完成**：需逐条与 ref `settings-selector.ts` 对齐，见下 |

## 4. 未完成部分（明确登记）
- `settings_selector.rs`（约 40 条 description）、`model_selector.rs`、`thinking_selector.rs` 的
  面板文案**尚未逐条对照 ref 英文原文**；
- 这些字符串已进入 golden（T04/T68/T77 等），改动会让多个 golden 需要重录，属于独立批次
  的工作量；本批未做，避免与"登录"这条主线混在一起。
