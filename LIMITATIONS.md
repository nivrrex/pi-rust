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

### 扩展（P9-1 起）

- **运行时依赖 `node`**：扩展跑在 Node 子进程里（控制通道是 loopback TCP + 一次性令牌，
  见 `notes/p9-1/protocol.md`）。找不到 `node`（或 `PI_NODE` 指错）时**不加载扩展**，
  打印一条提示，其余功能不受影响；`--list-tools` 只列内置工具。
- **`.ts` 扩展需要 jiti**：探测顺序 `PI_JITI` → 扩展目录的 `node_modules/jiti` →
  已安装 pi 的 `node_modules/jiti`（路径取自 `PI_PACKAGE_DIR`/`TS_PI_ENTRY`）。
  都没有时 `.ts` 扩展跳过并提示，`.mjs`/`.js` 照常加载。jiti 的分发方式（随程序打包 /
  要求用户安装 / 运行时探测）**仍未定**（`notes/questions.md`）。
- **扩展的渲染钩子未实现**：`renderCall`/`renderResult`/`ctx.ui.custom`/`setFooter`/
  组件式 `setWidget` 首版一律降级（工具行用内置渲染，UI 调用报"未实现"）。
- **控制通道的令牌对同用户可见**：令牌经环境变量传给 node 子进程，同用户
  （或 root）可读 `/proc/<pid>/environ` 拿到它；监听地址固定 `127.0.0.1`，且 `accept` 之后
  立即关闭监听（只收一个连接）。这是"不新增依赖 + Windows 可用"下能接受的最小暴露面，
  但不是安全边界：同机同用户的其它进程理论上可以读到令牌并抢先连接（概率低但存在）。
- **事件分发只做了两种**：`session_start`、`agent_end`（其余在后续批次）。
- **仓库约定检查依赖本机身份**：`scripts/check-repo-conventions.sh` 用 `id -un` 与 `hostname -s`
  生成"用户路径/主机名"模式；换一台机器（不同用户名/主机名）时，这条规则对**那台机器**的
  路径不生效（它防的是提交里混入这台机器的路径）。CI 场景应改用固定允许清单。

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

### 扩展的 `notify` / `setStatus`（P9-1-E1）

宿主能收 `host.ui.notify` / `host.ui.setStatus` 帧，但 **UI 渲染通道尚未接**：
TUI 下 notify 只打到 stderr（不在会话区、不套主题样式），setStatus 只记账不显示在 footer。
ref 的对应实现见 `interactive-mode.ts:2541`/`:2543`（`showExtensionNotify` / `setExtensionStatus`）。
接通道需要给宿主 actor 增加"上行事件 channel"，列为 P9-1-E2 的第一项。

## 扩展（P9-1-E1c 更新）

- **发现规则**：实现了项目级 `<cwd>/.pi/extensions` → 全局目录的顺序与去重、目录内直接文件
  （`.ts`/`.js`/`.mjs`）与 `index.ts`→`index.js`→`index.mjs` 入口（ref `loader.ts:657-800`）。
  `package.json` 的 `pi.extensions` 清单与 `-e/--extension` 显式路径**已支持**；
  排序按路径（ref 用目录读取顺序）。
- **UI 帧**：`notify` 三档已按 ref 渲染（info 状态行 / `Warning: ` / `Error: `），
  `setStatus` 在 footer 下单独一行（按 key 排序、清洗、截断）。本批 TS 录制逐帧 `same`
  （`scenarios-tui/T88-ui-notify` 与 `scenarios-tui-light/L-T88-ui-notify`）。
  **未验证**：颜色逐字节、空行计数。
- **工具执行帧**：扩展若用 ref 的 `createBashToolDefinition` 等"由宿主执行"的构造器，
  当前 shim 直接抛"未实现"（`notes/p9-1e1c/s3-matrix.md` 的 `bash` 行）——需要宿主动作帧。
- **Ctrl+O 只覆盖 `renderResult`**：扩展自定义组件（`ui.custom` 面板、工厂式 widget 里的
  自定义组件）不参与展开/折叠，`expanded` 变化不会触发它们的重渲染。
- **`pi.appendEntry` 的会话文件等待**：宿主可能早于 `SessionWriter` 启动，消费者线程最多等 10s
  再写；理想做法是"路径就绪后回放队列"（待查，见 `notes/p9-1e4e/ledger.md`）。
- **Ctrl+O 的展开语义**：`renderResult` 会在切换后按新 `expanded` 重新渲染（`tool-execution.ts:352`），
  但 ref 在展开态还会改启动提示行（`drop files to attach`）与把 `[Extensions]` 段展开成
  `user` + 路径；这两项**未实现**，因此 Ctrl+O 的逐帧对拍尚未通过（`notes/p9-1e4b/s5-design.md`）。
