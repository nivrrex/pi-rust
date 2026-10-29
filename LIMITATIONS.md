# 已知限制

本文件记录**与上游（pi 0.87.1）行为存在差异**或**尚未实现**的地方。所有条目都对应上游客
户端代码中的具体位置，便于后续补齐。

## 已知差异（P8 出口后遗留）

样式级基准：真彩 `golden-tui-truecolor/`、256 `golden-tui/`，判定工具
`tools/difftest/compare-style.mjs`（逐单元格 字符/前景/背景/粗体/斜体/暗淡/反显）。
**P8 收口实测**（7 个场景，两侧文本均 same）：真彩合计 **3 格**、256 合计 **5 格**，
且与 TS 的 run-2/run-3 **逐格相同**（详见下面第 1 条与证据文件）。

**样式对拍与回放器测试依赖 `tmux`**：K1' 的第三方基准用 `tmux new-session/capture-pane`
把 raw 字节喂给真实终端状态机（`tools/difftest` 侧不引入新依赖，但**需要系统里有 tmux**）。
没有 tmux 时只能跑两套自研回放器（`compare-tui` / `compare-style`），
无法验证"真实终端终态一致"这一条；相关证据：
`notes/evidence/K1p-replayer-compare.md`、`notes/evidence/K5-style-tables.md`、
`notes/evidence/K4c-markdown-styles.md`、`notes/evidence/K4c-t06-truecolor.md`。

1. **版本行 ` pi v0.87.1` 的 3 / 2 格** —— 属**上游自身不确定性**，有逐次录制对照：
   - `golden-tui/T01`（256）与 `golden-tui-truecolor/T77`：我们的输出与 TS 的 **run-2 / run-3 完全相同**，
     只有 run-1 多 3 格（`r1 c0` 的 `dim`）——即 TS 自己的 run-1 就是离群样本。
   - `golden-tui/T28`（256）的 2 格：TS 256 模式下该行 `pi` **没有粗体**（`\x1b[38;5;109mpi`），
     但同模式的 T77 录到的 `pi` **有** `1;`；真彩下两者都有。取"始终加粗"（与真彩、T77 一致），
     保留这 2 格。
   结论：该行的 `dim/bold` 取决于 TS 哪一帧重画了这一列，不是稳定语义，也不影响其余任何单元格。
   证据：`notes/evidence/K5-style-tables.md`（逐次录制对照表 + `compare-style` 原始输出）。

2. **P8e-5 批内已消除的差异（保留记录，便于回归）**：
   - 回放器假象（`compare-style` 缺隐式滚屏）—— 见 `notes/evidence/K1p-replayer-compare.md`；
   - 工具头单色（T73 4→0）、退出提示多打一行（T28/T35/T77 各 20+ → 0）、
     前导空格误染前景（T75 16→0）、markdown 元素色与列表符号/行内代码（T05 433→0）。
   最后实测：真彩七场景仅 T77 剩 3 格、256 仅 T01 3 + T28 2 格。
3. **首次渲染的帧切分与上游不同**（上游首帧只画启动头部，我们首帧基本画完）——
   已用 tmux 真实终端状态机回放证明**终态一致**（两侧 ` ls .` 都在第 12 行、非空行数一致），
   文本级 189 个场景全 same，登记为"时序差异，终态一致"，不做批量策略改造。
4. **`T06` 无真彩 golden**：上游在 `COLORTERM=truecolor` 的 pty 下不产出工具行（多次复现），
   该场景只有 256 基准。
5. **Windows 真机未验证**（仅交叉编译通过）；真机 IME 崩溃未复现；退出残留 500ms 有界。

## Windows 真机测试（2026-10-29 已完成，见 `notes/windows-real-test-2026-10-29.md`）

`commit 5a31915` 的产物在 Win10 LTSC 19044 上实测有 5 个 P1（按键翻倍、终端应答被当输入、
`Ctrl+P` 因一条无效 `enabledModels` 永久卡死、`Shift+Tab` 映射缺分支、经典 conhost 未开 VT 位）。
均已修复并在真机复测通过；产物需重新构建（`built … commit 5449bf3-dirty`）：

