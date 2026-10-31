# 已知限制与已知差异

本文件列出 `pi-rs` 与上游 pi **0.87.1** 之间尚未对齐、或尚未验证的部分。
"未验证"是按事实写的：没有在对应环境跑过，就不写"已支持"。

## 未实现的功能

### CLI 子命令与选项

以下上游存在的子命令/选项尚未实现（`--help` 里也不会列出，避免给出无法使用的提示）：

- 子命令：`install`、`remove`、`uninstall`、`update`、`list`、`config`、`auth`；
- 选项：`--resume/-r`、`--session-id`、`--fork`、`--no-thinking`、`--verbose` 等。
  会话续写用 `-c`；会话浏览用 TUI 里的 `/resume`、`/tree`、`/fork`。

### 扩展机制

上游的扩展（extension）宿主协议尚未实现，`--rpc` 也还不能承载扩展宿主。

### OAuth 登录

`/login` 目前只支持 **api_key 型** provider。上游标为 OAuth 的 provider
（anthropic、github-copilot、kimi-coding、meta、openai-codex、openrouter、
openrouter-images、radius、xai）在选择或指定时会给出
`OAuth login for <name> is not supported yet.`，不做任何网络请求。

api_key 流程已实现：provider 列表、`/login <provider>` 直达输入、
↑/↓ 选择 + Enter 确认 + Esc 取消；写入 `<配置目录>/auth.json`
（原子写、Unix `0600`、保留未知键）；成功后选中该 provider 的默认模型，
无模型启动的会话会就地重建，当次即可对话。`/logout` 删除对应条目。

### 思考档位映射表

上游对**声明了** `thinkingLevelMap` 的模型按映射表增删 `xhigh`/`max`（`null` 表示不支持）。
当前实现只按 `reasoning` 给固定档位（off/minimal/low/medium/high/xhigh/max），因此：

- 未声明映射表的推理模型：与上游一致；
- 声明了 `xhigh`/`max` 的模型：会少给最高两档；
- 声明某一档为 `null` 的模型：会多给该档。

### 键位

上游 `keybindings.ts` 共 43 条 `app.*` 绑定，当前覆盖 16 条：

- 已实现：`interrupt`(escape)、`clear`(ctrl+c)、`exit`(ctrl+d)、`thinking.cycle`(shift+tab)、
  `thinking.save`(ctrl+s)、`model.cycleForward`(ctrl+p)、`model.cycleBackward`(ctrl+n)、
  `model.select`(ctrl+l)、`tools.expand`(ctrl+o)、`thinking.toggle`(ctrl+t)、`suspend`(ctrl+z)、
  `editor.external`(ctrl+x)，以及命令路径 `session.tree`(/tree)、`session.resume`(/resume)、
  `session.fork`(/fork)、`message.copy`(/copy)。
- 未实现（27 条，主要在选择器/会话树内部）：`session.toggleNamedFilter`、`message.followUp`、
  `message.dequeue`、`clipboard.pasteImage`、`session.new`、`session.togglePath`、
  `session.toggleSort`、`session.rename`、`session.delete`、`session.deleteNoninvasive`、
  `tree.foldOrUp`、`tree.unfoldOrDown`、`tree.editLabel`、`tree.toggleLabelTimestamp`、
  `tree.filter.*`（7 条）、`models.save`、`models.enableAll`、`models.clearAll`、
  `models.toggleProvider`、`models.reorderUp`、`models.reorderDown`。

### `/bug` 提示行

未实现（上游在特定错误下会给一行 `/bug` 提示），这是有意的已知偏差。

## 与上游的行为差异

### 无模型时的提示与文档路径

无可用模型时，转录区显示 `Warning: No models available. Use /login … See:` 加两行文档路径，
footer 显示 `0.0%/0 (auto)`、模型名显示 `unknown`，与上游逐字一致。

但**文档路径不同**：上游用安装包的 `docs` 目录，pi-rs 是单二进制、没有安装包目录，
按 `PI_DOCS_DIR` → 可执行文件旁 `docs/` → 公开仓库地址 依次回落。80 列终端下
上游的长路径会折行、这里的短地址不折行，整屏行数可能因此差几行（纯路径长度所致）。

### `!command` 配置值在 Windows 上的 shell 选择

上游在 Windows 上找 `shellPath` → Git Bash → PATH 上的 `bash.exe`，都没有就报错。
当前实现在 Windows 上直接用 `cmd /C`，引号/转义语义与 bash 不同：含管道、
`$(…)`、单引号的 `!command` 在 Windows 上行为会与上游不同。
Linux 下两侧都是 `sh -c`，行为一致。

### 退出时的换行

上游退出提示只发 LF。这里已改为显式 `write!` + `\n`（不用 `writeln!`），
避免 Windows 上多发一个回车；Linux 侧输出不变。

### 首次渲染的帧切分

上游首帧只画启动头部，这里首帧基本画完。用真实终端状态机回放验证**终态一致**
（光标与屏幕内容相同），属时序差异，不影响使用与输出。

### 编辑框边框色

256 色模式下，上游录到的边框色是 `38;5;242`，而主题里没有任何颜色映射到该索引
（`#505050`→239、`#666666`→241、`#808080`→244）。未定性，登记为已知差异。

### 渲染的帧集合