- **渲染钩子的样式**：`theme.fg` 用 ref `theme/dark.json` 生成的色表（56 色，truecolor SGR），
  未知色名按 ref `theme.ts:323-327` **抛错**（宿主回落 `unsupported`）；`bg` 仍直通文本
  （背景色未实现）。`T90` 的逐帧 same 已经包含 SGR，说明前景色与 ref 一致。
- **真实扩展的逐行样式对拍未做**：`ask_user`/`cg_recall`/`web_read` 的 renderCall 已验证
  序列化为 text 帧，且其用到的 **8 个色名 SGR 与 ref 逐项一致**（`notes/p9-1e4b/s6-colors-compare.txt`）；
  但**没有**在真实工具调用下做 TS↔pi-rs 的整帧逐行 SGR 比对（`ask_user` 走 `ui.custom`、
  `cg_recall` 需要网关 id、`web_read` 需联网，三者都缺可复现输入）。
- 渲染钩子：`renderCall` 支持 `Text`/`Spacer`/`Container` 的降级（工具块 contentBox 左边距
  与 ref 对齐；`unsupported` 整块回落到内置渲染）；`renderResult` 的帧宿主侧可用但**未接进
  工具块**；`Box`/`Markdown`/`Table`/`SelectList` 不支持。
- **命令**：`registerCommand` 已接分发（`/name args` → handler，未命中按普通 prompt，
  handler 抛错仍视为已处理，ref `agent-session.ts:1766-1789`）。
- **flag**：支持（两阶段解析）。已注册的 `string`/`boolean` flag 从命令行取值、
  `pi.getFlag` 缺省回落注册默认值；未注册的 flag 报 `Unknown option(s)` 并退出 1，
  缺值报 `Extension flag "--x" requires a value`（ref `agent-session-services.ts:92-125`）。
  **`--help` 不列扩展 flag**（ref 的 help 在扩展加载前打印，同样不列）。

## 扩展事件（P9 E2 第一批）

- `input`：`continue`/`transform`/`handled` 三态都已实现（含端到端断言：改写后的文本进请求体、
  `handled` 不产生 provider 请求）。`images` 字段未处理（还没有图片输入）。
- `before_agent_start`：`message` 注入（`role:"custom"`，插在用户消息之前）与 `systemPrompt`
  覆盖均已生效（print 路径有测试）。TUI/rpc 路径未单独验证。
- `agent_before_settle`：`continue:true` 会再跑一轮（print 路径有测试；上限 10 轮），
  `entries` 以 `role:"custom"` 消息落盘并进入下一轮（不是 ref 的会话条目草稿形态）。
- `ui.select`/`ui.input`/`ui.confirm`：**无 UI 环境**（print/rpc）按 ref 的 `noOpUIContext`
  （`runner.ts:320-325`）返回 `undefined`/`false`；TUI 下宿主侧往返通道与面板都已实现：
  `select` 面板与 TS 逐帧 `same`（`scenarios-tui/T89-ext-select`、`scenarios-tui-light/L-T89-ext-select`），
  `confirm`/`input` 用同一套面板但**未单独录制 golden**。

## 扩展 widget（P9-1-E3b / S4 第一批）

- `ctx.ui.setWidget(key, string[], {placement})` 支持，`undefined` 清除；渲染在编辑器下方
  （`belowEditor`），按 key 升序、逐行输出。单测：`crates/pi-tui/src/lib.rs` 的
  `extension_widget_tests`。
- `ctx.getContextUsage()` 支持（宿主每帧把 `usage{tokens,contextWindow,percent}` 放进 ctx；
  TUI 的用量事件会同步进去）。
- `ctx.sessionManager` 提供只读子集（`getSessionFile()`/`getEntries()`，条目由扩展侧读文件解析）；
  其余方法（分支切换、`appendCustomEntry` 等）未实现。
- 宿主动作帧：`host.action.call` 已实现，`createBashToolDefinition` 的 `bash` 由宿主执行；
  其它动作名一律回 `unknown host action`（不静默成功）。
- `pi.getSessionName()` 恒 `undefined`：pi-rs 没有会话命名（无 `/rename`），不是假值。
- `pi.sendUserMessage(text)` 支持（print 路径注入下一轮）。print 的 `agent_end` 已改为
  **每轮结束、settle 之前**派发（ref `agent-session.ts:1061-1062`），因此扩展在 `agent_end`
  里排的消息能赶上本轮 settle。
- `pi.appendEntry(customType, data)` 支持（写 `{type:"custom", customType, data}` 进会话文件；
  宿主比 writer 早启动时会短暂等待路径就绪）。