- 按键只接 `KeyEventKind::Press`（crossterm 0.28 对 key-up 也发 `Release`）；
- Windows 不再发 DA1/kitty 查询（没有协商层消费应答），另加 `RespFilter` 抹掉 CSI/OSC/SS3 应答；
- `Ctrl+P` 跳过解析失败的模型（此前一条无效项会让循环永久停在同一错误上）；
- 补 `KeyCode::BackTab` 分支；`TerminalGuard` 启动时置 `ENABLE_VIRTUAL_TERMINAL_PROCESSING`；
- footer 补 `(provider)` 前缀（可用 provider >1 时，放不下则退回）与初始 `0.0%/<window>`（此前写死 128k）。

### 粘贴与输入法（wezterm + Rime/小狼毫，2026-10-29 用户真机反馈）

真机（wezterm + 小狼毫 rime）上报告的三个现象，**未复现、未定位**：

1. `Shift+Insert` 粘贴多行时，**只有偶尔**折叠成 `[paste #N +M lines]`，多数情况整段展开；
   先由输入法出字符再粘贴时折叠概率更高，输入法处于中文态时展开概率更高——表现为与
   输入法时序相关，但不是必然；
2. 粘贴**之后**方向键左右可用、**上下失效**；
3. 粘贴**之后**输入法候选框不跟随光标、光标位置错乱。

判断（**未验证**）：第 2、3 条更像"粘贴把编辑器的光标/重绘状态与输入法位置同步打乱"，
而非粘贴本身判定错误；折叠与否取决于字符到达的时间间隔，与本项目 Windows 侧的
PasteBurst 启发式（30 ms 攒批 / 60 ms 回车挂起）方向一致，但 wezterm 走的是**括号粘贴**
（`ESC[200~`），理论上不该触发那条启发式——需要先确认 wezterm 是否真的发了 `ESC[200~`。
下一步需要：在 wezterm 里抓原始输入字节（`SCRIPT`/`strace`）后再定位。

### Windows 粘贴（2026-10-29 真机报告后修复）

经典 conhost（`cmd.exe` 窗口）的 **Shift+Insert / 右键粘贴不是 bracketed paste**：整段文本被拆成
一串 `KEY_EVENT`（行间是普通回车）逐个送进输入缓冲，而 `Ctrl+V` 在 conhost 里本来就不是粘贴键
（Windows Terminal 才有 Ctrl+V）。因此 Windows 侧改用「突发合并」：

- 字符按 30 ms 窗口攒批（人工打字 ≳ 60 ms/字，不受影响）；
- 回车先挂起 60 ms——期间还有字符进来 ⇒ 判为粘贴里的换行；超时无输入 ⇒ 才当提交；
- 结果：多行粘贴整段进编辑框（>10 行按 ref `editor.ts` 门槛折叠成 `[paste #N +M lines]`），
  **不再触发提交**；再按一次回车才发出。

**这是启发式，不是协议**：合批靠时间窗，以下情形可能失效（已知未复现的边界）：
- 分块粘贴时块间隔 **> 60 ms**（慢速远程会话、ssh 链路上多跳、远程桌面复制大段文本），
  回车之后的字符会被判成新输入 ⇒ 可能提前提交；
- 反过来，人工输入若在回车后 **60 ms 内**继续敲字符，会被误判为粘贴而多等一拍（不丢字）；
- 30 ms 合批窗对**极慢**终端（串口/BBS）意味着一次粘贴会被切成多批——仍不提交，但每批
  各自成块，折叠计数会偏多。

仍未支持：conhost 里 `Ctrl+V` 不自带粘贴（终端层不发送任何按键），要在应用层读剪贴板需引入
剪贴板依赖或 `unsafe` FFI，两者都需先批准；目前用 Shift+Insert / 右键粘贴即可。

已知剩余（非阻塞）：
- GBK 控制台把 `↑`/`↓` 按双宽渲染，footer 尾部会折 2 个字符到下一行（Windows Terminal 预计正常，未实测）；
- Windows 上 `cargo test --workspace` 仍有 12 个平台相关 target 失败（mock-provider 为 Linux ELF、
  `/tmp` 路径、`COLORTERM` 环境等），与代码缺陷无关，清单见上述 notes。

