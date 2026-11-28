# A-b：空配置目录下的启动画面 / `/login` 面板对拍（P8e-11）

## 方法
`tmux` 100×30，`PI_CODING_AGENT_DIR` 指向空临时目录；TS（`node cli.js`）与
`pi-rs`（release）各跑一遍，逐行对比截图。

## 1. 启动画面（无任何配置）
| 行 | TS | pi-rs | 判定 |
|---|---|---|---|
| 版本行 | ` pi v0.87.1` | 同 | same |
| 键位提示 | ` escape interrupt · ctrl+c/ctrl+d clear/exit · / commands · ! bash · ctrl+o more` | 同 | same |
| 帮助行 | ` Press ctrl+o to show full startup help and loaded resources.` | 同 | same |
| 自我介绍两行 | ` Pi can explain its own features …` | 同 | same |
| `[Context]` 段 | `  AGENTS.md` | 同 | same |
| 警告 | ` Warning: No models available. Use /login to log into a provider via OAuth or API key. See:` + 两行 docs 路径 | 同 | same（文本）；**路径前缀不同**（TS 指向自身安装包的 docs） |
| footer | `… no-model` | 同 | same |
| stderr | 无 | 无（本批已删掉多余的 `pi-rs: …` 行） | same |

## 2. `/login`（无参）
| 项 | TS | pi-rs | 判定 |
|---|---|---|---|
| 标题 | ` Select authentication method:` | 同 | same |
| 第 1 项 | ` → Sign in with an account` | 同 | same |
| 第 2 项 | `   Sign in with an API key` | 同 | same |
| 提示行 | ` ↑↓ navigate  enter select  escape/ctrl+c cancel` | 同 | same |
| 进 API key 层 | 列出该类型下的 provider | 同（来源为 `models.json` 的 provider） | 列表来源不同：零配置时 TS 仍列出内置 registry，我们为空（已登记） |

## 3. `/login <provider>`（api_key 型）
| 步骤 | TS | pi-rs |
|---|---|---|
| 成功提示 | ` Logged in to …` 并刷新模型 | 同（` Logged in to mockp.`，footer 变为 `mock-model`） |
| `auth.json` | `{ "<provider>": { "type": "api_key", "key": … } }`，0600 | 同格式、0600（键序按字母序，语义等价） |
| 登录后立刻对话 | 可以 | 可以（pty 探针：登录 → 发消息 → mock 回 `pong`） |

## 为什么没有 golden
`tools/difftest` 的 TUI 运行器启动 mock provider 时会写 `models.json`（场景的
`provider`/`model`），因此**"零配置、无可用模型"这个初始态无法经 harness 复现**——一录就是
"有模型"的画面。已试两轮（`helperExit=3`，`rawBytes≈80`），判定为 harness 能力缺口而非实现
差异；本条改用 tmux 人工取证（上面三张表）。