- `setWidget` 的**组件工厂**：只支持 `render() -> string[]` 的形态（QR 那种），其它显式报错。
- `ctx.ui.setFooter` 支持 `render() -> string[]` 的工厂（`footerData` 只提供
  `getExtensionStatuses/getAvailableProviderCount/getGitBranch/getCwd/onBranchChange`）。
- `ctx.ui.setTitle` 支持（写 OSC 2）；`setHeader`/`setFooter` 见上。
- **并发语义**：工具调用仍**串行**（actor 一次一个 invoke）；UI 按键与渲染钩子在 invoke 期间
  可并发（`Wire` 直连 + 应答按 id 路由）。连接空闲时按键走 actor 的阻塞读取。
- `request_secret`（pi-secrets）已在隔离 `HOME`/`PI_CODING_AGENT_DIR` 下端到端验证：掩码渲染、
  结果只报名字与长度、临时目录全量扫描明文命中 **0**（`tests/request_secret_masked.rs`）。
  **未验证**：真实用户目录下的密钥存储与跨会话复用。
- **`ctx.ui.custom` 的 overlay**：**已支持**合成（居中、盒宽 80、盒内清空、盒外保留；
  `T95-overlay`/`L-T95-overlay` 逐帧 same）。`overlayOptions` 的 anchor/尺寸未实现。
  **注意**：本机 9 个扩展里**没有任何一个**使用 `overlay: true`（`pi-interact` 与 `pi-secrets`
  都走编辑器位置），因此 overlay 的对拍用的是**合成探针组件**，不是真实扩展。
  旧记录（**已实现**，见下）：ref 的 overlay 分支（`interactive-mode.ts:2858-2900`）要求浮在内容
  之上；pi-rs 现由 `pi-cli` 合成（居中、盒宽 `min(80,cols)`、盒内清空、盒外保留），
  `T95-overlay`/`L-T95-overlay` dark/light 逐帧 same、各 10 次稳定。
  但本机 9 个扩展**无一使用 `overlay: true`**，对拍用的是合成探针组件；`overlayOptions` 的
  anchor/尺寸仍未实现、`onHandle` 仅提示。
- **工具调用仍串行**：actor 一次处理一个 `Cmd::Invoke`；`Wire` 直连只在 `in_flight > 0`
  （有读取循环）时启用，连接空闲时按键回落到 actor 的阻塞读取。若 `in_flight` 在判定后
  立即归零，直连会等满 5s 超时并返回错误（不丢、不死锁，但会失败）——属已知窗口。
  **F1 复核：维持现状**，reader 重构暂停（三条路径均失败于 `ext.custom.input` 应答迟到），
  推翻条件见 `notes/decisions.md`「F1 / E4h 收尾」。
- `ctx.ui.custom`：只支持最小交互集合（编辑器位置或 overlay、`render`/`handleInput`/`done`）；`overlay` 与
  `onHandle` 未实现（显式抛错），`overlayOptions` 的 anchor/尺寸未实现；面板的按键映射目前覆盖
  Char/Enter/Esc/Backspace/Tab/方向键。
- **未实现**：
  宿主动作帧（目前只实现 `bash`）的其余动作、
  `createBashToolDefinition` 需要的宿主动作帧。

- **kitty 键盘协议 flags 偏离 ref（5 而非 7）**：关闭位2（上报事件类型），因为抬起事件是"退出后 shell
  回显 `0;5:3u`"的唯一来源（复现脚本 `tools/difftest/kitty-release.py`）。代价：拿不到 release 事件
  （我们本来就把它们全部丢弃，`tui.ts:1073-1076` 语义）；若将来实现 `wantsKeyRelease` 必须恢复位2。
  偏离的夹具通过生成器映射记录（`gen-term.mjs` 的 `applyKnownDeviation`），不是手改期望值。
  依据与推翻条件见 `notes/decisions.md`「F4/S4 改（定案）」。

- **`!` / `!!` 用户 bash 未实现**（P9-F6 结论）：ref 的语义与行号已登记在
  `notes/decisions.md`「F6/S3」（输入分流 `interactive-mode.ts:3225-3240`、`user_bash` 事件
  `:6725-6745`、落盘 `agent-session.ts:3496-3516`）。连带影响：`user_bash` 事件永不派发，
  `pi-secrets` 对 `!` 命令的脱敏路径不会触发。
  实现需要新增 `bashExecution` 会话消息类型（核心枚举，需保证与 TS jsonl 互操作）等五处改动，
  已列为独立批次。