## Windows 输入路径的三处偏差（P8e-9/C-a 登记，含更正）

| # | 偏差 | ref 做法（行号） | 我们的做法 | 影响 |
|---|---|---|---|---|
| 1 | 输入事件来源 | ref 走 **VT 输入字节**路径：`packages/tui/src/terminal.ts:194-198` 在 `setRawMode(true)` 后调 `enableWindowsVTInput()`（`SetConsoleMode` 打开 `ENABLE_VIRTUAL_TERMINAL_INPUT`），再按字节解析 | 用 **crossterm 事件**（`KeyCode` + `KeyModifiers`），不打开 VT 输入 | 键位语义一致（`9a9ab38` 真机实测 Ctrl+D / Ctrl+P / Shift+Tab 均正常），但拿不到 VT 字节流的附带能力（见 2、3） |
| 2 | Kitty 键盘协议 | ref 在 Windows 也**询问** Kitty 协议（`terminal.ts:200-202` `queryAndEnableKittyProtocol()`），失败才退回 `modifyOtherKeys` | 我们**不发** DA1/kitty 查询（无协商层消费应答；修复前应答会污染编辑框，现直接不发） | 无 Kitty 键盘协议 ⇒ 无法区分 `ctrl+shift+字母` 一类组合 |
| 3 | 括号粘贴 | ref 启用 `\x1b[?2004h`（`terminal.ts:184-185`），粘贴被包在 `\x1b[200~…\x1b[201~`，由 `stdin-buffer.ts:25-26/328-366` 拆整段 | 我们仍**发** `?2004h`，但 Windows 的 crossterm 事件路径只给 `Event::Paste`（是否含换行取决于终端） | **风险**：多行粘贴可能被当成多次回车 ⇒ 多次提交。需真人在 Windows Terminal 下验证 |

### 更正：此前"Windows 控制台不转换 Ctrl+D"的断言不成立
P8e-7 回复里我写过"Windows 控制台在 VT 输入模式下不把 `Ctrl+D` 转成 `0x04`"——**该说法无直接证据**，只是当时
"按键无反应"的一种猜测。Windows 侧实测的真因是两件事：①`map_key()` 未过滤 `KeyEventKind::Release`
（crossterm 对 key-up 也发事件）导致字符翻倍；②启动期 DA1 应答 `[?1;0c` 被当成输入写进编辑框，
编辑框非空 ⇒ `Ctrl+D` 不触发退出。两者均已修（`9a9ab38`）。

## 登录（`/login`、`/logout`，P8e-11 登记）

- **已实现**：`/login [provider]` 的 api_key 型流程——provider 列表（`models.json` 里的
  provider）、`/login <provider>` 直达 key 输入、无参时 ↑/↓ 选择 + Enter 确认 + Esc 取消；
  写入 `<agentDir>/auth.json`（原子写、Unix `0600`、保留未知键）；登录成功后选中该
  provider 的默认模型（`settings.defaultModel` 或该 provider 的第一个模型）。
  `/logout` 删除 `auth.json` 里的对应条目。
- **未实现**：OAuth 型登录。选择列表里标为 `oauth` 的 provider（ref 中 9 个：anthropic、
  github-copilot、kimi-coding、meta、openai-codex、openrouter、openrouter-images、radius、
  xai）在选择或指定时给出 `OAuth login for <name> is not supported yet.`，不做任何网络请求。
- **登录后行为**：无模型启动时 agent 部件（`AgentParts`）未构建；登录成功会**就地重建**
  部件（`RebuildParts` 回调），当次会话即可对话（pty 探针实测：登录 → 发消息 → mock 回
  `pong`）。
- **未实现的连带行为**：ref 登录成功后的"凭据来源/模型范围刷新"提示行、`/logout` 的选择器
  （本实现直接删除全部已配置 provider 的条目并给出 `Logged out of …` / `No stored
  credentials to remove. …` 文案）。

## 键位实现缺口（P8e-7 登记）

`ctrl+p`（ref `core/keybindings.ts:106-109` 的 `app.model.cycleForward`）**已实现**：
`crates/pi-cli/src/tui.rs:2176` 起按 `available_models` 顺序切到下一项并重建 provider
（只有一个模型时给状态提示 `Only one model available`）。此前我在本文件里误记为"未实现"，
原因是把**选择器打开时**那条"吞掉按键"的分支（`:2065-2078`）误当成主界面分支——已更正。

