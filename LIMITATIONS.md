# 已知限制与已知差异

本文件列出 `pi-rs` 与上游 pi **0.87.1** 之间尚未对齐、或尚未验证的部分。
"未验证"是按事实写的：没有在对应环境跑过，就不写"已支持"。

## 未实现的功能

### CLI 子命令与选项

已实现（与上游 `cli/args.ts:80-260` 同名同义）：
`--provider`、`--model`、`--api-key`、`--system-prompt`、`--append-system-prompt`、`--mode`、
`--print/-p`、`--continue/-c`、`--resume/-r`、`--session`、`--session-id`、`--session-dir`、
`--no-session`、`--name/-n`、`--models`、`--no-tools/-nt`、`--no-builtin-tools/-nbt`、
`--tools/-t`、`--exclude-tools/-xt`、`--thinking`、`--no-context-files/-nc`、
`--list-models [search]`、`--tui-mode`、`--offline`、`--verbose`、`--help/-h`、`--version/-v`。

尚未实现（运行时以 `Error: 不支持的参数：…` 明确拒绝并以 1 退出，不会静默忽略）：
子命令 `install`/`remove`/`uninstall`/`update`/`list`/`config`/`auth`；
选项 `--fork`、`--extension/-e`、`--no-extensions/-ne`、`--skill`、`--no-skills/-ns`、
`--prompt-template`、`--no-prompt-templates/-np`、`--theme`、`--use-theme`、`--no-themes`、
`--export`、`--approve/-a`、`--no-approve/-na`。

差异说明：

- `--list-models` 的输出格式与上游逐字一致（6 列、两空格分隔、`200K`/`1.0M` 这类 token 缩写），
  但**模型清单来自本二进制内置的目录快照**，可能与用户本机安装的上游版本目录不同步；
  没有可用凭据时输出与上游相同的 `No models available…` 提示（其中的 docs 路径按本项目的
  回落规则给出）。
- `--verbose` 被接受但没有可见效果：本实现始终显示完整启动信息（没有实现上游的
  `quietStartup` 静默启动），因此没有"强制显示"可做。
- 未识别的 `--foo` 与上游不同：上游把它当作可能的扩展旗标转交扩展系统，本实现没有扩展，
  直接报错退出（避免"参数被静默忽略"）。

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

### 大会话（十万行量级）的重绘耗时

34.5 万行的会话，一次**真正的**整屏重绘是约 35 MB 输出，release 实测
`build 0.14s / diff 0.62s / write 3.5s`（`PI_TUI_TIMING=<path>` 打印这三个数）。
整屏重绘只在首帧、宽度变化、`clearOnShrink`、以及"首个变化行在视口之上"时发生；
上游 `tui-main-screen.ts:277-300` 的 `fullRender` 同样逐行写全部行，这一项两边一致。

P8e-19 之前的"每帧都整屏重绘"是 bug，不是设计代价：条目渲染缓存只存了行文本与整行样式，
**没存工具头的行内分段**，命中缓存后这些行没有 span，差分层把第一处工具调用行判成变化
（大会话里它在视口之上）⇒ 每次按键都整屏重绘。缓存现在连 span 一起存，`Ctrl+P` 在该会话上
从 7 s 降到 0.44 s。

### 只有 `auth.json` 的机器与 `models-store.json`

没有 `models.json`、没有 `settings.json`（只放过 `/login` 写的 `auth.json`）时，
模型来自**编译期内置目录**（`fixtures/models-config/builtin-catalog.json`，随二进制分发），
只列已认证 provider 的模型；默认模型按上游 `defaultModelPerProvider` 表选，表里没有则取第一个。

`models-store.json` 在上游是**动态刷新后的目录缓存**（4 小时 TTL 的 pi.dev 目录、
`refreshModels` 拉到的 provider 模型），由 `publishProviderModels` 写入。pi-rs **不联网刷新、
也从不写这个文件**：若配置目录里已有（例如与 TS 版共用），按模型 id 叠加到内置目录上
（同 id 覆盖、新 id 追加，同 `remote-catalog-provider.ts` 的 `mergeModels`）。

差异：上游还会比较缓存里的 `lastModified` 与内置目录的生成时间，比内置更旧的缓存直接丢弃；
pi-rs 目前一律采用，即一份过期的 `models-store.json` 会遮住更新的内置条目。

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

## 已知差异：无 usage 时的上下文百分比（±0.1 个百分点）

footer 的 `x%/Nk (auto)` 在没有 assistant usage 时是**估算值**（ref `compaction.ts:218-246`
的 `estimateContextTokens`：把系统提示与会话里全部消息的字符数加起来除以 4）。两边的**估算公式一致**，
但"系统提示有多少字符"的取值口径不同：我们按 `sections` 的拼装长度与 serde 的紧凑 JSON 计，
上游按它自己内存里那份 system 对象计，实测同一场景下上游的基数比我们大约 150 token
（128k 窗口下即 1.2% vs 1.1%）。

影响范围：仅**没有真实 usage 的帧**（启动首屏、回放类场景）会差 0.1 个百分点，出现 usage 后
两边都走 `usage` 口径，完全一致。`golden-tui/T10`、`T52-tree-summarize` 记的正是这个差异。

## 输入层与渲染层的不变量

两条硬约束（源码与测试都按它写）：

- **字节偏移必须落在字符边界**：任何按字节切片的地方（渲染 span、截断、着色分段）都先收敛到
  `char_indices` 给出的边界；多字节字符（`·`、`…`、中文、emoji）不得被切半。
- **输入缓冲不得丢字节**：`RespFilter`、`BracketPaste`、`PasteBurst`、`InputState` 在超时/拆包时
  只能"重放已吞字节"或"原样放行"，不允许静默丢弃；被判为杂散的前缀要么能补全后被吞，要么原样交回。

## 平台与工具

- `ctrl+z`（挂起到后台）依赖作业控制：在孤儿进程组（某些自动化 pty 夹具）里
  `SIGTSTP` 会被内核丢弃，此时只能观察到"恢复终端 + 重画"。
- `grep` / `find` 委托外部 `rg` / `fd`，**不自动下载**（上游会先尝试下载）。
  `fd` 的探测顺序：`PI_FD_BIN` → `fd` → `fdfind`。
- TUI 里工具结果只渲染调用头与（按规则的）正文预览；`read` 在折叠态按上游语义不显示正文。