- **`bashExecution` 会话消息**：类型、serde、上下文投影（`!!` 不进上下文）、token 估算与 compaction
  切点**已与 TS 侧互验**（F7b）：
  ① TS `SessionManager` 生成的样本 → pi-rs 解析 → 再序列化**逐字节一致**（含未知字段与键序）；
  ② pi-rs 写出的会话被 ref 真实 `SessionManager.open` 加载后投影一致；
  ③ 六种情形的投影文本与 ref `convertToLlm` 的 oracle **逐字节一致**；
  ④ `-c` 恢复后的请求体断言**已完成**（`crates/pi-cli/tests/p9_f7c_resume_bash.rs`：普通项以
  `bashExecutionToText` 文本进入 user、`!!` 项不出现、顺序正确，且会话文件原有行逐字节不变）。
  **未完成**：TUI 的 `!`/`!!` 输入与 `BashExecutionComponent` 渲染、`/tree` 的命令摘要文本、
  `user_bash` 事件派发（F8/F9）。因此 `pi-secrets` 对 `!` 命令的脱敏路径仍不会触发。

- **`p9_e3b_append` 偶发（F10 已复现，未修）**：单跑 30/30、4 路并行 12/12 通过；仅在 workspace
  全量并行时观测到 1 次失败，失败形态是**整条 `custom` 条目丢失**（assistant 已落盘 ⇒ 不是
  `pending` 缓冲问题，而是 append 请求没到达 writer）。假设是退出时宿主动作帧未排空。
  详见 `notes/p9-f7c/append-flaky.md`。**F10 追加**：8 路并行压力脚本下 **80/80、240/240 稳定丢**；
  失败时 node 扩展进程报 `ECONNRESET` ⇒ 根因是**主进程先退出、扩展动作帧没处理完**（不是文件截断）。
  两处试修（等落盘标志、落盘时并入已有行）均无效且已回滚。

- **单文件发行包的扩展宿主（已修）**：此前 `runner.mjs` 只从 `<exe 目录>/ext-host/` 或源码树查找，
  只下载单个二进制的用户会看到 `pi-rs: 扩展未加载：找不到 ext-host/runner.mjs`（Armbian 真机实测）。
  现在**整个 `tools/ext-host/`（runner + resolve.mjs + 5 个 shim）**由 `include_str!` 内嵌，
  找不到时落到 `$TMPDIR/pi-rs-ext-host-<pid>/`（只内嵌 runner.mjs 会让 node 报
  `ERR_MODULE_NOT_FOUND .../shim/pi-tui.mjs`，Armbian 实测），
  `PI_EXT_RUNNER` 仍可覆盖。`.ts` 扩展仍需 `PI_JITI` 指向 jiti（与之前一致）。

- **`pi-handoff-gate` 的 todo 持久化载体（F12 核实）**：todo 状态写在 **toolResult 的
  `details.state`**（`pi-handoff-gate/index.ts:346/377/392`，`todo.ts:472-500`），重启/切分支时由
  `session_start`/`session_tree` 从 `ctx.sessionManager.getBranch()` 重建；`appendEntry`
  （`index.ts:405`）**只在 `/todo clear`** 时用。⇒ 早前用"会话里有没有 custom 条目"判断 todo 是否
  落盘是**错误的判据**，todo 一直正常。
- **`pi.appendEntry` 的入口覆盖（F11）**：print / TUI / TUI `-c` / `/resume` 切换 / rpc 的
  建 writer 处均已登记会话路径（`set_session_file`），**TUI 已有 pty 回归测试**
  （`crates/pi-cli/tests/p9_f11_entry_paths.rs::tui_entry_writes_append_entry`）。
  **用途说明**：appendEntry 只对"用 appendEntry 存状态的扩展"有效（如 `pi-handoff-gate` 的
  `/todo clear`）；`todo` 工具本身走 toolResult.details，不受此影响。
  **rpc 入口仍未覆盖**：rpc 的会话文件是懒创建的（首次 `prompt` 才建），而 `session_start` 在启动时
  就派发，此时路径未知 ⇒ 该入口的 `appendEntry` 仍会落空；修法需在 rpc 会话生命周期里提前确定路径
  （对应测试已写好但标 `#[ignore]`）。sink 超时不再静默丢弃，会打印 `customType`/`data` 告警。

- **`/new` 未实现**（F12 复核）：ref 有 `/new`（`interactive-mode.ts:3182`），pi-rs 没有该命令，因此
  "`/new` 之后 appendEntry 落到新会话文件"的用例不适用；`/resume` 切换路径已补登记
  （`tui.rs` 的 writer 切换处），但**尚无 pty 级用例**。

- **扩展加载问题的可见性（F14 已修）**：缺 `PI_JITI`、缺 runner、宿主握手失败现在既写 stderr，
  也进启动屏 `[Extension issues]` 段（ref `interactive-mode.ts:1874-1894`）。此前的 `--list-tools`
  实测在四个提交（`e4d2d297`/`013a993f`/`34c708c2`/`edc4de7a`）上都能列出 `todo`/`report_to_user`，
  无加载回归。