仍未实现：
- `app.model.cycleBackward`（Windows 上 `alt+p`、其它平台 `shift+ctrl+p`）；
- 其余 ref 键位中未覆盖的条目，见上一节。

## 与上游的已知差距（P8e-6 登记）

### 未实现的 CLI 子命令与选项（`--help` 里不再列，按 ref 应存在）
ref `cli/args.ts:272-300` 的 `printHelp` 还列了这些，我们没有实现，**静默缺失**过一段时间，现登记：
- 子命令：`install` / `remove` / `uninstall` / `update` / `list` / `config` / `auth`
- 选项：`--resume, -r`、`--session-id`、`--fork`、`--no-thinking`、`--verbose` 等（完整清单见该函数）

### `thinkingLevelMap` 未解析（影响思考档位集合）
ref `ai/src/models.ts:926-933` 的 `getSupportedThinkingLevels` 对**声明了** `thinkingLevelMap` 的模型
按映射表增删 `xhigh`/`max`（`null` 表示不支持）。我们目前**只按 `reasoning` 给固定五档**
（`off/minimal/low/medium/high`），所以：
- 无 `thinkingLevelMap` 的推理模型：**与 ref 一致**（你也实测 `openrouter/free • high` 两侧相同）；
- 声明了 `xhigh`/`max` 的模型：我们会**少给**最高两档；声明某档为 `null` 的模型：我们会**多给**该档。

### 退出提示的换行（F2，已按 ref 修，待 Windows 复测）
ref `interactive-mode.ts:4173` 用 `process.stdout.write("…\n")`：**只发 LF**。
我们此前用 `writeln!`，在 Windows 上会发 `\r\n`，退出后多一个回车/空行。已改为显式 `write!` + `\n`，
Linux 侧 golden 不变；Windows 侧需真机对比原版退出输出确认（真机清单第 4 项）。

### `!command` 配置值的 shell 选择（Windows 上不同）
ref `utils/shell.ts:67-110` `getShellConfig()` 在 Windows 上**不是** `cmd /C`：
1. `shellPath`（settings）若配置且存在 → 用它（bash 形态，`-c` 传命令）；
2. 否则找 `%ProgramFiles%\Git\bin\bash.exe`、`%ProgramFiles(x86)%\Git\bin\bash.exe`；
3. 再找 PATH 上的 `bash.exe`（Cygwin/MSYS2/WSL）；
4. 都没有 → **报错**列出安装建议，不静默降级。

我们的 `run_config_command`（`crates/pi-ai/src/credentials.rs`）在 Windows 上用 `cmd /C`，
**与 ref 不一致**：`cmd` 的引号/转义与 bash 不同，`!` 命令若含 bash 语法（管道、`$(…)`、单引号）
在 Windows 上会行为不同。Linux 下两侧都是 `sh -c`/`bash -c` 语义接近，已验证一致。
**已登记为已知差距**，修法（找 bash → 报错）需在 Windows 上验证后再落。

### Windows 二进制未真机验证
`bcryptprimitives.dll` 是本机 wine 8.0（Debian 包）**没有实现**的 DLL（`find /usr/lib -name 'bcryptprimitives*'` 无命中），
且本机缺 `wine32`；这属于环境限制，不是产物问题（PE32+ 合法、16 项导入）。替代验证见真机清单。

## 键位与交互

上游 `packages/coding-agent/src/core/keybindings.ts` 定义了 43 条 `app.*` 绑定。当前实现覆盖 16 条：

- 已实现（键位）：`app.interrupt`(escape)、`app.clear`(ctrl+c)、`app.exit`(ctrl+d)、
  `app.thinking.cycle`(shift+tab)、`app.thinking.save`(ctrl+s)、`app.model.cycleForward`(ctrl+p)、
  `app.model.cycleBackward`(ctrl+n)、`app.model.select`(ctrl+l)、`app.tools.expand`(ctrl+o)、
  `app.thinking.toggle`(ctrl+t)、`app.suspend`(ctrl+z)、`app.editor.external`(ctrl+x)。