逐格样式对拍时，除文本外还会比对每一帧。上游与这里的**帧数**可能不同
（例如 6 帧 vs 5 帧），逐帧序号无法对齐，多出/缺少的瞬时帧（`Working`、
重试倒计时、选择器）会整片计入格数。文本与终态是逐字节一致的。
样式对拍的第三方基准依赖系统里有 `tmux`（用真实终端状态机回放）。

已知的具体偏差只有一处：版本行 ` pi v0.87.1` 的 `dim`/`bold`。
上游同一条命令多次录制时该列属性不稳定（run-1 与 run-2/3 不同）；
这里取"始终加粗"，与上游真彩录制及多数 256 色录制一致，因此个别场景剩 2–3 格差异。

### 色彩模式

与上游一致：`COLORTERM` 为 `truecolor`/`24bit` 时输出真彩，否则 256 色。

## Windows

Windows 产物是**交叉编译 + 有限真机验证**，整体标为实验性。

### 输入事件来源

上游在 Windows 上打开 `ENABLE_VIRTUAL_TERMINAL_INPUT`，按 VT 字节解析输入；
这里用终端库的事件（`KeyCode`/`KeyModifiers`），键位语义一致，但不请求 DA1/kitty 协议
（没有协商层消费应答，反而不发查询），因此**无法区分 `ctrl+shift+字母` 一类组合**。

### 粘贴

- 经典 conhost（`cmd.exe` 窗口）**不发括号粘贴标记**，粘贴被拆成一串按键，只能按到达速率推断：
  字符间隔 ≤ 30 ms 视为同一段粘贴，回车先挂起 60 ms（期间还有字符 ⇒ 判为粘贴里的换行）。
  合批只影响编辑框里如何呈现（是否折叠成 `[paste #N +M lines]`），**不会**把内容发出去，
  提交始终要求一次真实回车。
- 由此带来的误判：输入法一次上屏很长的整句（rime/微软拼音在 conhost 里表现为极短间隔的
  字符流）可能被当成粘贴；极慢链路（串口、多跳 ssh、远程桌面）块间隔 > 60 ms 时折叠计数会偏多。
- conhost 里 `Ctrl+V` 本身不是粘贴键（那是 Windows Terminal 的），请用 Shift+Insert 或右键菜单。

### 输入法与光标位置

输入法候选框跟随**终端光标的实际位置**。默认与上游一致隐藏硬件光标，只把它定位到编辑框；
部分终端在光标隐藏时不更新坐标，候选框可能落在错误位置。可设
`settings.json` 的 `"showHardwareCursor": true`（或环境变量 `PI_HARDWARE_CURSOR=1`）
显示硬件光标，候选框即跟随。

在 WezTerm + 小狼毫（rime）上曾报告三个现象，**尚未定位**：

1. `Shift+Insert` 粘贴多行时，只有偶尔折叠，多数情况整段展开（与输入法时序相关）；
2. 粘贴**之后**方向键左右可用、上下失效；
3. 粘贴**之后**输入法候选框不跟随光标。

### 行尾多出 `;;` / `0m`（已定位并修复）

现象：Windows 上光标移动过之后，部分行尾会多出 `;;` 或 `0m`（macOS/Linux 同样可复现，
与平台无关，只是 Windows 上更容易被注意到）。

根因（用 `PI_TUI_OUTPUT_DUMP` 的原始字节定位，可离线复现）：编辑器把"软件光标"以
**行内转义**（`ESC[7m` 字符 `ESC[0m`）加 7 字节光标标记插进该行，而该行的**分段样式偏移
（spans）是在带标记的字符串上算出来的**；写终端前标记被剥掉，偏移却没有相应左移，
于是最后一个 span 的结束位置落进了行尾复位序列 `ESC[0m ESC]8;;BEL` 里面 —— 序列被 span
复位切成 `ESC]8` + `ESC[0m` + `;;BEL`，终端把剩下的 `;;` 当普通文本显示。旧构建的行尾复位
只有 `ESC[0m`，同样切法就显示成 `0m`。因此只有"光标所在行且有分段样式"时才会出现，
看起来是偶发的。

修法：剥标记时把该行标记之后的 span 偏移整体左移标记长度
（`extract_cursor_position_with_index` + `shift_past_marker`），并补了两条单测
（复刻真机字节形状、偏移换算的三个区间）。

早先"控制台按 4096 字节切分写入"的假设与此无关，已被 H1 实验否证；按行写入仍保留，
作为"切点不落在序列内部"的防御性约束。

### 其他

- GBK 控制台把 `↑`/`↓` 按双宽渲染，footer 尾部可能折 2 个字符到下一行；
- 在 Windows 上跑 `cargo test --workspace` 时，有十来个用例因平台假设失败
  （mock-provider 是 Linux ELF、`/tmp` 路径、`COLORTERM` 环境等），
  与代码缺陷无关；等价验证在 Linux 侧完成。
- `ctrl+x` 复制在 Windows 上退化为 OSC 52；上游在该平台用系统 `clip` 命令。

## 平台与工具

- `ctrl+z`（挂起到后台）依赖作业控制：在孤儿进程组（某些自动化 pty 夹具）里
  `SIGTSTP` 会被内核丢弃，此时只能观察到"恢复终端 + 重画"。
- `grep` / `find` 委托外部 `rg` / `fd`，**不自动下载**（上游会先尝试下载）。
  `fd` 的探测顺序：`PI_FD_BIN` → `fd` → `fdfind`。
- TUI 里工具结果只渲染调用头与（按规则的）正文预览；`read` 在折叠态按上游语义不显示正文。