- **`/new`（F16 已实现）**：会话切换按 ref 顺序（`session_before_switch` → 新 writer →
  `session_start{reason:"new"}` → 清屏）。与 ref 的差异：不重建扩展宿主（ref 会重建 runtime），
  见 `notes/decisions.md`「F16」。`/new` 的 TS 对拍 golden 尚未录制（见 PLAN）。

- **`session_shutdown{reason:"reload"}`**：pi-rs 没有扩展热重载入口，因此该 reason 永不派发
  （ref `core/agent-session.ts:3294`）。**推翻条件**：若将来实现 `/reload`，必须同时补该事件。

- **扩展加载错误的分类偏离 ref（F19 起，F23 登记）**：ref 把**所有**加载失败（import 失败、
  factory 抛错、语法错误）都收敛成 `type:"error"` 诊断并在交互模式 `process.exit(1)`
  （`loader.ts:563-578` 产生 `{extension:null,error}`、`:607-623` 收集、`main.ts:787-790` 映射、
  `:896-906` 退出并打印 `EXTENSION_LOAD_FAILURE_HINT`），实测只有语法错误会 fatal、另两类进 TUI；
  pi-rs 一律**容错**并把每条问题显示在启动屏 `[Extension issues]`（ref `interactive-mode.ts:1874-1894`）。
  **推翻条件**：用户要求严格对齐 ref 时改为 fatal。
- **请求体 JSON 键序**：`/skill:` 展开后的 user 内容块，TS 是 `{"type","text"}`，pi-rs 是
  `{"text","type"}`（对象语义相同）。**推翻条件**：若某个 provider 或对拍要求字节一致，再调整
  `Content::Text` 的序列化顺序（会影响所有消息，需全量回归）。

- **`/resume` 的 reason 断言未自动化（F24）**：`p9_f18_reason_events` 只覆盖 startup / new / quit；
  `/resume` 的 `shutdown{resume}` → `start{resume}` 顺序在真实运行里成立（F18 手工验证过），但用 pty
  驱动会话选择器在负载下不稳定（三次修改仍偶发"面板未出现"），已从用例中移除并在此登记。
  **推翻条件**：改为按会话 id 直接选中的确定性驱动后，补回该断言。
- **缺 jiti 时的提示合并（F24，偏离 ref）**：ref 没有"jiti 缺失"这个场景；我们把 stderr 与启动屏
  `[Extension issues]` 各合并成一条汇总（受影响扩展数 + 名单），避免 10 个扩展刷 30 行。

- **两个 pty 生命周期用例改为 `#[ignore]`（F24）**：`p9_f16_new_session`（`/new` 后 appendEntry 落新会话）与
  `p9_f18_reason_events`（startup/new/quit 的 reason 顺序）在**全量 `cargo test --workspace` 并行**时会被
  同机进程挤到超时（实测 300s 不退出），单跑稳定（约 4–6s）。按"同一假设链三次失败换方法"停止调延时，
  改为显式单跑：`cargo test -p pi-cli --test p9_f16_new_session -- --ignored`（`p9_f18_reason_events` 同）。
  **推翻条件**：把 pty 驱动改成按会话 id 直接选中 / 用虚拟终端（vt100 直接驱动状态机）后可恢复为常规用例。

## F28
- `ctx.ui.setToolsExpanded` 只记录状态并回执，**不驱动** pi-tui 的工具行展开（`/expand` 仍是唯一开关）；
  推翻条件：用户要求扩展能程序化展开工具输出。
- `[Extension issues]` 已按宽度折行（ref `interactive-mode.ts:1874-1894` 的 `Text` 语义），
  但 80/120 列的 pty 截图对照未做（只有单测）。
- 底栏 `?/0 (auto)`（真机 ②）：未取证、未修，留待下一批；ref 语义见 `getContextUsage`（`types.ts:294-300`）。

## F30 收尾
- `golden/_rejected/S19-attempt-3/`（TS 拒绝录制记录）在恢复时随 `.gitignore` 的 `golden/_rejected/` 规则丢失，
  已从 `5cfec2e` 用 `git show` 还原并以 `git add -f` 入库（`spec/d-compaction.md` 引用它，缺了会让 `check-spec-cites` 失败）。