- 已实现（命令路径）：`app.session.tree`(/tree)、`app.session.resume`(/resume)、
  `app.session.fork`(/fork)、`app.message.copy`(/copy)。

以下 27 条**未实现**，均依赖选择器/会话树/模型面板的内部状态机，且上游默认不占用主编辑区键位：

| 绑定 | 上游默认键 | 说明 |
|---|---|---|
| `app.session.toggleNamedFilter` | ctrl+n | `/resume` 内的"只看命名会话"过滤（命令内已有等价过滤，缺键位） |
| `app.message.followUp` | alt+enter | 队列中的后续消息插队 |
| `app.message.dequeue` | alt+backspace | 撤回排队中的消息 |
| `app.clipboard.pasteImage` | alt+v | 粘贴剪贴板图片（需要图片剪贴板能力） |
| `app.session.new` | （空） | 新建会话（`/new` 命令已具备） |
| `app.tree.foldOrUp` | 方向键 | `/tree` 折叠/上移 |
| `app.tree.unfoldOrDown` | 方向键 | `/tree` 展开/下移 |
| `app.tree.editLabel` | shift+l | `/tree` 编辑标签 |
| `app.tree.toggleLabelTimestamp` | shift+t | `/tree` 标签时间戳显示 |
| `app.session.togglePath` | ctrl+p | `/resume` 显示会话路径 |
| `app.session.toggleSort` | ctrl+s | `/resume` 切换排序 |
| `app.session.rename` | ctrl+r | `/resume` 重命名 |
| `app.session.delete` | ctrl+d | `/resume` 删除 |
| `app.session.deleteNoninvasive` | ctrl+backspace | `/resume` 非侵入删除 |
| `app.models.save` | ctrl+s | 模型面板保存启用集 |
| `app.models.enableAll` | ctrl+a | 模型面板全选 |
| `app.models.clearAll` | ctrl+x | 模型面板清空 |
| `app.models.toggleProvider` | ctrl+p | 模型面板按 provider 切换 |
| `app.models.reorderUp` | alt+up | 模型面板上移 |
| `app.models.reorderDown` | alt+down | 模型面板下移 |
| `app.tree.filter.*`（7 条） | ctrl+d/t/u/l/a/o/shift+ctrl+o | `/tree` 的过滤循环与预设 |

## 渲染与样式

- 样式级对拍（`tools/difftest/compare-style.mjs`，回放**整段录制流**取最后屏幕）：
  `T01` 已做到 **0 格差异**；其余场景剩余差异主体是**帧集合不一致**——同一条命令下
  对照侧的录制帧数与本实现不同（例如 6 帧 vs 5 帧），逐帧序号无法对齐，多出/缺少的
  瞬时帧（`Working`、重试倒计时、选择器）会整片计入格数。要严格逐格对齐，需要按
  **内容**配对帧，当前工具做不到。背景色（`48;5;…`）已全部对齐。
- 编辑框边框线的颜色：对照侧在 256 色模式下发出 `38;5;242`，而主题里没有任何颜色
  映射到该索引（`#505050`→239、`#666666`→241、`#808080`→244，按对照侧同一套
  `rgbTo256` 算法逐个算过）。未定性，登记为已知差异。
- 终端色彩模式遵循上游：`COLORTERM` 为 `truecolor`/`24bit` 时输出真彩，否则输出 256 色
  （`packages/coding-agent/src/modes/interactive/theme/theme.ts:202-232`）。

## 平台

- `ctrl+z`（挂起到后台）依赖作业控制：在**孤儿进程组**（如某些自动化 pty 夹具）里
  `SIGTSTP` 会被内核丢弃，此时只能观察到"恢复终端 + 重画"，无法观察到真正的挂起。
- Windows 侧只做交叉编译验证，未在真机运行。

## 工具

- `grep` / `find` 委托外部 `rg` / `fd`，**不自动下载**（上游会先尝试下载）。
  `fd` 的探测顺序为 `PI_FD_BIN` → `fd` → `fdfind`。
- TUI 里工具结果只渲染调用头与（按规则的）正文预览；`read` 在折叠态按上游语义**不显示正文**。