## F31-B（`!` / `!!` 用户 bash）
1. ~~**`!` 命令后的 footer 百分比与 TS 有约 1 个百分点残差**~~ **已修复（C1）**：根因是 footer 用了
   `system + history` 全量估算；ref `compaction.ts:218-246` 在有 assistant usage 时用 `usage + 其后消息`。
   改为复用 `estimate_context_tokens` 后，`!echo`/`!!echo`/Esc 取消 三个场景与 TS **逐帧 same**。
   以下为历史记录：：3000 行输出场景下 pi-rs `4.6%/128k`、TS `3.5%`；
   ref 的 bash 估算口径已确认（`compaction.ts:359-362`：`command+output` 字符/4，无 markdown 包装），
   按 2000 行截断推算应为 3.98%（含 system）/3.52%（不含），我们的投影还多出约 800 tokens，成因**未定**。
   推翻条件：定位到我们投影里多计的部分（疑似 tool 声明重复计入）并修正后两者一致。
2. **S2d 取消/截断态在 light / truecolor / 256 三档未逐帧对照**（仅 dark 取消态与 TS 逐行一致）。
   推翻条件：按主题各跑一次对照，差异行数落到与 dark 同级（仅临时路径）。
3. **运行中态（loader）未做定格对照**：loader 每帧变化，无法在不引入固定延迟的前提下稳定取同一帧。
   推翻条件：给 loader 加"固定帧号"测试钩子后可与 TS 同帧对照。
4. **`user_bash` 的 `{operations}` 返回尚未接入执行路径**：ref 把它交给 `executeBash(..., {operations})`
   （`interactive-mode.ts:6794-6800`）用于把文件读写等操作路由到扩展；pi-rs 的 `BashTool` 直接在本地进程执行，
   收到 `{operations}` 时按"无处理器"走本地执行（即扩展的执行实现不生效），流式回调不受影响。
   直接后果：**pi-secrets 对 `!` / `!!` 输出的脱敏不生效**——它的实现方式是
   `pi.on("user_bash", …)` 返回 `{operations:{...local, exec(...)}}`，在 `onData` 里逐块脱敏
   （`Extensions_dev/pi-secrets/src/index.ts:302-319`）；operations 不生效 ⇒ 流式输出与会话 `output` 都是原文。
   我们的事件宿主是**独立 Node 进程**，`operations` 里是函数值，无法跨进程调用 ⇒ 要支持必须新增
   "宿主反向请求执行 bash"的 RPC（协议级扩展，非补丁级）。
   推翻条件：实现该 RPC 并用 pi-secrets 实测：`!echo <敏感串>` 的流式帧与会话 `output` 均为脱敏文本。

5. ~~**`!` / `!!` 场景暂无 TUI golden**~~ **已解决（C3/C2）**：`T89a/T89b/T89c/T89d` 四个场景已用 TS 录制入库，官方比较器全部 same。历史记录：：先落盘再执行 `!` 时 TS 与 pi-rs 仍有 3 类差异未收敛——
   ①bash 块前的空行（ref 直接 `chatContainer.addChild(bashComponent)`，`interactive-mode.ts:6786`，无 Spacer）；
   ②footer 百分比（见第 1 条）；③`tools/difftest/compare-tui.mjs` 未归一化随机临时路径 `/tmp/tui-<场景>-XXXXXX/work`。
   推翻条件：三类差异各自消除后，用 TS 录制 4 个场景并全部 same。
6. **颜色回归没有默认闸门**：`tools/difftest/compare-style.mjs` 已存在（逐单元格比 `fg/bg/bold/italic/dim/inverse`），
   但 `scripts/regress-tui.sh` 之前**没有调用**它。本批加了 `STYLE_CHECK=1` 开关（只增加调用、不改比较器判据）。
   实测在既有 88 个 dark golden 上开启后**全部报差异**，首个差异分类：`ch` 40、`fg` 13、`dim` 11、`bold` 11、
   `ch,fg` 10、`bg` 1 —— 其中 `ch` 类与 `compare-tui` 的"same"结论矛盾（例：`T100-skills` r10c0，
   compare-tui 判 same、compare-style 判 `' '` vs `'─'`），说明**该比较器自身对这些 raw（含 OSC-8 超链接）解析有偏差**，
   不能直接作为闸门。推翻条件：修好 compare-style 的屏幕解析（含 OSC-8/滚动）后，开关打开仍全绿。
   （颜色真实偏差确实存在过：`[Skills]`/`[Extensions]` 标题色，已在 `c82b0bc8` 修。）
7. **Markdown 引用块内的空行**：TS 打 ` │`，pi-rs 打空行 —— pulldown-cmark 对 `>` 单独一行**不产生任何事件**
   （`Parser::new_ext` 事件探针确认），没有源位置就无法还原该行。推翻条件：改用 `into_offset_iter` 拿到源区间后补齐。
8. **配色回归默认不设闸门**：见第 6 条；本批新增 `STYLE_CHECK=1` 开关（`scripts/regress-tui.sh`），默认关闭。
9. **颜色差异分类（F34/C1+C2 重新取证后）**：regress 文本层 **dark 91/0/21**。
   `color-classify`（复用官方 `normalizeScreen`）对 91 个 dark golden：**exact=36**、逐格不同 **55**。
   代表样本 `T01` 的逐格 dump：
   | 侧 | ` pi v0.87.1` 行样式段 |
   | --- | --- |
   | 旧 `T01` golden | `0-0:-/-/d`（首列 dim）、`1-2:i109/-/b/d`（`pi` bold+dim） |
   | 当前环境 TS 重录 ×2 | `0-0:-/-`、`1-2:i109/-/b`（**无 dim**） |
   | `T02` golden | 同"无 dim" |
   ⇒ 旧 `T01` 的 `dim` 是**录制期残留**（该 golden 里 `\x1b[2m` 只出现在退出时的
   `To resume this session:` 行），不是 TS 的真实渲染；pi-rs 已按当前 TS 修正（`e5843ef7`）。
   其余 54 个"逐格不同"场景尚未逐类定性（需按 `color-dump` 逐行核）。

9a. **颜色差异尚未清零（官方归一化后重分类，旧记录）**：新增 `tools/difftest/color-classify.mjs`（复用
   `compare-tui.normalizeScreen`，不改它）。对 90 个 dark golden：**exact=22、逐格样式不同=68**。
   **关键发现**：语料自身不自洽 —— `T01` 的 golden 里 ` pi v0.87.1` 行的首列是 `dim`，`T02` 的 golden
   同位置**没有** `dim`（`r1` 逐格 dump 实测）。同一份 UI 不可能两种样式 ⇒ 这些 golden 是在不同终端能力/
   不同 pi-rs 版本下录的，追求"逐格样式全绿"必须以**重新录制 golden** 为前提（属红线，需授权）。
   9a. **颜色差异尚未清零（初版记录）**：修复"启动屏 ` pi v…` 行样式"后 regress **dark 90/0/20**。
   用 `color-dump.mjs` 对 90 个 dark golden 重新分类：`exact=0`、仅空行数不同 **9**、逐格不同 **81**；
   但其中相当部分是**归一化不彻底**造成的假阳性——手工核 `T01`：去掉随机临时目录与 resume 行的
   session id 后，逐格 dump **只剩 2 行环境差异**，样式与 TS 完全一致。要得到真实分类，分类脚本需要
   与 `compare-tui.normalizeScreen` 用同一套占位规则（当前自写的正则漏项）。推翻条件：分类脚本复用
   官方归一化后仍报逐格差异的场景，才需要按 ref 主题色逐类修。

9b. **颜色差异尚未清零（初版记录，已被上条取代）**：新增 `tools/difftest/color-dump.mjs`（只 import 既有 `renderStyledScreen`，不改 compare-*）
   把每帧导出为「行号│文本│样式段」的稳定文本。对 90 个 dark golden 分类（路径已归一化）：
   `exact=0`、仅空行数不同 `=22`、逐格样式不同 `=68`。已定位的真实类别举例：
   `T01` r1c0 —— 启动屏 ` pi v0.87.1` 的**首列空格**在 TS 是 `dim`、我们不是（`dim` 属 ref `theme.ts` 的
   `getStylePrefix` 对 padding 的处理）。推翻条件：上述类别逐类修完后该脚本对全部 dark/light golden 报 0 差异。
   （`compare-style.mjs` 的 `ch` 类假差异并非比较器错误：它不做路径归一化与空行对齐，`color-dump` 可复现同一现象。）

10. **markdown 直渲染对拍尚未收敛（F34/M1，最新）**：`md-diff` 的差异已全部归为**纯样式**类
    （文本层一致），共 8~9 处，代表项：行尾 padding（TS `Text` 补满宽度）、H1 的分段 SGR、
    引用行前缀与正文分段、代码块语法高亮（TS 走 `theme.highlightCode`，`markdown.ts:523`）。
    本轮尝试把引用行改成"前缀+正文两段 span"，`T89f-md-elements` golden 立刻判 different
    （引用块内的空行位置随之改变）⇒ **已回退**：既有 TUI golden 是判据，直渲染的样式微调
    不能反过来动它。推翻条件：这几类样式在不动 `golden-tui` 的前提下对齐（例如只在
    `md-render` example 侧模拟 TS 的 padding/分段，不改进程内渲染）。

10a-2. **嵌套引用内的空行**（`nested.md` 剩余差异）：TS 在引用块内**空行也画 `│ ` 前缀**
    （`markdown.ts:589-593` 对每行都加 `quoteBorder`），我们目前只在有内容时加前缀。照此修会
    影响 `T89f-md-elements` 的既有 golden（该场景引用块内空行的期望不同）⇒ 暂缓，逐类修复需
    先厘清两处 golden 的差异来源。推翻条件：按 `T89f`/`nested.md` 两侧逐行对齐后同时 same。

10a-1. **表格单元格右边界**：`T89g-md-table-cjk` 中 TS 会把超宽单元格内容**折行**（`renderTable`
    的 `wrapCellText`，`markdown.ts:830-837`）后按列宽补齐，我们的行尾空格少 5 列（内容相同）。
    已修的部分：表格总宽不再超过可用宽度（此前被外层 `wrap_to_width` 折断成两行）、
    HTML/行内 HTML 按纯文本处理（`markdown.ts:612-618`）。剩余是 padding 差异，属样式类。

10a. **引用块与列表的空行/缩进语义（F34/M1 新增语料）**：`tools/difftest/md-corpus/nested.md`
    （嵌套引用 + 引用内列表 + 列表内代码块 + 任务列表）对拍 13 行全差，代表：
    - TS 在引用内**每层**都重画 `│ `，空引用行也画出 ` │`；我们的引用内多/少一行、列表项与引用前缀
      顺序不同（TS 是 ` │ │ - 引用里的列表`，我们拆成了两行）；
    - 任务列表 `- [ ]`：TS 渲染成 `- [ ] …`，我们目前把标记吞进段落。
    这类差异**不能靠改渲染修**（会打回 `T89f` 等既有 golden），已按上条结论搁置。
    推翻条件：先按 ref 重录受影响 golden（需授权）或把这些差异限制在 example 侧。

10b. **markdown 直渲染对拍尚未收敛（初版记录）**：新增 `tools/difftest/md-oracle.mjs`（TS 侧直渲染，
    构造参数照 ref `assistant-message.ts:114`、主题 `theme.ts:1170`，必须 `FORCE_COLOR=3` 否则
    chalk 会关色）、`crates/pi-tui/examples/md-render.rs`（pi-rs 侧同输入输出）与
    `tools/difftest/md-diff.mjs`（逐列归一 SGR 后比较）。
    首轮差异（`tools/difftest/md-corpus/basic.md`，dark/60 列）共 16 行，分类：
    ①**行尾 padding**：TS 每行补满宽度（`Text` 的 paddingX），pi-rs 不补（表现"仅样式"差）；
    ②**H1 下划线**：ref `markdown.ts:472` 的 H1 是 `heading(bold(underline(text)))`，我们缺 underline；
    ③**行内强调**：TS 的 `**粗**`/`*斜*`/`~~删~~`/链接下划线是独立 SGR 段，我们整行只有一个样式；
    ④**代码块高亮**：TS 走 `theme.highlightCode`（`markdown.ts:523`）带语法高亮，我们是单色；
    ⑤**段落与列表的空行**：TS 在段落/列表前后补空行（`markdown.ts:487-497`），我们少/多一处。
    推翻条件：上述五类逐类修完后 `md-diff` 报 0 差异。

11. **代码块语法高亮只覆盖 6 种语言**（F35/G2）：ref 用 `highlight.js`（`utils/syntax-highlight.ts:1-45`
    eager 注册 bash/c/cpp/csharp/dart/go/groovy/java/javascript/kotlin/lua/nix/perl/php/python/ruby/
    rust/scala/swift/typescript）；我们自写分词器只覆盖 **sh/bash、rust、python、js/ts、go**，
    其余语言整行 `mdCodeBlock`（与 ref 的"未注册语言"表现一致，但与"已注册但未实现"不一致）。
    已对拍的语言（`md-corpus` 合成样例）：rust/sh 逐格 0 差异，python/js 各剩 1~2 处（函数参数
    `params` 类与 `literal` 类的映射差异）。推翻条件：为剩余语言补分词器并逐格对拍为 0。

12. **F36 未完成项**：H4（渲染钩子按需取行 + 缓存，取代"注册时渲染一次"）只写了设计未实现；
    H6b（退出尾帧是否重复渲染页脚）未在真机确认。H3/H5 已完成（widget 内边距、`R/W/CH` 段）。
    推翻条件：按 PLAN 逐项完成后移除本条。

13. **F37 未完成**：J2（退出尾帧页脚重复，Windows 现象）、J3（`/todo` 菜单与启动屏 `Model scope` 行）、
    J4（渲染钩子按需渲染，F36 plan 方案 A）、J5（T89h 引用块块树重写）均未做。
    J1 的"`^[[A`"结论：pi-rs 退出序列在三种终端应答配置下与 TS 完全一致、无遗留模式 ⇒ 需从 shell 侧取证
    （`printf '\e[?u' | cat -v`、`stty -a`、Alacritty 里 `cat -v` 后按 ↑ 的原始字节）。
