27b. **扩展事件派发状态**：ref `extensions/types.ts` 的 `ExtensionAPI.on` 重载共
     **30 个**事件名。F60/19 当时记录 10 个未派发，**F64 补齐 9 个**：

     | 事件 | 状态 | 落点 |
     |---|------|------|
     | `session_compact`(`:1373`) | 已派发 | `perform_auto_compaction_cancellable`（三路径共用） |
     | `session_compact_failed`(`:1374`) | 已派发 | 同上（`aborted` 区分取消/真失败） |
     | `session_info_changed`(`:1360`) | 已派发 | `append_session_info` 与 `…_to` 两个出口 |
     | `thinking_level_select`(`:1415`) | 已派发 | `SessionWriter::append_thinking_level_change`（唯一出口，仅变化时） |
     | `model_select`(`:1414`) | 已派发 | TUI `/model` 选中（`set`）与 `ctrl+p`/scoped 循环（`cycle`） |
     | `ui_prompt_start`(`:1404`) | 已派发 | `tui::run` 的 `UiAsk` 收/应答处 + 退出兜底 |
     | `ui_prompt_end`(`:1405`) | 已派发 | 同上 |
     | `context_with_system`(`:1382`) | 已派发 | `transform_context` 第二相（含 5 条早退） |
     | `before_provider_headers`(`:1391`) | 已派发 | `EventSink::before_provider_headers`，原地改 headers |
     | `after_provider_response`(`:357-362`) | 已派发 | `Stream::response_meta` 带出 status/headers，`EventSink::after_provider_response` 在收到响应、消费流之前派发 |

     **顺带修复**：`SharedSink`（TUI 与 rpc 共用）此前没有实现
     `before_provider_request` ⇒ 该事件只在 `--print`/`--json` 下派发过。

     **`after_provider_response` 的实现方式**：`Provider::stream` 仍返回
     `Result<Stream, _>`，但 `Stream` 增加了 `response_meta`
     （`Arc<ResponseMeta>{status, headers}`，与既有 `final_message` 槽位同型），
     两个 HTTP provider 在 `bytes_stream()` **消费 `response` 之前**填它。
     离线重放/测试构造的流是 `None`，此时不派发——避免空载荷。
     **已验证（有自动用例）**：`response_meta_is_absent_on_replayed_streams`；以及真实 HTTP 流——
     `crates/pi-cli/tests/p9_f114_after_provider_response.rs` 跑一轮 mock，断言探针扩展收到的
     `status` 是 200、`headers.content-type` 是 `text/event-stream`，即两者都是**真实值**、不是占位。
     （F111 当时是手工一次性跑出来的，F114 才固化成回归；去掉 `with_response_meta` 后该用例
     立即报"探针日志为空"。）
     **与 ref 一致的一处细节**：错误响应（如 429）**不会**产生该事件——ref 的
     `onResponse` 在 `retryProviderRequest` 之后调用（`openai-completions.ts:369-378`），
     只有拿到响应才触发；pi-rs 的 `stream()` 在拿到响应前就返回 `Err`，同样不填元信息。

     **其它已知偏离**：
     - `thinking_level_select` 的 `previousLevel` 来自 `SessionWriter` 记录的
       "最近一次写入/读到的档位"；续写会话时取自文件里最后一条
       `thinking_level_change`。ref 用的是 `session.thinkingLevel`（内存态），
       两者在"文件被外部改动"时可能不一致——**待验证**。
     - ~~`session_compact` 的 `willRetry` 直接取 `reason == "overflow"`~~ **已修（F112）**：
       ref 的口径是 `willRetry = stopReason !== "stop"`（`agent-session.ts:2663`），
       即"这次溢出恢复之后会不会重跑该回合"取决于**中止原因**，与 `reason` 本身无关。
       `auto_compaction_plan` 早就算对了（`OverflowRetry` ⇒ true、`OverflowNoRetry` ⇒ false，
       有单元测试钉住），但那个 `will_retry` **从未被传给** `emit_session_compact`——
       调用链在中间断了，事件里一直是硬算值。现已接上。
       **未观察到端到端差异，且确认构造不出这样的场景**：S13e 改前改后 `willRetry` 分布
       完全相同（8 false / 4 true），因为它的溢出恢复都走 `OverflowRetry`。
       尝试为 Case 2 专门造场景时查清了为什么造不出来——`overflow_outcome` 的
       `context_overflow` 需要 `usage_matches_projection`（`compaction.rs:1634`），
       即助手消息的 usage 必须与**投影估算的实际 token 数相符**；mock 里写死一个
       24000 的假 usage 会与真实投影（内容很短）不符，因此不会被判成溢出。
       要触发 Case 2 得先有**真实超出窗口的会话**，那是另一个量级的准备。
       所以这条修复只有单元测试（`t4_18_append_compaction` 钉住 Case 1/Case 2 的
       `will_retry` 值）与代码路径作证。
     - `ui_prompt_*` 未复刻 ref 的 `uiPromptDepth` 嵌套计数（同一时刻只有一个询问）。
     - `rpc` 的 `set_model` 只允许设置当前模型（`validate_set_model`），
       `modelsAreEqual` 恒真 ⇒ 那里不派发 `model_select`，也不存在漏派发的真实切换。

# 已知限制与已知差异

本文件列出 `pi-rs` 与上游 pi **0.87.1** 之间尚未对齐、或尚未验证的部分。
"未验证"是按事实写的：没有在对应环境跑过，就不写"已支持"。

> 本文保留了大量历史批次记录，前段中的旧数字和旧结论只代表当时状态。
> 当前公开状态以 `README.md`、下方"CLI 场景的差分现状"和"扩展（P9-1-E1c 更新）"为准。

## 未实现的功能

### CLI 子命令与选项

已实现（与上游 `cli/args.ts:80-260` 同名同义）：
`--provider`、`--model`、`--api-key`、`--system-prompt`、`--append-system-prompt`、`--mode`、
`--print/-p`、`--continue/-c`、`--resume/-r`、`--session`、`--session-id`、`--session-dir`、
`--no-session`、`--name/-n`、`--models`、`--no-tools/-nt`、`--no-builtin-tools/-nbt`、
`--tools/-t`、`--exclude-tools/-xt`、`--thinking`、`--no-context-files/-nc`、
`--list-models [search]`、`--tui-mode`、`--offline`、`--verbose`、`--help/-h`、`--version/-v`。

尚未实现的**子命令**（以 `Error: 不支持的参数：…` 明确拒绝并以 1 退出）：
`install`/`remove`/`uninstall`/`update`/`list`/`config`/`auth`。

**`--help` 列出的选项现已全部可用**（`Args::unsupported` 已无成员，由
`t3_27_args::every_help_flag_is_actually_implemented` 钉住）：包括
`-a`/`-na`（F214）、`--use-theme`（F215）、`--export`（F216）、`--fork`（F218）、
`--theme`/`--no-themes`（F220）、`--session-id`（F225 才真正生效）、
`-nbt`（F226）、`@file`（F227）。

**仍不受支持的只有子命令**（`install`/`remove`/`uninstall`/`update`/`list`/`config`/`auth`），
以及 `--skill`、`--prompt-template` 这两个"名字已识别但需扩展声明"的长形式——
它们走"未识别长选项 → 转交扩展 flag"的通用路径（与上游 `args.ts:237-250` 一致），
扩展没声明就报错退出，不会静默忽略。（`--extension`/`-e` **有独立分支**，是实现的。）

差异说明：

- `--list-models` 的输出格式与上游逐字一致（6 列、两空格分隔、`200K`/`1.0M` 这类 token 缩写），
  但**模型清单来自本二进制内置的目录快照**，可能与用户本机安装的上游版本目录不同步；
  没有可用凭据时输出与上游相同的 `No models available…` 提示（其中的 docs 路径按本项目的
  回落规则给出）。
- 未识别的 `--foo`：上游把它当作可能的扩展 flag 转交扩展系统；本实现同样交给扩展
  （`registerFlag`），扩展未声明该 flag 时报错退出，避免"参数被静默忽略"。

### OAuth 登录

`/login` 目前只支持 **api_key 型** provider。上游标为 OAuth 的 provider
（anthropic、github-copilot、kimi-coding、meta、openai-codex、openrouter、
openrouter-images、radius、xai）在选择或指定时会给出
`OAuth login for <name> is not supported yet.`，不做任何网络请求。

api_key 流程已实现：provider 列表、`/login <provider>` 直达输入、
↑/↓ 选择 + Enter 确认 + Esc 取消；写入 `<配置目录>/auth.json`
（原子写、Unix `0600`、保留未知键）；成功后选中该 provider 的默认模型，
无模型启动的会话会就地重建，当次即可对话。`/logout` 删除对应条目。

### 思考档位映射表（F64 已补齐）

`models.json` 的 `thinkingLevelMap` 已按上游解析并参与档位解析
（`ai/src/models.ts:921-935` 的过滤规则 + `:937-955` 的夹取顺序）：

- 未声明映射表的推理模型 ⇒ `off/minimal/low/medium/high`（`xhigh`/`max` 按上游
  规则**不出现**，因为扩展档只在声明时可用）；
- 声明了 `xhigh`/`max` 的模型 ⇒ 该档可用；
- 把某档声明为 `null` 的模型 ⇒ 该档被排除。

`thinkingLevelMap` 的值类型是 `Option<BTreeMap<String, Option<String>>>`，
显式 `null` 与"键缺失"可区分。

已知限制：内置模型目录（`BuiltInCatalog`）里没有该字段，走内置目录的模型
按"无映射表"处理；上游的内置目录（`scripts/generate-models.ts`）会带这个字段。

### 键位

上游 `keybindings.ts` 共 43 条 `app.*` 绑定。**F61/29 按实际代码重新核对**（此前这
条清单严重过时，把 F16/F17-F20 已实现的项仍列为未实现）：

- 已实现（42 条 = 35 条显式 + `tree.filter.*` 7 条）：`interrupt`、`clear`、`exit`、`suspend`、`thinking.cycle`、
  `thinking.save`、`model.cycleForward`、`model.cycleBackward`、`model.select`、
  `tools.expand`、`thinking.toggle`、`editor.external`、`message.copy`、
  `session.new`（`/new`，F16）、`session.tree`（`/tree`）、`session.resume`（`/resume`）、
  `session.fork`（`/fork`）、`session.toggleNamedFilter`、`session.togglePath`、
  `session.toggleSort`（均在 `/resume` 面板内，`tui.rs:3057-3080`）、
  `models.save`、`models.enableAll`、`models.clearAll`、`models.toggleProvider`、
  `models.reorderUp`、`models.reorderDown`（F61/17-F61/20，`/scoped-models` 面板内）、
  `tree.foldOrUp`、`tree.unfoldOrDown`（`/tree` 面板）、`tree.filter.*` 共 7 条
  （`TreeFilterMode`，`/tree` 面板内）。
- 未实现（1 条）：`session.delete`（面板内 `ctrl+d` 已实现，见 F61/31-33；
  这里指主界面无该绑定，ref 同样只在面板内）。
  （`clipboard.pasteImage` 在 F61/35 实现：`ctrl+v` 图片优先——写临时文件并把路径
  插入光标处，无图片则回退插入剪贴板文本，两者皆无则静默；
  ref `interactive-mode.ts:3047-3068`、`utils/clipboard-image.ts`。）
  （`session.deleteNoninvasive` 在 F61/32-33 完整：`/resume` 面板内
  `ctrl+backspace` 搜索框非空时删一个词、为空时进入删除确认，
  ref `session-selector.ts:599-608`。）
  （`tree.editLabel`/`tree.toggleLabelTimestamp` 已在面板内实现：`shift+l`/`shift+t`。）
  （`session.rename` 在 F61/31 实现：`/resume` 面板内 `ctrl+r` 进入 `Rename Session`，
  字符/Backspace/Enter/Esc 全链路接通，保存写 `session_info` 到**目标**会话文件
  （`session_writer::append_session_info_to`），空名不保存，成功后刷新列表。）
  （`message.followUp`/`message.dequeue` 在 F61/30 实现：`alt+enter` 空闲时等同
  `Enter`、忙碌时排队；`alt+up` 把排队消息取回编辑器并给 ref 的状态行。）
  **否决依据**：用户已装扩展无一依赖这些按键（逐个 grep 为 0），且它们都有等价的
  命令路径（如删除会话走 `/resume` 的 `ctrl+d`，已实现）。

### `/resume` 搜索（F61/33，全文匹配 F61/34 已补齐）

- 搜索文本 = `id + name + allMessagesText + cwd`，与 ref
  `session-selector-search.ts:29-31` 一致。`scan_sessions` 在**已有的逐行解析**里
  顺带累积每条 user/assistant 消息文本（`session-manager.ts:836-876` 的
  `allMessagesText`），因此能搜到会话中间的内容；额外开销只有内存
  （ref 同样如此）。
- **搜索框不在屏幕上**（ref 同样如此）：`searchInput` 不参与渲染——
  `buildBaseLayout`（`session-selector.ts:745-754`）只加 header 与列表，
  T36 golden 首屏里也没有输入行。我第一版给它加了一行可见的 `> `，被
  `resume_tests::lines_match_t36_first_screen` 直接判红（那 9 行是逐字节对齐的）。
- 端到端用例只断言"过滤发生了"，过滤逻辑本身由单元测试覆盖
  （`resume_search_tests` 10 项 + `resume_visible_search_tests` 2 项）：
  逐字符输入产生大量中间帧、退出时边界框重画会把最终帧变成收尾画面，
  从原始 pty 流里切"过滤后的那一帧"不可靠（试过三种切法均被干扰）。

### `/bug` 提示行

未实现（上游在特定错误下会给一行 `/bug` 提示），这是有意的已知偏差。

## 与上游的行为差异

### 主题的 auto 形式与终端背景探测（F89）

`settings.json` 的 `theme` 支持上游的 `"<light>/<dark>"` auto 形式
（ref `theme.ts:580-598` 的 `parseAutoThemeSetting` + `theme-controller.ts:60-72`）：
按终端背景选其一，规则与 ref 一致（`getThemeForRgbColor` 的
相对亮度 ≥ 0.5 判 light、`COLORFGBG` 取**最后**一段的 0–255 索引、
无提示回落 `dark`）。此前 pi-rs 把 `"light/dark"` 当普通主题名，
又被 `BUILTIN.contains` 过滤掉 ⇒ **auto 形式完全没生效**（永远默认 dark）。

**OSC 11 查询已接线（F100）**：启动序列现在会发 `\x1b]11;?\x07`
（ref `tui.ts:1427`），并在输入线程的 `Osc11State` 里登记对应配额 ⇒ 终端的
`]11;rgb:…` 应答被正确消费、不会插进编辑器（已实测：注入应答后编辑框无残留）。
此前只有解析侧、**没有发出侧**，那条查询从未发出。

**应答已用于选主题（F101）**：`theme` 写成 `"<light>/<dark>"` 且**没有**
`COLORFGBG` 提示时，收到 OSC 11 应答后按相对亮度二选一
（`settings::theme_for_rgb`，ref `theme-controller.ts:60-66`）。已实测链路：
注入/真实应答被消费后，`Rgb{r:0,g:0,b:0}` 被判为 dark。

**`theme` 未设置时也会用该颜色（F102）**：OSC 11 应答到手后，
`theme` 未设置 ⇒ 直接按亮度选 `light`/`dark`；auto 形式 ⇒ 在两半之间选。
两者的优先级与 ref 一致：**终端查询优先于 `COLORFGBG`**
（`theme.ts:693-706` 先查、失败才回落 env），实测 pty 回黑背景时
即使 `COLORFGBG=0;15` 也判 dark。

**仍未实现**：ref 在 `confidence === "high"` 时会把探测结果**写回**
`settings.json`；pi-rs 不写回（每次启动重新探测）。
推翻条件：需要"第一次之后就不再探测"时补写回。



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
  都没有时 `.ts` 扩展跳过并提示，`.mjs`/`.js` 照常加载。jiti 的 shim 随程序内嵌
  （`EMBEDDED_EXT_HOST`），用户无需另行安装 jiti。
- **控制通道的令牌对同用户可见**：令牌经环境变量传给 node 子进程，同用户
  （或 root）可读 `/proc/<pid>/environ` 拿到它；监听地址固定 `127.0.0.1`，且 `accept` 之后
  立即关闭监听（只收一个连接）。这是"不新增依赖 + Windows 可用"下能接受的最小暴露面，
  但不是安全边界：同机同用户的其它进程理论上可以读到令牌并抢先连接（概率低但存在）。
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

## `S28` 已全五层 same（键序 + fork 后 parentId 都修了）

`git_tree`/`fork`/`clone` 这几条 rpc 响应的**键序**此前是字母序，与 TS 的
`JSON.stringify` 插入序不同。三处一起改的：

- 响应外壳按 `{type, command, id?, success, data?}` 输出（`rpc-mode.ts:64-73`）；
- `fork` 的 `data` 是 `{"text":…,"cancelled":false}`、`clone` 只有 `{"cancelled":false}`；
- 树节点是 `{entry, children}`，条目保持会话文件里的原始字段顺序
  （新增 `compact_keep_order`：剥结构空白、不碰键序，因为先 `from_str::<Value>`
  就已经按字母序排好了）。

**fork 后 `parentId` 取错**的根因在 `SessionWriter::set_chain`：它只在共享叶子
（`SESSION_CHAIN`）还是 `None` 时才用 `writer.last_id` 初始化。fork 之前共享叶子
早已被旧会话设过，于是 fork 之后补写的条目（`thinking_level_change`）把**旧会话的
叶子**当成了 `parentId`——那个 id 不在新会话里，`build_tree` 于是把它当根，
第二次 `get_tree` 的树结构就与 golden 不同。
修法：fork 里**先** `set_ctx_leaf_id(新路径最后一条)`、再挂 chain。

`S28` 随后 A/B/C/fs/exit **五层 same**，CLI 全量从 80 升到 **81 same / 11 different**。

## F144：`toolcall_end` 与 bash `details` 两个真缺陷已修

两处都不是"环境差异"，是**实现与 ref 不一致**，已修并实测：

1. **`toolcall_end` 缺 `toolCall`**：ref `openai-completions.ts:466-471` 的事件是
   `{type, contentIndex, toolCall, partial}`，pi-rs 只发 `{type, contentIndex}` ⇒
   消费者拿不到 `id`/`name`/`arguments`。已给 `AssistantEvent::ToolCallEnd` 加
   `tool_call` 字段，三处构造点（decoder 两处、responses 一处）补齐。
2. **bash 的 `details` 无条件返回**：ref `bash.ts:320-341` 只在
   `truncation.truncated` 为真时才给 `details`，pi-rs 无条件塞
   `{truncated:false, fullOutputPath:null}` ⇒ `tool_execution_end.result`
   比 golden 多一个 `details` 键。已改为只在截断时才有。

**效果（CLI 全量实测）**：`same` 从 **75 → 80**、`different` 从 **17 → 12**。
`S05`/`S10`/`S11`/`S18`/`X01` 五个场景从 different 变为 **same**。

**剩余 12 个的最终定性**（F146 逐个核过）：
- **9 个**（`S13c S13e S13f S19 S20 S20b S20e S21 S22 S24b` 中除 `S13e` 的 `details` 外）
  的 `estimatedTokensAfter` 差 35 —— **路径长度**，可精确复算（F143），非缺陷。
- **`S28`** —— 纯 **JSON 键序**，结构逐字符相等（F145），非缺陷。
- **`S49`** —— 当时受限 YAML 不支持锚点，现已由 `serde_yaml` 修复；
  `S24b` 的内置 `llama.cpp` 命令也已接入。当前 CLI 全量为 **92/92 same**。
- **`S13e`** —— 除那 35 外还多一处：`compaction_end.result.details`
  golden **省略**、pi-rs 给 `{readFiles: [], modifiedFiles: []}`。
  ref 的 `CompactionResult` 确实含 `details` 字段（`agent-session.ts:2526`），
  但它是**可选**且从上次压缩累积；首次压缩时 ref 给 `undefined`。
  **未修**：属"空对象 vs 省略"的呈现差异，需先查清 ref 累积语义再动。
`g0` 的 16 项对照也全过（它**本来就期望 `toolCall`**——golden 录制时就有，
只是生产代码没实现，测试一直在拼出预期值而没有覆盖到生产侧的缺失）。

## `pi.setActiveTools` 已实现

工具集此前在 `AgentLoop::new` 之后不可变，扩展调用 `pi.setActiveTools` 没有任何效果。
现在：

- `AgentLoop.tools` 改为 `RwLock<Vec<Arc<dyn Tool>>>`，新增 `set_tools`/`tool_names`；
  读路径（请求体、`toolsAdded`、工具分发）都先取快照、不把锁带进 `await`。
- rpc 侧 `RpcShared.tools` 同样入锁，新增 `select_active_tools`（纯函数：
  按入参顺序、只在已知集合里取、未知名字忽略、重复名字保留——与
  ref `agent-session.ts:1283-1288` 一致）。
- 扩展侧：`runner.mjs` 的 `pi.setActiveTools(names)` → `host.tools.setActive`
  → `SessionCmd::SetActiveTools` → `PENDING_ACTIVE_TOOLS`，由 `run_prompt`
  开头取走并应用。
- 回归：`crates/pi-cli/tests/p9_f157_active_tools.rs`（3 组）与
  `crates/pi-core/tests/t_f157_set_tools.rs`（2 组）。

## `pi.setLabel` 已实现

`SessionWriter::append_label_change(target_id, label)` 写一条
`{type:"label", id, parentId, timestamp, targetId, label}`（ref
`session-manager.ts:1439-1460`）；目标条目不存在时按 ref 报
`Entry <id> not found` 且**不写任何条目**。链路：
`runner.mjs` 的 `pi.setLabel` → `host.session.setLabel` → `SessionCmd::SetLabel`。
`label` 为空 ⇒ 写 `label: null`（清除标记）。
回归：`crates/pi-cli/tests/p9_f162_set_label.rs`（3 组）。

## `before_agent_start` 现在三条路径都派发

此前**只有 print 路径**派发 `before_agent_start`，rpc 与 TUI **完全不派发**
——扩展的该钩子在常驻模式下不工作。**rpc 与 TUI 都已补**，语义与 print 一致：
`systemPrompt` 覆盖生效、返回的 `message` 以 `role:"custom"` 插在用户消息之前。
TUI 全量回归（125 场景 × 3 runs）实测 `same=357 different=18`，
与补派发前**完全一致**（仍是那 6 个 footer 用量场景），无画面副作用。

## 按 Esc 中止后的界面反馈（已修）

工具执行期间按 Esc 时，`run_loop` 会**正常返回**（被杀的 bash 给出 `Command aborted`
结果），不走 `Err` 分支；而那条成功路径此前只撤掉 `Working` 行、不写任何文案，
屏幕上看不到反馈 —— 表现成"按 Esc 没反应"（真机反馈）。
现在取消生效时会追加 `Operation aborted`（ref `interactive-mode.ts:3898-3908` 的文案）；
`Err` 分支同样处理。取消本身一直是生效的：实测 bash 收到的 `CancellationToken`
在 Esc 后由 `false` 变 `true`，`sleep 60` 的子进程被杀、`run_loop` 立即返回。

## `/resume` 的 `All` 作用域（已修）

`All` 此前只是放开了过滤条件，`sessions` 里仍然只有**当前分区**扫到的数据，
所以按 Tab 切过去看不到别的项目下的会话。现在首次切到 `All` 会遍历
`<agentDir>/sessions` 下每个项目分区（ref `session-manager.ts:1935-1960` 的 `listAll`），
并入已有行后按最后修改时间倒序。回归：`scan_all_sessions` 与
`toggle_scope_loads_sessions_from_other_partitions` 两条单测。

**扫描根必须按配置区分**（F183）：默认分区是 `<agentDir>/sessions/--<cwd>--`，
`All` 要扫它的**父目录**；而 `--session-dir` / `PI_CODING_AGENT_SESSION_DIR`
指定的目录**本身**就是会话所在处（不再按项目分子目录），取父目录会扫到无关位置。
此前写死 `<agentDir>/sessions`，自定义目录下那个根不存在 ⇒ Tab 后什么都看不到；
一律取 `.parent()` 同样错（会扫到用户主目录）。两种都表现为"按 Tab 看不到全部"。
回归：`settings_selector::f183_tests::all_scope_reads_from_the_configured_root`。

**端到端实测**（真 pty，发布产物 `a03c9ba`）：造两个项目分区 `--tmp-…-work-a--` / `--tmp-…-work-b--`，
在 `work-a` 下启动 `pi-rs`，输入 `/resume`：

- `Current Folder` 下显示 `No sessions in current folder. Press Tab to view all.`
  （该次 cwd 与分区名不一致，属构造问题，不影响结论）；
- 按 Tab 后**两条都列出**：`hello from b`（`~/work-b`）与 `hello from a`（`~/work-a`）
  ⇒ 跨分区确实可见。

`run-tui.mjs` 是红线文件、且 `sessionFiles` 只支持单一会话目录，所以这条**没有 TUI 场景回归**，
只能靠上面的单测 + 这次手工 pty 取证。

## bash 工具 stdout/stderr 合并的顺序竞态（已修）

`F52g-ctrl-o-w80` 在 TUI 回归里偶发判 different，隔离复现率约 **2/12**。
差异不是丢流：用 `difflib` 对齐后可见 `cd: … 没有那个文件或目录` **一个字都不差**，
只是位置从 `seq` 输出**之前**（golden 行 46）跑到**之后**（失败那次行 106）。

**根因**：`crates/pi-core/src/tools/bash.rs` 用一个 `tokio::select!` 在同循环里轮询两路
`read`。两路都就绪时 `select!` 随机取一路，而 stdout 一次可读满 8KB、数据量大，
于是可能连续命中它直到 EOF，stderr 的数据被整段推到后面。

**修法**：拆成两个独立读取任务，每次读到数据立刻抢一个递增序号
（`AtomicU64::fetch_add`），再按序号合并。序号取的正是"读到的先后"，
与 ref `core/tools/bash.ts:122-124` 的 `onData` 到达顺序同义；两任务并发推进，
彼此不会被饿死。

**验收**：修前同一场景 **10/12**、修后 **12/12 全 same**，
`F52a`–`F52f` 六个同族场景全部 same；回归用例
`crates/pi-core/tests/p9_f179_stream_order.rs`。

> 排查记录（避免重走）：此前用四类放大都没复现——隔离 10 次、`cd`+`seq 1 4000` 200 轮、
> stdout 200KB 冲刷 200 轮、stderr 落在 stdout 中途 40 轮。这些都在**单进程低负载**下跑，
> 而真实触发需要 TUI 的竞争环境。另注意 `DEFAULT_MAX_BYTES`（50×1024）会从**尾部**截断输出，
> 早期的 stderr 会随开头被裁掉——那是 ref 的既有语义，**不是本次的顺序问题**。

## TUI 回归里 `T116-toolrow-collapsed` 的偶发 different

并行回归下偶发判 different，隔离连跑 5/5、4 路并发 4/4 全 same。脚本里是
`delay 8000` + `waitStable(1500ms)` 等一整轮 LLM 回合，并行时写终端变慢，
"流本身停顿 > quietMs" 会被判成稳定、拍下中间帧。加进 `SERIAL` 后不再出现。

## `agent_before_settle` 的 `context.llmMessages`（已修）

真机上每轮都会刷一行
`pi-rs: 扩展事件 agent_before_settle 处理器失败：Cannot read properties of undefined (reading 'llmMessages')`。
根因：载荷只有 `{type, outcome}`，而 ref `types.ts:797-819` 要求 `BoundaryState`
带 `context`（`BoundaryContextPreview`：`llmMessages`/`contextMessages`/
`contextEntries`/`pendingMessages`/`canContinue`）。
`pi-handoff-gate` 与 `pi-agent-resilience` 都靠"最后一条 `llmMessages` 是否空 assistant"
决定让出/续跑，**字段缺失时这两个核心扩展的续跑与空回复保护完全不工作**。

修法：`emit_agent_before_settle_with_messages(outcome, messages)` 把当前上下文
序列化进 `context.llmMessages`（`canContinue` = 最后一条不是 assistant），
4 个调用点（print/rpc/TUI 两条）都传真实消息；旧的单参版本保留为薄包装。
回归：`crates/pi-cli/tests/p9_f185_settle_context.rs`（改前报 `missing-context`、
改后通过）。实测换成真实配置副本后该行告警消失。

## `models.json` 的错误在 TUI 里看不见（已修）

两处叠加才导致完全静默：

1. **顺序**：`build_tui_agent_parts` 里 `load_models_config` 的检查原本排在
   `resolve_startup_model` **之后**。坏文件必然解析不出模型，于是提前 `return Ok(None)`
   进"无模型"分支，那句 `Failed to parse models.json` **永远走不到**；
2. **呈现**：即便走到，它也只 `eprintln!`——TUI 是 inline 渲染，stderr 会被首屏覆盖。

实测：`models.json` 写成 `{ broken json`，TUI 与 `-p` 都只报
`No models available`，**真正原因一个字都没有**。

修法：把检查提到解析模型**之前**，并走一条**专用**队列
（`models_diagnostics`，渲染在帧顶部 `startup_warnings`）。
**不能复用 `settings::pending_diagnostics()`**：那个队列的诊断要在**会话区**
（F63 的 `tui_shows_settings_diagnostics_in_the_transcript` 钉住了位置），
混用会把 settings 诊断顶到首行——这个坑实际踩过一次，两个用例同时红。
现在坏文件的输出是：

```
Warning: Failed to parse models.json: key must be a string at line 1 column 3
File: /path/to/models.json
```

## 扩展事件载荷与 ref 的字段对照（F186 全量核对）

按 ref `types.ts` 的接口定义逐项对过 pi-rs 派发的 21 个事件。**已修的真缺陷**是
`agent_before_settle` 缺 `context`（见上）。其余**已知且未补**的两处，都因为
已装的 11 个扩展里没有使用方：

- **`session_before_tree.preparation`**：ref 的 `TreePreparation`（`types.ts:645-656`）
  有 `targetId`/`oldLeafId`/`commonAncestorId`/`entriesToSummarize`/
  `userWantsSummary`/`customInstructions`/`replaceInstructions`/label 等；
  pi-rs 只给 `targetId` + `fromId`（后者 ref 没有）。`signal` 也没给
  （`AbortSignal` 是运行时对象、JSON 无法承载；ref 里只用于取消）。
  **没有扩展订阅 `session_before_tree`**，故未补。
- **`input.images` / `input.streamingBehavior`**：pi-rs 只给 `text` + `source`。
  `pi-secrets`（唯一使用者）只读这两个字段，行为正确；`source` 取值
  （`interactive`/`rpc`）与 ref 的 `InputSource` 枚举一致。
  `images` 恒缺是**实现差异而非遗漏**：pi-rs 的 `ctrl+v` 图片粘贴是把图片**写到
  临时文件、把路径当文本插进编辑器**（`tui.rs:4640` 附近），并不构造 `ImageContent`
  对象，所以这里没有图片可传。

其余事件核对结论（字段齐）：`before_agent_start`（`systemPromptOptions` 已由 F167 补）、
`after_provider_response`（`status`/`headers`）、`before_provider_headers`、
`context`/`context_with_system`（`messages`）、`model_select`（`model`/
`previousModel`/`source`）、`project_trust`（`cwd`）、`session_compact`
（`compactionEntry`/`fromExtension`/`reason`/`willRetry`）、
`session_compact_failed`（`errorMessage`/`aborted`/`willRetry`）、
`session_info_changed`（`name`）、`session_shutdown`/`session_before_switch`、
`session_start`（`previousSessionFile`）、`thinking_level_select`、
`tool_call`/`tool_result`、`user_bash`、`ui_prompt_start`/`_end`
（`reason`/`kind`/`title`）、`agent_end`（`messages`）。

## F198–F204 批末 L3 实测（全会话解析改动之后）

| 项 | 结果 |
|---|---|
| CLI 全量（`regress-cli.sh`） | **81 same / 11 different / 0 runfail**（92 个可跑场景）——与基线逐项一致，差异仍是已定性的那 11 个 |
| TUI dark 全量（`regress-tui.sh`） | **357 same / 18 different / 69 nogolden / 0 runfail**——差异仍是 `T05 T08 T09 T18 T45-tree-checkout T106-md-list-hang` 那 6 个 footer 用量场景 |
| `precommit` | **19/19 通过**（含 workspace 全量、lint-banned、hash-order、残留进程检查） |
| 会话互通三方向 | 全一致（见上节） |

本批改动集中在**会话读/写/解析**（BOM 剥除、`build_tree` 迭代重写、`seen` 换 HashSet），
CLI 的 `S13*`/`S19`–`S24b`/`S28`/`S29` 与 TUI 的 `/tree`、`/resume`、`/fork` 系列正好覆盖这条路径，
两侧都无新增差异 ⇒ 改动没有破坏既有互通行。

## 会话互通的回归检查（改会话解析后必跑）

`scripts/session-interop-check.sh` 验三个方向，是"与上游双向互通"这条承诺的直接证据：

1. **TS 读 pi-rs 夹具** —— `tree=5 context=2` 一致；
2. **反向：TS 生成会话 → pi-rs 读** —— `tree=10 context=7` 一致；
3. **pi-rs 写会话 → TS 读** —— `tree=5 context=3` 一致。

用法（`TS_PI_PACKAGE_DIR` 必须给，否则直接报错退 2）：

```sh
TS_PI_PACKAGE_DIR=<pnpm 的 pi-coding-agent 包目录> \
TS_PI_ENTRY=<同目录>/dist/bundle/cli.js \
PI_JITI=<jiti 包目录> sh scripts/session-interop-check.sh
```

**凡改动会话的读/写/解析路径（F201 的树构造、F203 的 BOM 剥除等）之后都要重跑**，
它是这类改动最直接的对拍。F203 之后重跑仍是三个方向全一致。

## "No models available" 提示里的 docs 路径（已修）

无可用模型时，提示第二/三行本该指向**真实存在**的文档，但两条路径都有问题：

1. **`--list-models`**：写成 `{}/docs/providers.md` 拼 `docs_dir()` 的返回值，
   而 `docs_dir()` 返回的**已经是 docs 目录本身** ⇒ 多拼一层；
2. **`-p` / `json` / `rpc`**（`print_no_model_error`）：直接用
   `agent_dir.join("docs")`，**绕过了 `docs_dir()`** ⇒ 拿不到 `PI_DOCS_DIR`、
   可执行文件旁的 `docs/`、以及最后的 GitHub URL 回退。

两种都打印出 `<agentDir>/docs/providers.md` 这类**并不存在**的路径（实测该目录不存在），
用户照着找只会扑空。

修法：两处都改成 `docs_dir(agent_dir)`，再用 `.join(文件名)`。
现在无本地文档时给出可访问的
`https://github.com/nivrrex/pi-rust/tree/main/docs/providers.md`；
设了 `PI_DOCS_DIR` 或可执行文件旁有 `docs/` 时优先用本地路径（三种情形都实测过）。

## Esc 中断后的孤儿 tool call（已实现，与 ref 一致）

工具执行中按 Esc 会在会话里留下 **assistant 的 `toolCall` 而没有对应结果**。
下次 `-c` 时若把它原样发出去，provider 会拒（`tool_call` 没有匹配结果）。

实测（真 mock 抓请求体）：pi-rs **自动合成**一条
`{"content":"No result provided","role":"tool","tool_call_id":"call_x"}`，
请求因此合法。**这不是"丢弃"而是"补齐"**——丢弃会让那条 assistant 的工具调用也失效。

**与 ref 同文案**：`"No result provided"` 出自 ref `transform-messages.ts:175`。
另有一条同类规则也已实现：`stopReason` 为 `error`/`aborted` 的 assistant 整条丢弃
（回放半截 reasoning 或未闭合工具调用会让 provider 报错）。

`crates/pi-ai/src/context.rs` 顶部原写"尚未实现：孤儿 tool call 的丢弃规则"，
**已过时**（实现在 `message::transform_messages`），已订正为实际状态。

## F215–F218 批末 L3 + 两处既有测试同步

| 项 | 结果 |
|---|---|
| TUI dark 全量 | **357 same / 18 different / 69 nogolden / 0 runfail**（与基线一致） |
| CLI 全量 | **81 same / 11 different / 0 runfail**（名单未变） |
| precommit | **19/19 通过** |

**两处既有测试需要同步**（它们断言的是"这些 flag 仍未实现"，而现在已经实现）：

1. `t3_27_args::no_context_files_is_accepted_while_other_unimplemented_flags_stay_rejected`
   原本拿 `--export` 当"仍未实现"的样本，先后换成 `--theme`、最后改成钉住
   "`unsupported` 为空"这个不变量（`--theme` 也在 F220 实现了）；
2. `t3_31_settings::nothing_configured_says_so_instead_of_guessing_a_model`
   原本要求错误文案含"未实现"——**那是过时的**（回落已实现，见 F209）。
   改为要求文案**可操作**（含 `--provider` 与 `defaultProvider`），并显式断言
   **不得**再说"未实现"。

**`T74-retry-success` 也进了 `SERIAL`**：它与 `T116`/`F52g` 同形
（`delay 12000` + `waitStable(1500)` 等整轮重试回合），并行全量里实测 1/3 失败，
差异只是**少一个空行**（23 vs 22 行，状态行清除与快照的先后）；隔离连跑 6/6 全 same。
串行后全量回到 18。

## `/settings` 的主题菜单只看得到内置两套（已修）

加载了自定义主题不等于用户能用上它：`/settings` → Theme 子菜单里硬编码
`vec!["dark", "light"]`，`--theme <dir>` 载入的主题**在界面上根本不出现**，
用户只能靠 `--use-theme <名字>` 手输。

修法：新增 `theme::available_names()`（内置在前 + 已注册自定义、按名字排序），
两处构造点都改用它。实测菜单列出 `dark` / `light` / `solar`，
当前主题带 `✓`。

回归：`crates/pi-tui/tests/t_f224_theme_names.rs`。

## 自定义主题缺色键时的行为（已核，安全）

自定义主题只写几个颜色是常态，所以"缺键怎么办"必须确定。`Palette::of` 是**两级回落**：
自定义主题 → 内置 `dark` → 硬编码默认值。实测：

- 只写 `accent` 的主题：`accent` 用自定义值，`text`/`border` 等从内置 dark 补上；
- **完全空的主题（`colors: {}`）也不崩**，整体回落到内置。

回归：`crates/pi-tui/tests/t_f220_custom_theme.rs` 的
`sparse_and_empty_themes_fall_back_to_builtin`。

## 工具名被后来的空分片擦掉，会话被 `name: ""` 毒化（F243，已修）

**现象**（真机反馈）：

1. 执行工具时报 `Tool  not found`（名字为空，双空格）；
2. 之后**每一轮**请求都被拒：`400 [input_invalid] tool_calls.function.name is required`；
3. 该会话此后既不能用，也看不出发生了什么。

**成因**：SSE 里工具名可能分片到达，某些网关还会在后续 chunk 补发
`"function": {"name": ""}`。pi-rs 的解码器是**无条件覆盖**：

```rust
if let Some(name) = name { *slot_name = name; }   // 空串会把已有名字擦掉
```

而 ref 的规则是**首值优先**（`ai/src/api/openai-completions.ts:539-541`）：

```ts
if (!block.name && name) block.name = name;
```

于是好名字被擦成 `""` → 工具查不到 → 把 `"name": ""` 连同"未找到"结果写进会话 →
之后每次请求都被服务端拒，且这条坏消息一直留在历史里。

**已改**：
1. 解码器按 ref 改成**首值优先**（空串不覆盖、id 同样只认第一个）；
2. 序列化请求时**剔除空名工具调用及其 tool 结果**——这一条是**对 ref 的有意偏离**：
   ref 会原样发出去，然后同样被服务端拒绝（`429`/`400`），用户只能弃掉会话。

**偏离的推翻条件**：如果将来所有目标 provider 都接受空名工具调用，或上游改成在
解码阶段就丢弃这类块，则第 2 条可以撤掉。当前证据：`opencode` 网关直接 400。

**验证**：三条用例分别钉住"空分片不覆盖""后到的名字要补上""坏历史被剔除且正常调用保留"；
再用真实 SSE 场景（只给 arguments、不给 name）跑一遍，第二次请求里已无 `tool_calls`。

## `tool_result` 处理器只要"只观察不返回"就崩（F242，已修）

**现象**（真机反馈，Windows 版同样）：调用任一工具后报

```
pi-rs: 扩展事件 tool_result 处理器失败：Cannot read properties of undefined (reading 'content')
```

该扩展的 `tool_result` 钩子整体失效。本机复现的触发者是 `pi-secrets`：
未配置任何密钥时它的处理器第一行就 `if (redactor.isEmpty()) return;`，
返回 `undefined`。

**成因**：`tools/ext-host/runner.mjs` 在 `tool_result` 分支里直接读
`result.content` / `result.details` / `result.isError` / `result.usage`，
**没有先判断 `result` 是否为空**。而 ref 的语义恰好是"没有返回值 = 不改写"：

```ts
// ref `core/extensions/runner.ts:1090-1091`
const handlerResult = (await handler(currentEvent, ctx)) as ToolResultEventResult | undefined;
if (!handlerResult) continue;
```

**已改**：补上 `if (!result) continue;`（并写注释说明 ref 出处）。同文件其余事件分支
早已用 `result?.` / `result &&` 保护，只有这一处漏了。

**验证**：回归用例装两个扩展 —— 一个只观察不返回、一个把内容改写成标记串；
改前报错且改写不生效，改后无报错且改写进入下一次请求体（用 mock 的请求日志断言）。
注意断言点：改写结果**不在 stdout**，只看打印输出会误判"没生效"。

## `opencode` 系缺 `x-opencode-session`，请求被服务端拒绝（F239，已修）

**现象**：`openai-completions` 下选 `opencode-go` 的模型，每个请求都被拒：

```
Error: 400 Request is missing x-opencode-session and cannot be routed efficiently.
```

**成因**：ref 给 `opencode` / `opencode-go` 的**所有** adapter 套了一层
`withOpenCodeSessionHeader`（`ai/src/providers/opencode-headers.ts:11-31`，
接在 `opencode-go.ts:16-18`），把 `StreamOptions.sessionId` 写成
`x-opencode-session` 头；pi-rs 完全没这条路径。

**已改**：
- `pi_ai::provider::opencode_session_header()`：照抄 ref 三条语义 ——
  只对这两个 provider 生效、没有 session id 就不加、**已有同名头不覆盖**
  （大小写不敏感）；
- `LoopConfig::session_id` 是**取 id 的闭包**而不是字符串：会话惰性创建，
  构造配置时未必已有 id（`--no-session` 更是一个都没有）。请求构造时调用它，
  非 TUI 路径用 `ext_tools::provider_session_id()`（真实会话 id，没有则进程级 uuid ——
  ref 的 `sessionId` 同样始终存在）。

**验证**：单测覆盖四种情形（两个 provider 要头、无 id 不加、配置优先、其它 provider 不加）；
端到端用本地抓包服务器确认 `opencode-go` 请求带 `x-opencode-session: <uuid>`、
`command-code` 不带。

## 扩展帧上限 1 MiB 让 `context`/`agent_before_settle` 全部失效（F239，已修）

**现象**：

```
pi-rs: 扩展事件 context 超时/失败（protocol error: frame too large: 2044527 bytes (limit 1048576)），已按无处理器处理
pi-rs: 扩展事件 agent_before_settle 超时/失败（protocol error: timeout waiting for `ext.event`），已按无处理器处理
```

**成因**：`context` / `agent_before_settle` 会把整份上下文（全部消息与工具结果）
交给扩展，长会话下单帧轻松超过 1 MiB，于是每次都被判成"无处理器"，两个扩展功能
（`context-gateway`、`pi-agent-resilience`）**整体失效**；后续调用还会因为连接状态
被打乱而报 `timeout waiting for ext.event`。

**已改**：`MAX_FRAME_BYTES` 从 1 MiB 提到 **64 MiB**，并在注释里写明依据 ——
ref 的扩展与主进程**同进程**、没有帧上限；pi-rs 跨进程（loopback TCP + 一行 JSON）
必须留界防止扩展把整块内存塞进一帧。该值同时作为 `limits.maxFrameBytes` 通告给扩展。

**验证**：造一个含 2 MB 消息的会话 —— 改前 `frame too large: 2097373 bytes (limit 1048576)`，
改后 0 次、扩展正常收到（handler 记录 `bytes=2097321`）。

**已知边界**：单帧仍受 64 MiB 约束；超过它的上下文（约 64 MB 纯文本，远超任何模型的
上下文窗口）会被判为协议错误而非静默截断。

## 运行期告警曾直接写 stderr，落在输入框里（F238，已修）

**现象**（真机反馈）：会话进行中偶发出现"错误信息显示在用户输入框里"。

**成因**：TUI 是 inline 渲染，stderr 与画面挂在**同一个 PTY** 上。主循环画完一帧后
写 stderr，这段字节落在**当前光标位置**——也就是编辑器/输入行。首帧之前写的还会被
后续帧覆盖，**会话中途**写的就留在屏幕上了。改前实测：扩展处理器抛错时，
`pi-rs: 扩展事件 … 处理器失败：…` 的字节紧跟在 `\x1b[3A\x1b[1G`（上移 3 行、回到行首）
之后**裸写**出来，没有帧内逐行清除，正好盖在输入框上。

**已改**：新增 `crates/pi-cli/src/tui_notices.rs`——TUI 下告警入队、主循环**逐帧**取走，
以 `Notice`（错误色）渲染进转录区；非 TUI（`-p` / `--mode json` / rpc）照旧写 stderr。
改造范围：

| 位置 | 原写法 | 后果 |
|---|---|---|
| `tui.rs` 8 处（键位加载、技能读取、bash 运行时、会话写盘…） | `eprintln!` | 运行期落在输入框 |
| `ext_tools.rs` 十余处（扩展事件失败、扩展加载失败、命令执行失败…） | `eprintln!` | 同上，且是高频路径 |
| `pi-ext-host/src/client.rs` 5 处（处理器失败、超时、扩展 `console.log`） | `eprintln!` | 同上 |

`pi-ext-host` 跑在**主进程内**，所以经 `set_warning_sink()` 把去向交给上层决定。

**顺带修掉一个静默丢弃**：`ext_tools::warn_runtime` 原先入的是
`settings::pending_diagnostics()`——那是**启动期**队列，只有 `tui::run` 开头排空一次，
运行期推进去的告警**永远不会显示**（既不进屏幕也不进 stderr）。

**保留的直写**（不算缺陷，守卫已白名单）：`flush_extension_log` 在
`guard.restore()` **之后**一次性把扩展日志倒到 stderr（退出路径，画面已恢复）；
扩展参数的启动期致命错误（随即 `exit(1)`，首帧之前）。

**守卫**：`scripts/check-tui-stderr.sh`（已挂 precommit）禁止上述三个文件里出现裸写，
双向验过（往 `tui.rs` 塞一行 `eprintln!` 立即报红）。

## `settings.json` 的 14 个字段逐个核对（F229）

沿用 F228 的做法核对 `Settings` 的每个字段是否在 `settings.rs` 之外被消费。脚本报
`cache_warming` "无引用"，实际是**误报**——它经方法 `cache_warming_mode()` 被
`tui.rs:6129` 使用。其余 13 个都有消费。

**`cacheWarming` 的真实状态：解析与展示对、预热机制没有。**

- 解析与回落与 ref 一致：`off`/`idle`/`streaming` 原样返回，
  未知值或缺省 ⇒ `streaming`（ref `settings-manager.ts:77`）；
- 但 ref 有完整的 `cache-warmer.ts`（会在空闲时**真发**预热请求、并有
  `status`/`cancel`/`onAgentSettled` 状态机），pi-rs **没有这个机制**；
- 因此该设置目前只影响 `/session` 面板的**显示值**，不产生任何预热行为。

**判据**：预热是**额外花钱**的可选优化（ref 注释明说 "each refresh costs money"），
不影响正确性；本机配置也没设它。按"不为无使用方的场景扩大范围"暂不实现，
若要补，按 `cache-warmer.ts` 的状态机移植即可。

## `Args` 的 47 个字段逐个核对消费情况（F225–F228）

`--session-id` 那个坑说明"解析了却不消费"会存在，于是把 `Args` 的全部字段脚本化核对
"是否在 `args.rs` 之外被读过"。结果 47 个里 3 个真未消费：

| 字段 | 结论 |
|---|---|
| `no_builtin_tools`（`-nbt`） | **已修**：`default_tools_without_extensions` 完全没读它 ⇒ 传了内置工具照样全在。现在置空内置集（扩展工具仍保留）。回归 `t_f226_no_builtin_tools`（2 项） |
| `file_args`（`@file`） | **已修**：解析层两处 push，消费端完全缺失 ⇒ `@x` 被当普通 prompt 文本发出，模型看到字面量路径。现在 `collect_file_args` 按 `<file name="绝对路径">\n内容\n</file>` 拼进初始消息（空文件跳过、不存在报 `File not found` 退 1、BOM 剥除）。回归 `t_f227_at_files`（2 项，含"内容真的进请求体"） |
| `offline`（`--offline`） | **不实现**：pi-rs **没有任何启动期网络操作**（模型目录是内置快照，无版本检查、无远端刷新）⇒ 没有可关闭的对象。接受该 flag、行为等价于永远 offline |

`print` 也在"未消费"名单里，但那是误报——它在 `Args::app_mode()` 内部被读（`self.print`）。

**`@file` 的已知限制**：ref 对图片走 `processImage`（转 base64 图片块），
pi-rs 的图片输入管线不同，本实现**只处理文本**；传图片文件会按文本读、内容出现乱码。

## `--session-id` 被解析但从未生效（已修）

README（以及 `--help`）写着"精确 ID，不存在则新建"，但 `main.rs` **从头到尾没用过
`parsed.session_id`** ⇒ 无论传什么，仍然生成随机 UUID，flag 形同虚设。

实测（修前）：`--session-id my-fixed-id` 生成的文件是
`2026-11-25T08-17-06-184Z_01a211c7-a488-…jsonl`，header 里也是那个随机 UUID。

修法照 ref `main.ts:314-336` 的 `validateSessionIdFlags` + 会话创建：

1. **互斥校验**：与 `--session` / `--continue` / `--resume` 同时给 ⇒
   `Error: --session-id cannot be combined with …`、退出 1；
2. **格式校验**（ref `session-manager.ts:268-274` 的 `assertValidSessionId`）：
   非空、只含字母数字与 `-_.`、首尾必须是字母数字，否则报 ref 原文案、退出 1；
3. **用该 id 建会话**：`SessionWriter::create_with_id` → `precompute_session_with_id`，
   文件名与 header 的 `id` 都用它。

实测（修后）：文件 `…_my-fixed-id.jsonl`、header `"id":"my-fixed-id"`；
`--session-id bad/id` 与 `--resume` 冲突都按预期报错。
回归：`crates/pi-cli/tests/t_f225_session_id.rs`（2 项，改前全红）。

## `read` 的 `offset`/`limit` 边界（已核，与 ref 一致）

| 输入 | 行为 |
|---|---|
| `offset` 超 EOF | `Offset 10 is beyond end of file (4 lines total)`，`is_error=true` |
| `offset: 0` | 从第一行开始 |
| `offset: -1` | 视为从第一行（ref `read.ts:139` 就是 `Math.max(0, offset - 1)`，**同样夹住负数**） |
| `limit: 0` | 正文为空 + 续读提示 |
| `limit: -5` | 视为未指定 |

**不 panic、不越界**。负数能静默接受与 ref 一致（ref 的 `limit` 负数会走 JS `slice` 的负索引，行为更怪），
属"与 ref 对齐"下的可接受状态，不改。

## 主题的两条设置路径（已核，语义分明）

| 路径 | 是否写 `settings.json` | 作用范围 |
|---|---|---|
| `/settings` → Theme 子菜单 | **写**（`persist_setting("theme", …)`，ref `settings-manager.ts:793-797` 的 `setTheme`） | 持久，下次启动仍是它；写后**立即重绘**，不用重启 |
| `--use-theme <name>` | **不写** | 只影响本次运行（ref `cli/args.ts:190-195` 的 `useTheme`） |

`persist_setting` 也验过：对已有 `{defaultProvider, defaultModel, quietStartup}` 的
`settings.json` 写 `theme` 后，四个键都在、`quietStartup` 等**原值保留**——
不是整文件重写覆盖。

## `--help` 列出的 41 个选项逐个实测（F214–F216）

> 本节标题原写 F214–F215，F216 补了 `--export`，一并更新。



脚本化核对：把 `--help` 里的每个 `--flag` 都真跑一遍，统计哪些被
`不支持的参数` 拒绝。**修前 5 个被拒**：`--export`、`--fork`、`--theme`、
`--use-theme`、`--no-themes`。

| 选项 | 处置 |
|---|---|
| `--approve` / `--no-approve` | **已修**（F214，见上节）——底层 `trust_override` 早就支持，只是命令行没接 |
| `--use-theme <name>` | **已修**（F215）——pi-rs 有内置 `dark`/`light`，它能临时换主题而不改 `settings.json`；缺值报 `--use-theme requires a theme name`（ref 同文案） |
| `--theme <dir>` / `--no-themes` | **已实现**（F220）——`--theme` 追加自定义主题目录，目录里的 `*.json` 按上游格式（`{name, vars?, colors}`）加载；`Theme::from_file`/`from_dir` 复用内置主题同一套解析（`vars` 间接引用也支持）、坏文件跳过不 panic、BOM 自动剥除；注册后 `Theme::resolve(name)` **先查自定义、再回落内置**；`--no-themes` 不加载。与 `--use-theme` 配合即可用自定义主题 |
| `--export` | **已实现**（F216）——读**任意**会话文件导出 HTML，打 `Exported to: <path>` 后退出；输出路径取第一个位置参数，省略时用 `pi-session-<basename>.html`；源文件不存在 ⇒ `Error: … No such file or directory`、退出 1（ref `main.ts:624-635` 同一语义） |
| `--fork <path\|id>` | **已实现**（F218）——把源会话的**全部非 header 条目**复制成当前目录下的新会话（含其它分支），新 header 带 `parentSession`=源文件、`cwd`=当前目录；源找不到 ⇒ `No session found matching '<arg>'`、退出 1（ref `main.ts:367-390`、`session-manager.ts:1812-1867`）。**注意与 rpc 的 `fork` 不同**：那个只复制目标条目的祖先链 |

**至此 `--help` 列出的选项全部真正可用**（`args.unsupported` 已无成员）；
原有两条"仍有未实现 flag"的测试相应改为钉住这个不变量。

`--use-theme` 的覆盖语义实测：`settings.json` 写 `theme: "dark"` 时，
`--use-theme light` 生效为 `light`；不带 flag 仍取 `dark`。回归
`crates/pi-cli/tests/t_f215_use_theme.rs`（3 项）。

## `--approve` / `--no-approve`（已实现）

这两个 flag **在 `--help` 里列着**（`Trust project-local files for this run`），
但 `args.rs` 把它们塞进了 `unsupported` ⇒ **传了就报错拒绝**，用户按 help 用会被打回。
底层 `trust::resolve_project_trusted` 的 `trust_override` 参数早就支持
（它是**第一级**裁决，优先于扩展裁决、`trust.json`、`defaultProjectTrust`），
只差命令行没接上。

现在 `--approve`/`-a` ⇒ `Some(true)`、`--no-approve`/`-na` ⇒ `Some(false)`，
并传进 `assemble_system_prompt` 的信任裁决。**端到端实测**：

- `--approve`：项目级技能 `<cwd>/.pi/skills` 的内容进系统提示；
- `--no-approve`：同一份技能**不进**；
- 两者都**不影响根目录的 `AGENTS.md`**——这与 ref 一致
  （`resource-loader.ts` 里 AGENTS.md 的发现与 trust 无关，只有 `.pi/skills`、`.pi/prompts`
  这类**项目级**资源受信任门控）。

回归：`crates/pi-cli/tests/t_f214_approve_flag.rs`（4 项，改前四个 flag 全被塞进
`unsupported`、字段不存在）。

## 系统提示的 docs 段在未设 `PI_PACKAGE_DIR` 时指向不存在的路径（已修）

`PI_PACKAGE_DIR` 是**差分时**才设的（指向 TS 包目录以便与 golden 对齐）。生产环境不设它，
此前直接取空串 ⇒ docs 段拼成：

```
- Main documentation: /README.md
- Additional docs: /docs
- Examples: /examples
```

全是**根目录下不存在**的路径——模型照着去读必然失败。实测请求体里就是这三行。

ref 用 `getReadmePath()`/`getDocsPath()`/`getExamplesPath()`（`system-prompt.ts:154-156`），
始终基于真实包目录，不会出现空串。

修法：未设或为空时按两级回退——
1. **可执行文件旁**（该目录下有 `docs/` 或 `README.md`，对应单机部署时把文档放在二进制旁边）；
2. 否则用**仓库 URL**（`https://github.com/nivrrex/pi-rust/tree/main`）——发布产物是单二进制，
   没有随附文档，让模型读线上文档总比读不存在的路径好。

**对拍无影响**：显式设了 `PI_PACKAGE_DIR` 时原样使用，`S01`/`S02` 仍五层全 same。

## 代码里"未实现"标记的实际核对（F209–F212）

把 `crates/` 下的"未实现/暂不支持"逐一核过，结论分三类：

**① 已实现但注释过时（已订正）**
- 模型解析的四级回落（F209）；
- 孤儿 tool call 的合成与 error/aborted 丢弃（F210，代码在 `message::transform_messages`）；
- `Decision::OverflowExhausted`——**实际已实现**（返回带 ref 原文案的 `failure` plan，
  由 `t4_18_append_compaction.rs` 的反证锚点钉住），注释却写"未实现"。
  真正缺的是**场景覆盖**：53 个 CLI 场景都走不到（需要"压缩重试一次后再次溢出"的连续状态）。

**② 真实未实现，但对当前使用无影响**
- `options.thinkingBudgets`（settings 的自定义思考预算，ref `settings-manager.ts:147`）：
  pi-rs 没有该 settings 项，只用内置默认表；
- `compat.forceAdaptiveThinking` / `compat.supportsMidConvoEffort`：
  `ModelDefinition` 未建模 `compat` 字段（只落在 `extra` 里）。
- **判据**：两者都是 **anthropic 专属**。本机 `models.json` 的 api 只有
  `openai-completions` / `openai-responses`，且 `settings.json` 里没有 `thinkingBudgets`
  ⇒ 当前使用不受影响。补给约 30 行（解析 + 传递），按"低概率场景先说明再加代码"
  暂不做；若将来接 anthropic 或用到自定义预算，按 ref 的 `simple-options.ts:57-62` 补即可。

## 模型解析的四级回落（代码里曾有"未实现"的过时表述，已订正）

`resolve_initial_model` 的实际顺序是**四级**：

1. 命令行 `--provider` + `--model`；
2. `settings.json` 的 `defaultProvider` + `defaultModel`；
3. `defaultModelPerProvider` 内置表；
4. **`models.json` 里第一个可用模型**（`available.first()`）。

实测：`settings.json` 为 `{}`、`models.json` 里配两个 provider 时，pi-rs 能正常选到模型
并发起请求（走到连不上 mock 才失败）——第 4 级是**真的实现了**。

但代码里有两处写成"未实现"，已订正：

- `settings.rs` 那条 `(None, None)` 的错误文案原写
  "内置默认表与「第一个可用模型」的回落本版本未实现"——**假的**，回落存在；
  且该文案只在四级全部落空时才会出现（实测那种情况下实际显示的是 ref 的
  `No models available. Use /login …`，根本走不到它）；
- `main.rs` 的解析顺序注释同样声称未实现。

⇒ 行为本来就对，只是注释/文案在骗人，属于"文档与实现不一致"，已改成与代码一致。

## `read` 读非图片二进制文件（与 ref 一致，不改）

实测：读一个含 NUL 的 ELF 文件（`.o`）时，`read` 返回 `is_error=false`，
把原始字节当文本吐出（含 `\u{0}`、`\u{7f}`，无效 UTF-8 处变成 `\u{FFFD}` 替换字符），
这些垃圾会进模型上下文。

**为什么不改**：ref 也是无条件按 UTF-8 读——
`core/tools/read.ts` 里 `const textContent = buffer.toString("utf-8")`，没有任何二进制嗅探。
加检测会改变工具结果的字节，进而偏离对拍基线，属于"与 ref 对齐"这条约束下的**不该改**。

**用户侧的实际影响有限**：模型看到的是乱码，通常会自己换方式（比如用 `bash` + `xxd`）；
且 `read` 对图片（jpg/png/gif/webp/bmp）有单独分支，正常场景不受影响。
若将来要加，得先确认 ref 是否在更新版本里也加了——否则会变成新的行为差异。

## 外部工具依赖（`rg` / `fd`）缺失时的行为（已核，正确）

`grep`/`find` 委托 `rg`/`fd`。清空 `PATH` 模拟未安装的机器，实测：

- `grep`：`is_error=true`，提示
  `` `rg` (ripgrep) is not available. Install it first, e.g. `apt install ripgrep` (Debian/Ubuntu) or `pacman -S ripgrep` (Arch), then retry. ``
- `find`：`is_error=true`，提示同样给出安装命令，**并点明 Debian/Ubuntu 上二进制可能叫 `fdfind`**。

**`fdfind` 回退确实实现了**（`find.rs:167-177`：优先 `PI_FD_BIN` → `fd` → `fdfind`）。
用一个只有 `fdfind` 的 `PATH` 实测：正常找到目标文件，`is_error=false`。
这一点对 Devuan 尤其要紧——`apt install fd-find` 装出来的二进制就叫 `fdfind`。

⇒ 依赖缺失是**可诊断的**（不是 `command not found` 那种谜语），无需改动。

## `edit` 工具对 CRLF / BOM 文件的匹配（已核，行为正确）

F203 修了会话文件的 BOM 问题后，顺手验了 `edit`——它是唯一会**改用户文件**的工具，
若编码处理不对会污染用户的仓库 diff。实测三种情形（`oldText` 一律用 **LF**，模拟模型输出）：

| 文件编码 | 结果 |
|---|---|
| LF | 匹配成功，替换正确 |
| **CRLF** | 匹配成功，**替换后仍保留 CRLF**（不会把整个文件改成 LF） |
| **BOM** + LF | 匹配成功，**BOM 保留** |
| **CRLF 且 `oldText` 跨两行** | 匹配成功，结果 `a\r\nMERGED\r\nz\r\n`——跨行匹配也能容忍行尾差异，且保留原编码 |

这三条是关键：若 CRLF 被改成 LF、或 BOM 被吃掉，用户 `git diff` 会看到整文件全变。
当前实现对**行尾与 BOM 做匹配期归一、写回时保留原样**，行为正确。

`write` 是整体覆盖（`new\n` 原样写入，不做编码推断），与 ref 语义一致。

## 会话文件带 UTF-8 BOM（已修）

Windows 上的记事本等编辑器保存 UTF-8 会加 **BOM**。带 BOM 时会话的首行变成
`\u{FEFF}{"type":"session"…}`，`parse_line` 解析失败、整行被跳过 ⇒ `MissingHeader`，
报「首行不是 session header」，**`-c` 直接失败**。

**ref 容忍**：TS 侧 `utils/text.ts` 有 `stripBom`；实测同一份带 BOM 的会话，
TS 能读到历史（一路走到连模型才报 `Connection error`）。所以这是行为差异、不是设计选择。

**三处读会话文件的地方统一修**（都用 `SessionFile::strip_bom`）：
1. `SessionFile::parse` —— 不剥则 `-c`/`--session` 读不出；
2. `/resume` 的扫描 —— 不剥则该会话在列表里 `cwd`/`id` 为空；
3. rpc 的条目读取 —— 不剥则 `get_tree`/`get_entries` 把整份会话当不存在
   （那里是 `.ok()?`，首行解析失败会直接返回 None）。

回归：`crates/pi-core/tests/t_f203_bom_session.rs`（2 项，改前第一条失败）。
端到端：带 BOM 的会话 `-c -p` 能读（走到连模型那步），`/resume` 扫描能拿到
`cwd=/tmp/bom/work`、`msgs=1`。

## 其它递归路径的爆栈风险评估

F201 修了 `build_tree` 后，把全仓的递归函数过了一遍，挑出**输入深度可控**的两处：

| 位置 | 实测阈值 | 能否被用户触发 | 处置 |
|---|---|---|---|
| `build_tree`（会话树） | 3000 层 abort | **能**——会话长了必走 `/tree`、`/fork`、`get_tree` | **已修**（迭代 + 前序写缓冲） |
| `ext_tools::flatten_render_frame`（扩展 UI 树） | 200 层正常、**800 层 abort** | 需要扩展返回 ≥800 层 `stack` 嵌套 | **不改**，理由见下 |

`flatten_render_frame` 的处置理由（按"低概率场景先说明再加代码"）：
- **失败模式**：扩展返回极深嵌套的 `stack` 组件树时，`flatten_render_frame` 递归爆栈，
  `SIGABRT` 整个 TUI 进程（不是优雅报错）；
- **发生概率**：低。正常组件树十几层；只有写错或异常生成的扩展才会嵌套几百层；
- **代价**：改成迭代约 20 行（与 F201 同形）；
- **ref 对照**：ref 的组件渲染同样是递归、同样没有深度限制
  （`session-selector.ts:262` 的 `walk`），所以这不是本实现的偏差。

⇒ 暂不改。若将来有扩展真的这么干，按 F201 的同形改法（显式栈 + 直接写缓冲）即可。

**已确认安全的路径**：`/tree` 面板的构建走 `build_tree`（F201 后是迭代），
`settings_selector` 里没有深度递归。

## `build_tree` 递归爆栈（已修）

`build_tree` 用递归构造树。深链实测：**2000 层 173ms 正常、3000 层
`has overflowed its stack` 并 `SIGABRT` 整个进程**。真实会话上万条，
`/tree`、`/fork`、rpc `get_tree` 都会走到这里 ⇒ **会直接崩**。

修的过程踩了两次：

1. 第一版改成"显式栈 + 按 id 缓存整棵子树"，栈不爆了，但 10000 层要 **4.03s**
   —— 每层都把子树字符串复制进父节点，深链下总复制量 O(n²)；
2. 最终版用**显式栈的前序遍历，直接写进一个输出缓冲**
   （`{"entry":E,"children":[` → 子节点 → `]}`），每段只写一次，O(输出大小)。
   2 万层 **0.695s**，比原来 3000 层就崩是质的差别。

**一个必须记下的边界**：树本身不爆栈了，但**消费方**未必受得住——
`serde_json::from_str` 解析 2 万层嵌套会撞它自己的递归上限
（`recursion limit exceeded`）。这是解析器约束，不是本实现的问题；
测试因此改为按文本结构核对，不再整棵树喂给解析器。
若将来有真实场景需要把极深树序列化后**再解析**，那条路要单独评估。

回归：`crates/pi-core/tests/t_f201_deep_tree_stack.rs`（改前 SIGABRT）；
`t3_40_session_tree` 3 项仍通过；`S28`/`S29` 的树形 rpc 响应**五层全 same**
（输出形状零变化）。

## 大会话加载耗时的实测分解（`-p` 13.5s 不是单一瓶颈）

6 万条 / 11MB 会话、发布产物、`-c -p`（provider 连不上以便快速失败）实测 **13.5s**。
逐段打点后分解如下，**没有一处是本次该修的缺陷**：

| 环节 | 耗时 | 性质 |
|---|---|---|
| `--version` / `--help` / `--list-tools` | 9ms | 基础启动，极快 |
| 系统提示组装（读 docs/AGENTS.md/skills/templates） | **0.3ms** | 可忽略 |
| 会话读 + 解析 6 万条（debug） | 2.06s | 必要 IO + JSON 解析 |
| 投影 3 万条消息（debug） | 0.92s | 必要计算 |
| **重试退避**（`retry_delay_ms=2000`，2 次 ⇒ 2s+4s） | **6s** | **正确行为**（ref 同样指数退避） |
| `build_session_path` | 毫秒 | F199 已修 |

关键发现：那 **6 秒是 `-p` 的指数退避重试**，不是启动慢——我用连不上的 mock
才会稳定触发。换可用 provider 时这段不存在。
先前的"6 秒启动成本"判断是**错的**：`--no-session` 那 6 秒同样来自重试，
不是初始化。

## 深链路径仍是 O(n²)（已修）

F56 把 `id → 下标` 换成了索引，但**漏了环保护的 `seen`**：它当时是
`Vec<usize>`，每跳 `seen.contains(&index)` 都是线性扫描 ⇒ 整体仍是 O(n²)。

实测（修前）：5000 条 154ms、10000 条 585ms、20000 条 **2.34s**，每翻倍约 3.8 倍。
换 `HashSet` 后同一组变成 12/18/37ms（翻倍约 2 倍），20000 条提速 **63 倍**；
40000 条 71ms。真实会话 6.4 万条时，旧实现会到十几秒量级。

**怎么暴露的**：`precommit` 里那条性能测试（阈值 5s）在并行负载下报红
（实测 5.19s），单跑却通过 2.7s。起初以为只是阈值脆弱，量了 5k/10k/20k
三档才发现是**真实的平方增长**——阈值红得有道理，只是刚好卡在边界。

**测试同时改进**：
- 绝对阈值从 5s 收到 2s（O(n) 实测约 40ms，留 50 倍余量）；
- 新增 `scaling_is_linear_not_quadratic`：比 **n 与 2n 的耗时比**
  （O(n)≈2 倍、O(n²)≈4 倍，分界取 3），**不受机器快慢影响**，
  避免再出现"并行时假红、单跑绿"。
- 双向验过：把 `seen` 换回 `Vec` 时该用例报
  `规模翻倍后耗时涨了 3.91 倍`，改回后通过。

## `/model` 与 `Ctrl+P` 切模型已实测

预置两个 provider（`p1/m-alpha`、`p2/m-beta`），真 pty 实测：

- `/model` 面板选中另一项 ⇒ 页脚从 `(p1) m-alpha` 变 `(p2) m-beta`，
  **provider 与 model 一起换**（不是只换名字）；会话追加 `model_change`；
- `Ctrl+P` 循环同样生效。

两条路径都会同步 `ctx.model`（扩展读到的也跟着变）。

## `/logout` 已实测

真 pty 实测（`auth.json` 里预置两个 provider）：
`/logout` 删掉 `login_providers` 里那一个（`acme`）、**保留其余**
（手工写进 `other` 的凭据原样留在文件里），状态行 `Logged out of acme.`，
页脚 provider 前缀消失。

**行为说明**：只有 `login_providers`（= `models.json` 里声明的 provider）中的条目
会被 `/logout` 处理；手工塞进 `auth.json`、但 `models.json` 里没有对应 provider 的
条目不会被删——它本来就不在登录面板的管理范围内。这是有意的边界，不是漏删。

## `auth.json` 损坏时的可见性（已修）

与 F193 的 `models.json` 同一个模式：`stored_api_key` 里 `serde_json::from_str` 失败后
`.ok()?` **静默返回 `None`**，于是"凭据文件写坏了"的表现是
`Provider "x" 没有可用的 API key：请在 auth.json、--api-key、环境变量…中提供其一`——
用户会去翻环境变量、翻 models.json，而真正原因一个字都不提。

现在解析失败会推一条启动期配置诊断，TUI 顶部显示
`Warning: Failed to parse auth.json: not valid JSON` + 文件路径。

**踩到的第二个坑**：诊断入队了却不显示。`build_tui_agent_parts` 有多条提前
`return Ok(None)` 的路径（resolve 失败 / `build_provider` 失败），坏配置命中哪条
取决于坏在哪一步；而我原先只在 `AgentParts` 构造处 drain。现在统一在 `tui::run`
里兜底：`parts` 为 `None` 说明那条路没搬，就地补上。
同时修掉一处**覆盖**：`parts` 为 `None` 时原代码会把 `state.startup_warnings`
无条件覆盖成空数组，把已入队的告警整个抹掉。

## `/login` 全流程已实测（无模型时的唯一出路）

README 承诺"无可用模型也能进 TUI，`/login` 配置后当次会话即可对话"。真 pty 实测（发布产物）：

- `/login acme` 直接进 key 输入态（ref `interactive-mode.ts:5708-5716` 的同名行为）；
- 输入后 `auth.json` 写成 `{"acme":{"key":"…","type":"api_key"}}`——与 ref 的
  `{provider: {type:"api_key", key}}` 一致；
- 状态行 `Logged in to acme.`，页脚 provider 从 `unknown` 变为 `acme-1`
  ⇒ **当次会话即可用**，不必重启。

**一点如实记录**：API key 在输入时**明文回显**。查 ref 用的是普通 `input` 组件、
没有掩码（`startProviderLogin` 里只有 oauth/api_key 两种 label，无 password 模式），
所以这与 ref 一致、不是本实现的疏漏。想改成掩码会偏离 ref 的屏幕字节。

## 会话文件损坏与 `-c` 无会话：行为已核，未改

**坏行（截断/非法 JSON）被静默跳过**——这是**与 ref 一致**的行为，不是偏差：
ref `session-manager.ts:355-369` 的 `parseSessionEntries` 里明确
`catch { /* Skip malformed lines */ }`，pi-rs 的 `SessionFile::parse` 同样
`let Ok(parsed) = parse_line(line) else { continue; }`。
实测：末行被截断成半截 JSON 的会话，`-c` 仍能读出前面的完整消息并继续。
**已知代价**：用户不会被告知丢了哪一行——与 ref 同样静默。

**`-c` 在无会话时**：
- **TUI**：正常进入（与 ref 的 `continueRecent` 一致——它找不到就新建一个空会话）；
- **`-p`**：明确报错退出（`pi-rs: 找不到可续写的会话文件（目录：<分区>）`）。
  与 ref 的"静默新建"不同，但 print 模式"续写一个不存在的会话"本就没有意义，
  报错比静默新建更诚实。**该分支没有对拍场景覆盖**（`S13d` 只覆盖"有会话可续"），
  若将来需要与 ref 对齐，改这里即可。

## 会话列表排序需要确定的次级判据（已修）

`scan_sessions` 与 `scan_all_sessions` 此前只按 `modified_ms` 降序排。**mtime 相同的几条**，
相对顺序就由 `read_dir` 的返回顺序决定——而那在 POSIX 上**没有保证**
（不同文件系统、不同批次都可能不同）。`/resume` 的默认选中项与对拍结果都依赖顺序稳定。

实测：三个 mtime 完全相同的会话（`zz`/`aa`/`mm`）扫出来是 `["mm","aa","zz"]`，
既不是创建序也不是字典序，纯粹是目录项顺序。现在次级判据取**路径**
（同一目录内唯一，足以定序）；跨分区聚合列表同理。
回归：`settings_selector::f191_tests`（2 项，改前第一条即失败）。
`T53-resume-sort`/`T62-resume-sort-order`/`T36-resume` 三个场景仍然 same。

## 输出顺序不得依赖哈希迭代（已加自检）

`HashMap`/`HashSet` 在 Rust 里**每个进程用随机种子**，迭代顺序每次启动都不同——
比 `read_dir`（单次运行内至少稳定）更危险：表现是"偶发、不可复现、换进程就变"。

F191 全仓盘过一遍，**当前没有违规**：

- `projected_messages` 用 `order: Vec` 保序、`by_id` 只查表；
- `build_tree` 的 `kids: HashMap<String, Vec<String>>` 里 Vec 按文件顺序推入；
- `context_edit` 的 `removed: HashSet` 只做成员判断，遍历的是 `order`；
- 渲染层 `spans_by_line` 只按 `index` 查，遍历 `md_lines`；
- 请求头是 `Vec<(String,String)>` 或 `BTreeMap`（有序）。

加了 `scripts/check-hash-order.sh` 并接进 precommit（第 19 项）：
抓 `for … in <xxx_map/set/by_id/kids>.iter()/.keys()/.values()` 这类形状，
可用 `// hash-order-ok: 理由` 就地豁免。**双向验过**——塞一个
`by_id.iter()` 的临时文件会 FAILED，删掉恢复 OK；precommit 19/19。

## 回归脚本必须自己固定 locale

golden 里的 bash 报错是**中文**（`F52g` 的 `/bin/bash: 第 5 行：cd: … 没有那个文件或目录`），
而 bash 的报错语言随 `LANGUAGE`/`LC_MESSAGES` 变。`regress-tui.sh` 与 `regress-cli.sh`
此前**都不固定** locale ⇒ 同一个二进制在不同 shell 下得出不同结论：

- `LANGUAGE=C` 时 F52g 判 **different**（纯假红：文本一个字都没差，只是成了
  `No such file or directory`），连带 `F52a`/`F52f` 一起红——全量从 18 涨到 27；
- 固定成 `zh_CN` 后恢复 357 same / 18 different。

两个脚本现在都 `export LANGUAGE="${DIFTEST_LANGUAGE:-zh_CN}"`、
`LANG="${DIFTEST_LOCALE:-zh_CN.UTF-8}"`（可用环境变量覆盖，便于在没装中文 locale 的
机器上跑——但那种机器上 golden 必然不同，属于环境不满足而非代码问题）。
**已在 `LANGUAGE=C` 的外壳下验证**：全量仍是 357/18，差异清单不变。

**`TZ` 查过，不需要固定**：`T53-resume-sort`（显示相对时间）与 CLI 的 `S01` 分别在
`TZ=UTC` / `TZ=Asia/Shanghai` 下跑，四层全部 same、画面逐行相同。
`raw.bin` 的字节确有差异，但定位后是**每次运行随机生成的临时目录名**
（`/tmp/tui-T53-…-kjq58u` vs `…-j2ifW6`），与 `TZ` 无关——所以"字节不同"不能当
判据，要按 `compare-tui` 的规范化结果看。

## 发布产物必须当成被测对象跑一遍

所有对拍与回归默认用 `target/debug/pi-rs`（`run.mjs` 里写死该路径，属红线文件）。
**构建树通过 ≠ 发布产物可用**：F183 的缺陷就是拿已发布产物做冒烟时发现的
（`/resume` 的 `All` 在 `PI_CODING_AGENT_SESSION_DIR` 覆盖会话目录时扫错根），
而此前用构建树 + 默认布局验过多次、都是通过的。

做法（不改红线文件）：把发布产物临时拷到 `target/debug/pi-rs`，
跑完再拷回原二进制。已按此验过 `a03c9ba` 与 `ed6b16b`：

| 项 | 发布产物结果 |
|---|---|
| CLI 全量（`regress-cli.sh`） | 81 same / 11 different / 0 runfail（与构建树一致） |
| **TUI dark 全量**（`regress-tui.sh`，`THEME=dark LANGUAGE=zh_CN`） | **357 same / 18 different / 0 runfail**，差异场景与构建树**完全一致**（`T05 T08 T09 T18 T45-tree-checkout T106-md-list-hang`，各 3 runs，全是 footer 用量百分比）；`nogolden=69`（23 场景 ×3，无 golden） |
| TUI `T01 T05 T08 T09 T36-resume` | 全部 same |
| Esc 中断（gate 场景） | `Operation aborted` **1 条**、`Working` 立即消失 |
| `/export` HTML | 单文件 891B、user+assistant 都在、无外部引用 |
| `/resume` Tab（自定义会话目录） | 跨分区会话可见 |
| `--version` / sha256 | 与 Release 清单一致、`static-pie linked` |

**每次发布后按这张表跑一遍**，别只信构建树。

## CLI 场景的差分现状（全量实测）

`scripts/regress-cli.sh` 是 CLI 侧的批量回归，覆盖 `golden/` 那批。
当前全量：**92 个可跑场景，92 same / 0 different / 0 runfail**。

`S24b` 的内置 `llama.cpp` 命令已接入；`S49` 的 YAML 锚点/别名已由 `serde_yaml` 支持。

**历史路径长度差异（当前 CLI 已消除）**：早期 `estimatedTokensAfter` 曾因 system prompt
真实路径与 golden 占位符长度不同偏移 35 token；当前 92 个 CLI 场景已全部 same。

`S13c` 的事件顺序问题已修：`compaction_start` 现在在**摘要请求之前**派发
（ref `agent-session.ts:2761-2772`），单次运行的事件序列与 golden 逐条相同。

### TUI 场景

`scripts/regress-tui.sh` 每个场景默认跑 3 次（脚本汇总行是 run 计数，不是场景计数）：

| 主题 | 场景数 | 结果（按场景） |
|---|---|---|
| dark | 125 | 125 same / 0 different |
| light | 27 | 27 same / 0 different |
| truecolor | 7 | 7 same / 0 different |

dark 的 148 个场景中有 23 个没有 golden，脚本记为 nogolden；不计入上表。
回归脚本现在同时检查 `helperExit` 与 `exitCode`，超时 SIGKILL 不再被视为 same。

## 平台与工具

- `ctrl+z`（挂起到后台）依赖作业控制：在孤儿进程组（某些自动化 pty 夹具）里
  `SIGTSTP` 会被内核丢弃，此时只能观察到"恢复终端 + 重画"。
- `grep` / `find` 委托外部 `rg` / `fd`，**不自动下载**（上游会先尝试下载）。
  `fd` 的探测顺序：`PI_FD_BIN` → `fd` → `fdfind`。
- TUI 里工具结果只渲染调用头与（按规则的）正文预览；`read` 在折叠态按上游语义不显示正文。

### `-c` 首屏性能

本机 120×34 PTY、相同配置与扩展、热缓存首帧中位数：

| 会话 | 日常 AOT | F245 优化后 pi-rs |
|---|---:|---:|
| 23 MB | 4215.9 ms | **3661.7 ms** |
| 119 MB | 5454.1 ms | **9108.9 ms** |

23 MB 会话已快于 AOT；119 MB 仍慢于 AOT。剩余成本主要来自跨进程 Node/jiti
扩展宿主启动，以及首屏前对全历史 thinking 标签和工具行的 eager 处理。
该数据是本机实测，不作为所有硬件上的通用性能承诺。

### 扩展的 `notify` / `setStatus`（已修复）

`notify` 三档与 `setStatus` 已接入 TUI；当前实现与验证见下方
"扩展（P9-1-E1c 更新）"。

## `llama.cpp` 内置命令

- RPC `get_commands` 已包含官方内置 `/llama` 元数据（`S24b` same）。
- `/llama` 刷新 router 目录并写入 `models.json`；`/llama load|unload|download <model>` 执行对应管理操作。
- `/login llama.cpp` 保存可选 API key 与 `LLAMA_BASE_URL`；未设置 URL 时使用 `http://127.0.0.1:8080`。
- 与官方完整扩展的差异：当前使用原生子命令而不是完整 TUI 管理器；尚未实现启动期自动刷新与扩展 `ctx.modelRegistry` API。

## 扩展（P9-1-E1c 更新）

- **发现规则**：实现了项目级 `<cwd>/.pi/extensions` → 全局目录的顺序与去重、目录内直接文件
  （`.ts`/`.js`/`.mjs`）与 `index.ts`→`index.js`→`index.mjs` 入口（ref `loader.ts:657-800`）。
  `package.json` 的 `pi.extensions` 清单与 `-e/--extension` 显式路径**已支持**；
  排序按路径（ref 用目录读取顺序）。
- **UI 帧**：`notify` 三档已按 ref 渲染（info 状态行 / `Warning: ` / `Error: `），
  `setStatus` 在 footer 下单独一行（按 key 排序、清洗、截断）。本批 TS 录制逐帧 `same`
  （`scenarios-tui/T88-ui-notify` 与 `scenarios-tui-light/L-T88-ui-notify`）。
  **F61/44 已修复（已验证）**：T88 的样式层差异从 **819 格降到 6 格**，且那 6 格
  全是临时目录随机后缀（`/tmp/tui-T88-ui-notify-<rand>/work` 里的字符），
  与样式无关 ⇒ **颜色与空行现已逐单元格一致**。
  根因（F61/43 定位）：`Status` 补**尾随**空行、`Notice` 补**前导**空行，相邻时叠成 2 行；
  ref 的 `showStatus`/`showWarning`/`showError` 各自只是 `Spacer(1)` + `Text`
  （`interactive-mode.ts:3709-3712`、`:4469-4473`），相邻两条之间只有 1 行。
  修法：`paragraph_blank` 增加 `previous_notice_like` 参数——上一条是
  `Status`/`Notice`/`Warning` 时，本条 `Notice`/`Warning` **不再补前导空行**
  （前一条的尾随空行已充当）。
  **关键**：`Notice` 的**尾随**空行必须保留——F61/43 曾试图去掉它（样式差异降到 177），
  但那会让**末条目**少一行，`compare-tui` 从 same 变 different（回归）。
  只抑制前导、保留尾随，两个指标才同时成立（样式 6 格、文本 same）。
  单元用例 `crates/pi-tui/tests/t_f61_notify_spacing.rs`；改前必失败。
  **取证方法**（后续同类问题沿用）：`compare-tui` 会**丢弃 SGR 并折叠连续空行**，
  文本层 same 不代表样式一致；必须用 `compare-style.mjs` 逐单元格比对
  （`{ch,fg,bg,bold,italic,dim,inverse}`）。

- **工具执行帧（已支持）**：`createBashToolDefinition` 等宿主动作经
  `host.action.call` 交回宿主执行；见下方"宿主动作帧"与
  `crates/pi-ext-host/tests/action_call.rs`。
- **Ctrl+O 只覆盖 `renderResult`**：扩展自定义组件（`ui.custom` 面板、工厂式 widget 里的
  自定义组件）不参与展开/折叠，`expanded` 变化不会触发它们的重渲染。
- **`pi.appendEntry` 的会话文件等待**：宿主可能早于 `SessionWriter` 启动，消费者线程最多等 10s
  再写；理想做法是"路径就绪后回放队列"（待查，见 `notes/p9-1e4e/ledger.md`）。
- ~~**Ctrl+O 的展开语义**~~ **已生效（F110 核实）**：`renderResult` 会在切换后按新
  `expanded` 重新渲染（`tool-execution.ts:352`）。证据是 `T90-render-hook` 的 golden 里
  `expanded=false` 与 `expanded=true` **都出现**，而该场景在 125/125 全量回归里是 same。
  此前记的"逐帧对拍尚未通过"已不成立（那份记录早于 `T90` 落地）。
- **渲染钩子的样式**：`theme.fg` 用 ref `theme/dark.json` 生成的色表（56 色，truecolor SGR），
  未知色名按 ref `theme.ts:323-327` **抛错**（宿主回落 `unsupported`）。`T90` 的逐帧 same
  已经包含 SGR，说明前景色与 ref 一致。
  **`bg` 已按 ref 实现（F107）**：背景色是**另一套键**（`theme.ts:92-101` 的 `ThemeBg`
  只有 `selectedBg`/`searchMatchBg`/`userMessageBg`/`customMessageBg`/`toolPendingBg`/
  `toolSuccessBg`/`toolErrorBg` 七个），值取自两张主题表的 `vars`。此前生成器
  （`tools/ext-host/gen-theme-colors.mjs`）直接用**前景色键集**加 `48` 前缀造背景表，
  于是 `theme.bg("accent", …)` 会返回一个 ref 里不存在的颜色——而 ref 对这类型错误
  参数是抛 `Unknown theme background color`。生成器已改为只生成那 7 个键，
  并且**只替换色表块、保留文件尾部手写的 `makeThemeStub` 等**（此前整文件覆写会抹掉它们）。
- **真实扩展的逐行样式对拍未做**：`ask_user`/`cg_recall`/`web_read` 的 renderCall 已验证
  序列化为 text 帧，且其用到的 **8 个色名 SGR 与 ref 逐项一致**（`notes/p9-1e4b/s6-colors-compare.txt`）；
  但**没有**在真实工具调用下做 TS↔pi-rs 的整帧逐行 SGR 比对（`ask_user` 走 `ui.custom`、
  `cg_recall` 需要网关 id、`web_read` 需联网，三者都缺可复现输入）。
- 渲染钩子：`renderCall` 支持 `Text`/`Spacer`/`Container` 的降级（工具块 contentBox 左边距
  与 ref 对齐；`unsupported` 整块回落到内置渲染）；`renderResult` 的帧**已接进工具块**
  （`tui.rs` 的 `render_tool_block(…, "result", …)`，`T90` 的 golden 里出现
  `result-extension:…` 且该场景 same）；`Box`/`Markdown`/`Table` 不支持（已装扩展中无使用）。
  **`Text` 已与 ref 对齐（F109）**：三处此前的实现差异都已修正——
  ①空文本（或纯空白）返回**空数组**而不是 padding 空行；
  ②每行加**左右两侧** padding 并补齐到整宽（此前只加左侧）；
  ③`paddingY` 的空行是 **`width` 个空格**而不是空串；
  另补上 ref 的 paddingX 夹取（`min(paddingX, floor((width-1)/2))`）与制表符换 3 空格。
  `Text` 是扩展渲染的基础组件，这三处差异会直接体现在逐帧对拍里。
  **`SelectList` 已支持（F108）**：`pi-interact` 的多选面板真用它，而 shim 的 `render`
  此前**完全忽略**构造时传入的五个样式函数（`selectedPrefix`/`selectedText`/
  `description`/`scrollInfo`/`noMatch`）⇒ 面板没有高亮、没有描述列、没有滚动提示。
  现已按 ref `select-list.ts:75-109`/`:183-215` 实现：空列表走 `noMatch`、
  宽度 > 40 时渲染描述列、超出 `maxVisible` 时给 `(n/total)` 的 `scrollInfo` 行、
  选中行套 `selectedText`。
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
- `ctx.ui.custom`：支持编辑器位置或 overlay、`render`/`handleInput`/`done`；
  `overlayOptions` 的 anchor/尺寸与 `onHandle` 未实现；面板按键映射覆盖
  Char/Enter/Esc/Backspace/Tab/方向键。
- **未实现**：宿主动作帧目前只实现 `bash`，其余动作尚未接入。

- **kitty 键盘协议 flags 偏离 ref（5 而非 7）**：关闭位2（上报事件类型），因为抬起事件是"退出后 shell
  回显 `0;5:3u`"的唯一来源（复现脚本 `tools/difftest/kitty-release.py`）。代价：拿不到 release 事件
  （我们本来就把它们全部丢弃，`tui.ts:1073-1076` 语义）；若将来实现 `wantsKeyRelease` 必须恢复位2。
  偏离的夹具通过生成器映射记录（`gen-term.mjs` 的 `applyKnownDeviation`），不是手改期望值。
  依据与推翻条件见 `notes/decisions.md`「F4/S4 改（定案）」。

- **`!` / `!!` 用户 bash（已实现）**：执行前派发 `user_bash`，扩展可返回
  `{result}` 或 `{operations}`；取消、折叠与 `bashExecution` 会话消息均已接入。
  回归见 `crates/pi-cli/tests/p9_f7c_resume_bash.rs`。

- **`bashExecution` 会话消息**：类型、serde、上下文投影（`!!` 不进上下文）、token 估算与 compaction
  切点**已与 TS 侧互验**（F7b）：
  ① TS `SessionManager` 生成的样本 → pi-rs 解析 → 再序列化**逐字节一致**（含未知字段与键序）；
  ② pi-rs 写出的会话被 ref 真实 `SessionManager.open` 加载后投影一致；
  ③ 六种情形的投影文本与 ref `convertToLlm` 的 oracle **逐字节一致**；
  ④ `-c` 恢复后的请求体断言**已完成**（`crates/pi-cli/tests/p9_f7c_resume_bash.rs`：普通项以
  `bashExecutionToText` 文本进入 user、`!!` 项不出现、顺序正确，且会话文件原有行逐字节不变）。
  **未完成**：TUI 的 `!`/`!!` 输入与 `BashExecutionComponent` 渲染、`/tree` 的命令摘要文本、
  `user_bash` 事件派发（F8/F9）。因此 `pi-secrets` 对 `!` 命令的脱敏路径仍不会触发。

- **`p9_e3b_append` 偶发（F10 记录，F61/36 复核）**：F10 时 8 路并行 80/80、240/240 丢条目，
  报 `ECONNRESET`，根因是**主进程先退出、扩展动作帧没处理完**；两处试修（等落盘标志、
  落盘时并入已有行）无效并回滚。
  **F61/36 复核结果**：在 8 路 / 16 路并行下各压 80、120、240 次**都没复现**（含把运行时
  退回 current-thread 的对照），说明该偶发在当前代码上已不可见（`c966b99` 改成
  1-worker 多线程运行时后，actor 空转时派生任务也会被轮询，退出窗口被大幅收窄）。
  但复核过程中找到了**同一根因下的确定性缺口**并已修：TUI 退出路径会
  `emit_session_shutdown_sync` → `shutdown_host`，而 **`--print` 路径两条都没做**
  ⇒ 只在 `session_shutdown` 里 `appendEntry` 的扩展，print 模式下条目**必然丢失**
  （单次即复现，无需并发）。已补上（顺序：先派发事件再关宿主，各带 1s 上限）。
  用例 `p9_f61_print_shutdown_append.rs`；改前必失败。
  **F61/37**：`rpc` 入口有同一缺口（也没有派发 `session_shutdown`/关宿主），已一并补上，
  三个入口（print / TUI / rpc）现在一致。用例
  `p9_f61_rpc_shutdown_append.rs`（关 stdin 触发**正常退出**路径，等 10s 条件轮询）；
  改前必失败（实测 50s 超时后失败）。
  **仍未做**：8 路并行档的可重复复现手段（当前无法稳定复现 ⇒ 按纪律不改那部分代码）。

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
  ~~**rpc 入口仍未覆盖**：rpc 的会话文件是懒创建的……对应测试已写好但标 `#[ignore]`~~
  **F61/29 核实为过时记录**：rpc 路径在建会话时就 `set_session_file`
  （`main.rs` 的 rpc 装配处），`p9_f11_entry_paths::rpc_entry_writes_append_entry`
  一直正常执行并**通过**（实测 2/2），注释里说的 `#[ignore]` 从未真正加上。
  已改写该注释与本文。sink 超时不再静默丢弃，会打印 `customType`/`data` 告警。

- ~~**`/new` 未实现**（F12 复核）~~ **F61/29 核实为过时记录**：`/new` 在 F16 已实现
  （`tui.rs` 的命令分支 + `p9_f16_new_session.rs` 的 pty 用例），与本文后面
  "**`/new`（F16 已实现）**"那条重复。保留此行只为说明曾被误记。

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

- ~~**两个 pty 生命周期用例改为 `#[ignore]`（F24）**：`p9_f16_new_session` 与
  `p9_f18_reason_events` 在全量并行时被挤到超时，改为显式单跑~~
  **F61/29 核实为过时记录**：这两个文件里**没有任何 `#[ignore]`**（全仓仅 1 处 `#[ignore]`，
  在 `t3_45_compaction_oracle.rs`，与本条无关），它们一直在 test-gate 的全量并行里正常执行：
  实测各约 2s 通过，近若干轮 test-gate 全绿即为证据。本条按记录规范删除过程性叙述。

## F28
- ~~`ctx.ui.setToolsExpanded` 只记录状态并回执，**不驱动** pi-tui 的工具行展开~~
  **F61/28 已实现（已验证）**：新增 `UiFrame::SetToolsExpanded` → `TuiEvent::ToolsExpanded(bool)`
  → `pi-tui` 设置 `tools_expanded` 并刷 `Tool output: expanded|collapsed` 状态行
  （与 `Ctrl+O` 同一句文案，ref `interactive-mode.ts:4419`）。与 `Ctrl+O` 的区别是
  **设置而非取反**，重复调用幂等。
  推翻条件原先写的是"用户要求扩展能程序化展开工具输出"——该条件**已成立**：
  已装扩展 `pi-remote-control` 用它在远程端切换（`adapter.ts:463`），此前远程切换无效。
- `[Extension issues]` 已按宽度折行（ref `interactive-mode.ts:1874-1894` 的 `Text` 语义），
  但 80/120 列的 pty 截图对照未做（只有单测）。
- ~~底栏 `?/0 (auto)`（真机 ②）：未取证、未修~~ **F61/26 部分修复（已验证）**：
  真实配置副本并排对比（100×24，`--no-session`）pi-rs 与 TS 的底栏**逐字一致**
  （`0.0%/1.0M (auto)`），`?/0` 当前代码**无法复现**。但取证过程发现一处**真实的
  逐字差异**并已修：`percent` 为 `null`（压缩后、下次助手响应之前的"用量未知"，
  ref `agent-session.ts:3865-3868`）时，ref `footer.ts:150-156` 输出
  **`?/<window>`（无 `%`）**，我们输出 `?%/128k`——而被当作预期的单测
  （`usage=0 不应清零已有用量`）把这个错误**固化**成了断言。已改输出并逆转该断言，
  新增 2 项用例（未知不带 `%`、已知仍带 `%`），改前必失败。
  **F61/27 已补齐恢复路径（已验证）**：`pi_core::context_usage`（`has_post_compaction_usage`/
  `usage`，ref `agent-session.ts:3858-3902` 的 `getContextUsage` 判定，6 项单元用例）
  + 两处接入（`main.rs` 的 `resumed_context_tokens` 与 `tui.rs` 的 `resumed_usage` 估算）。
  判据是"最近一次压缩之后有没有可用助手 usage"——没有就是**未知**（页脚 `?/<window>`，
  扩展侧 `ctx.usage.tokens === null`），不是 0。
  改前实测：同一会话恢复后页脚 `0.7%/200k`（把**压缩前**的 usage 当成压缩后），
  或 `0.0%/200k`（未知被写成 0）；改后 `?/200k (auto)`。

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

10a-2. **已关闭（F38/K5）**：`T89h-md-nested` 与 `T89f-md-elements` 的"互斥"是误判——
    `T89f` 的列表是 tight（pulldown 不产生 `Paragraph`）、`T89h` 的是 loose（保留 `Paragraph`）；
    按 ref `markdown.ts:800-802` 对 loose 列表项之间补空行后，两份 golden 同时 same。

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

10b. **markdown 直渲染对拍尚未收敛（初版记录）**

    > **该对拍链一度被误删**：`crates/pi-tui/examples/md-render.rs` 在 `e5ad11c`
    > （keybindings 那个提交，内容与之无关）里被整段删掉，而
    > `tools/difftest/md-diff.mjs:33` 与 `md-aggregate.mjs` 仍在 `cargo run --example md-render`，
    > 两条对拍工具静默失效多个批次没人发现。已从 `e5ad11c^` 原样恢复（175 行），
    > 复跑 `md-diff.mjs --entry <cli.js> --theme dark --width 60 --in md-corpus/basic.md`
    > 输出 `different=13`，与本条记录的现象一致。：新增 `tools/difftest/md-oracle.mjs`（TS 侧直渲染，
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

14. **F38 未完成**：K5 已关闭（见 10a-2），但「真实 `pi-ai-summary` 的折叠工具行/thinking 行」仍未对齐——
    该扩展是对 `ToolExecutionComponent.prototype.render` / `AssistantMessageComponent.prototype` 打**原型补丁**，
    ref 里扩展与 TUI 同进程所以生效；我们扩展在独立 node 进程，补丁对 Rust 渲染零影响。
    需要对「宿主把工具行渲染委托给 runner」的通道（约 200–300 行，跨 runner/Rust/tui），未实施。

15. **F40 未完成（缺项：N3）**；F41 已定位并修复该缺项的根因（见 17）。F39 未完成（缺项：M0 工具行委托、M1e 归因数量）。
    退出后 `^[[A`：`restore()` 的 drain idle 已对齐 ref `terminal.ts:384` 的 `idleMs=50`（原 150 ms），
    退出收尾从 161 ms 降到 60 ms；但**真机（二进制 `e62e986`）退出后立刻按上键仍出现 `^[[A`**，
    即该修改**不是主因**。drain 50 ms 保留。

16. **F40 N1/N2 结论**：用真实 10 个扩展（脱敏副本）在 pty 夹具里逐项对拍（10 全开 / `-ne` / 逐个单开，
    各 3 次），**pi-rs 返回时该 pty 上没有任何存活子孙进程、termios 也没有被改回、`ESC[A` 不被字面回显**，
    与 TS 一致 ⇒ 夹具仍缺真机变量（Alacritty 自身行规程/回显、真实 shell 的 readline 时序）。
    已确认的**代码偏离**（但夹具下无可观测后果）：pi-rs 的 `session_shutdown` 经 `enqueue_emit`
    投递到独立线程队列即返回（`ext_tools.rs:53-67`），不等处理器完成、也不等宿主退出；
    ref 是 `await emit(event)`（`core/extensions/runner.ts:254-263`，由
    `interactive-mode.ts:4170` 的 `runtimeHost.dispose()` 等待）。N3 因"未给明确机制"未实施。

17. **`^[[A` 的根因与修复（F41）**：真机 strace 显示 pi-rs 恢复 cooked → 5.8 ms 后 pi-rs 退出 →
    12.5 ms 后**宿主 node** 对 fd 1/2 各做一次 `TCSETS`（写回 `ICANON|ECHO`）→ 20.3 ms 后 node 退出。
    根因：宿主 node 的 stdout/stderr **继承了 pty**（`crates/pi-ext-host/src/client.rs` 原为
    `stdout(inherit)/stderr(inherit)`），Node/libuv 在进程退出时会对 tty 做 `uv_tty_reset_mode`
    写回启动时保存的 cooked 模式；此刻 shell 早已把 tty 设为 raw ⇒ 用户首个按键被内核字面回显。
    修法：①宿主 stdio 全改管道，stdout/stderr 由宿主转发到自身 stderr（日志不丢）；
    ②退出顺序对齐 ref：等 `session_shutdown` 处理器（≤1s）→ 关宿主并等其退出（≤1s，超时按 PID kill）
    → 再恢复终端。用例 `crates/pi-cli/tests/p9_f41_host_no_tty.rs`（改前 FAILED）。
    影响面：扩展再也看不到 `process.stdout.isTTY`/`columns` 为真（已 grep 现有 10 个扩展，无依赖）；
    Node 侧的 `console.log` 之类 stray stdout 会被转到 stderr 而非终端 stdout。

18. **F44/S0 并发日志的已知窗口**：`extensions.log` 的启动截断用「读尾部 → 写临时文件 →
    `rename`」，在"读"到"rename"之间其他实例写入的少量行会随旧 inode 丢弃（每次截断最多
    丢弃该窗口内的行）。日志本身另有**每实例内存缓冲**（256 KiB，丢最旧并提示丢弃行数），
    因此退出输出不会混入别的实例的行。推翻条件：改为按实例分段写不同文件（会改变
    "单文件可 grep"的现有用法，需先确认）。
19. **F44/S4 Windows 路径规则未验证**：页脚 cwd 缩写依赖 `home_from` 的
    `HOME` → `USERPROFILE` 回落（`crates/pi-cli/src/runtime.rs:31-50`，ref
    `interactive-mode.ts:1322-1332` 的 `formatDisplayPath` 用 `os.homedir()`）。Linux 上
    「只设 `USERPROFILE`」与「设 `HOME`」两种情形均已对拍一致；**Windows 下
    `\` 分隔符、盘符大小写、`USERPROFILE` 含非 ASCII** 未验证。推翻条件：Windows 真机
    显示仍为完整路径或出现 `~\Desktop\…` 与实际不符。
20. **F44/S3 Windows 终端标题未验证**：改用 OSC 0（ref `terminal.ts:520-523`）后 Linux pty
    已与 TS 一致；Windows conhost 是否需额外 `ENABLE_VIRTUAL_TERMINAL_PROCESSING`
    （crossterm 的 `supports_ansi()` 已尝试开启，失败忽略）未验证。推翻条件：Windows 真机
    仍显示 cmd 默认标题。
21. **F58/Q 工具行首帧等待阈值无 ref 依据**：`crates/pi-cli/src/tui.rs` 的
    `FIRST_DRAW_WAIT_MAX_ROWS = 8`（待定扩展工具行 ≤8 时首帧**不**空等最多 1500 ms）。
    ref 在同一进程内同步渲染工具行（`coding-agent/src/modes/interactive/components/tool-execution.ts:110-118`），
    不存在"等扩展回填折叠行"这一步，因此**找不到对应阈值**；这是 pi-rs 跨进程渲染的实现细节。
    设为 8 的依据是"少量行重画两遍的字节代价可忽略"（3 行约几百字节），并非 ref 规格。
    推翻条件：①在 ref 找到等价的"首帧等待渲染器"点 ⇒ 改为照抄 ref 策略；
    ②折叠行改回本进程内渲染 ⇒ 删除该阈值。
22. **F58/C Alacritty 的 OSC 52 开关未核实**：`/allcopy` 在无 `xclip`/`xsel`（headless）或
    远程会话下走 OSC 52（`\x1b]52;c;<base64>\x07`，与 ref `utils/clipboard.ts:18-24` 逐字节一致）。
    **Alacritty 默认是否启用 OSC 52 写入**未在 ref 的 `docs/`、`CHANGELOG.md` 中找到依据，
    本批未验证，标"未核实"。推翻条件：在 Alacritty 中实测 `copyToClipboard` 后剪贴板内容
    与写入文本不一致，或需在 `alacritty.toml` 显式开启对应选项。
23. **F58/H 编辑器制表符口径与排版不同（有意保留）**：排版统一展开 `\t` → **3 空格**
    （ref `text.ts:61`/`markdown.ts:298`/`diff.ts:18`/`render-utils.ts:32`），
    而编辑器粘贴归一化用 **4 空格**（ref `editor.ts:1146` 的 `normalizeText`）。
    两处未合并成同一常量，避免改变括号粘贴的缩进与光标列。推翻条件：ref 后续版本
    把两处口径统一（届时合并）。
25a. **F61/24 起 `/hotkeys` 面板跟随 `keybindings.json`；默认键位仍走 golden 常量**：
    `hotkeys_panel(&keymap, cols)` 在 `keymap.is_all_defaults()` 时返回
    `hotkeys_panel_for_width`（既有 golden 逐字转录，保证屏幕输出不变），
    否则按 `keybindings::hotkeys_markdown` 现场生成并渲染。
    **已知差异**（这也是默认走常量的原因，不是可以忽略的小事）：动态生成的表格在
    60 列下 Editing 段列宽为 34（golden 是 33），且 `Tab` 行在 golden 里归入
    Other 段首位、ref 模板里归入 Editing 段末——**两者内容都对**，但与 golden
    不逐字节一致。推翻条件：把 ref 模板的分段与列宽算法对齐到逐字节，
    然后删掉这个分叉、统一走动态生成（届时需重录各宽度档 golden）。
24. **F61/25 起 2 条内置斜杠命令未移植**：`share`、`bug`
    （`changelog` 在 F61/25 实现：`pi_core::changelog`（`parse`/`render_all`/
    `normalize_links`/`parse_file`，ref `utils/changelog.ts` 逐项对照，
    11 项单元用例）+ `/changelog` 命令。数据来源与 ref 同源
    `<packageDir>/CHANGELOG.md`（`config.ts:455-457`），按 `PI_PACKAGE_DIR` 解析；
    **pi-rs 自身不带这份文件**（它属于 TS 包），缺失时按 ref 语义显示
    `No changelog entries found.`（`interactive-mode.ts:6515`）。
    已知限制：ref 末尾的 `Extensions` 快捷键段未实现（需要扩展注册表里的快捷键，
    pi-rs 无该接口）；面板宽度在**打开时**取一次终端宽度，之后改窗口大小不重排
    （ref 由 TUI 布局每帧重排））
    （`reload` 在 F61/22 实现：`reload_host()` 换掉扩展宿主进程并重新派发
    `session_start`；前置检查照 ref `interactive-mode.ts:6171-6178`（流式/压缩中拒绝），
    状态行原文照 `:6243-6246`。为此把 `HOST`/`SPECS` 从 `OnceLock<…>` 改为
    `OnceLock<Mutex<…>>`——**ESM 模块缓存决定"重新 import 不会重新求值"，
    只能换进程**，这也正是 `/reload` 长期未能实现的原因）
    （`trust` 在 F61/21 实现：`pi_core::trust`（`trust.json` 读写、最近祖先查询、
    `project_trust_options`、`has_trust_requiring_project_resources`，ref
    `core/trust-manager.ts` 逐项对照）+ `TrustSelector` 面板 + `/trust` 命令；
    项目级 `skills`/`prompts` 与 `ctx.isProjectTrusted()` 都改为读真实决定，
    ref `resource-loader.ts:1029/1043`、`extensions/types.ts:336-338`）
    （`scoped-models` 在 F61/16-F61/20 实现：纯逻辑 `crates/pi-tui/src/scoped_models.rs`
    + 模糊匹配 `crates/pi-tui/src/fuzzy.rs`（ref `packages/tui/src/fuzzy.ts` 逐行转写）
    + 面板 `ScopedModelsSelector`。**Enter 切换**（ref 用 `tui.select.confirm`，空格属于
    搜索框）/`ctrl+a` 全开/`ctrl+x` 全关/`ctrl+p` 整组按 provider 开关/`alt+↑↓` 重排/
    `ctrl+s` 落盘 `enabledModels`；搜索框、`maxVisible=8` 分页、`(n/total)`、`Model Name:`
    行、按键提示页脚、`名称 [provider]` 行格式均对齐 TS 实测（100×30 并排对比：
    结构、提示行、分页、计数逐行一致）。
    **未做**：TS 多一行 `Refreshing model catalogs…`（catalog 热刷新状态，我们不热刷新）；
    `ctrl+c` 在面板内的"清空搜索/取消"语义（`TuiEvent` 无 `CtrlC`，该键走中止路径）。
    推翻条件：补上 catalog 热刷新，或给输入层加 `CtrlC` 事件）
    （`copy` 在 F61/5 实现：与 `ctrl+x` 同为"复制最后一条助手消息"；
    `name` 在 F61/6 实现：写 `session_info` 条目，ref `session-manager.ts:1302-1325`；
    `clone` 在 F61/8 实现：`handleCloneCommand` → 复制当前叶子路径到新会话，
    ref `interactive-mode.ts:5379-5398`；
    `fork` 在 F61/13 实现：列出可分叉用户消息 → 以该消息**之前**为界分叉，
    ref `interactive-mode.ts:5341-5368`；
    `import` 在 F61/14 实现：`/import <path.jsonl>` 把外部会话复制进会话目录并切过去，
    ref `interactive-mode.ts:5591-5640`；
    `export` 已实现两条分支（ref `interactive-mode.ts:6260-6275`：`.jsonl` 结尾走 JSONL、
    **否则走 HTML**、无参数给默认文件名）：
    `/export <path.jsonl>` 导出当前分支（ref `session-export.ts:32-45`）；
    `/export <path.html>` 或 `/export` 导出**单文件 HTML**。
    **HTML 与上游的差异（有意）**：ref `core/export-html/` 是带侧栏/搜索/过滤器的
    交互式页面（约 300KB 模板与 vendor JS）。本实现不内嵌上游资源
    （README 承诺不分发上游源码），只做**静态可读**的单页：消息按角色分段、
    工具调用折叠成 `[tool] <name>` 一行、深色主题、样式内嵌、**无脚本、无外部依赖**。
    默认文件名 `pi-session-<源文件 basename>.html`（ref 用 `<app>-session-<basename>.html`）。
    HTML 特殊字符做实体转义；`content` 为空的消息（被中断的回合）跳过不生成空段。
    回归：`crates/pi-cli/tests/f182_html_export.rs`（3 项）。
    （ref `core/slash-commands.ts:19-44` 的 24 条中余下部分）。pi-rs **识别**它们
    （不会误当 prompt 发给模型），回车后打印
    `Command /<name> is not implemented yet.`，不执行任何动作。
    改前这些命令落到 `other => {}` ⇒ 屏幕一个字节都不变（与 `/todo` 冻结、`/compact`
    无反应同属"无响应"缺陷，见 `notes/p9-f60/plan.md` 第 3 节）。
    推翻条件：需要其中任一条时按 ref 逐条移植（每条都有自己的 ref 实现与状态副作用）。
25. **F60/3 未实现的扩展 `ctx` / `ctx.ui` / `pi` API**（F60/5 已补
    `abort`/`shutdown`/`isIdle`/`isProjectTrusted`/`hasPendingMessages`，见下）（用户已装扩展均未使用，
    全仓 grep 逐项确认；ref 行号见 `extensions/types.ts`）：
    - `ExtensionContext`：`modelRegistry`(`:325`)、`scopedModels`(`:332`)、
      `compact`(`:350`)。
      ~~`getSystemPrompt`~~ **F68 已实现**：runner 的 `ctx.getSystemPrompt()` 返回
      宿主下发的 `ctx.systemPrompt`；三个入口（TUI / rpc / print）在装配时都用
      `set_ctx_system_prompt(&system_text)` 填入当前生效的完整提示。
      **F77 补派发 `agent_before_settle`**：ref `agent-session.ts:1531-1545` 在**会话层**
      派发，不区分运行模式；pi-rs 此前只有 `--print` 有（`print_mode.rs:662`），
      TUI 与 rpc 从不派发 ⇒ `pi-handoff-gate`（用户已装）的**核心续跑机制**
      在交互模式下完全不工作（它靠这个事件决定放行/续跑/熔断）。
      现三入口都派发，TUI 侧还把它给的 `entries` 以 `custom` 消息落盘。
      **续跑已实现（F106）**：`should_continue` 为真且扩展给了 `entries` 时，
      TUI 用那些条目把上下文推进到"最后一条不是 assistant"（ref 的 `canContinue`
      条件），再跑一轮——不带新的 user 消息，与 ref 一致。上限 8 轮兜底。
      实测：扩展要求续跑 2 次时 `agent_before_settle` **恰好**被调用 3 次
      （首轮 + 2 次续跑），且第二次请求体里确实带着扩展注入的催促文本。
      **回归**：TUI 全量 125/125 same，续跑没有改变任何既有场景的画面。
      **F74 修复 `sessionManager.getLeafId()`/`getBranch()` 的起点**：ref
      `session-manager.ts:1467-1477` 用 `this.leafId`（**当前叶子**），pi-rs 此前用
      "文件最后一条"。分叉后导航到中间节点时两者不同 ⇒ 扩展沿错误分支回溯
      （`pi-secrets`/`pi-ai-summary` 共 7 处 `getBranch()`，据此算上下文与用量）。
      宿主经 `ctx.leafId` 下发真实叶子，且 `ctx_snapshot()` **每次重读**
      （导航/分叉后即变，缓存会过期）。
      **F73 修复 `pi.getCommands()` 的语义偏差**：ref
      `agent-session.ts:3021-3044` 返回 `[...extensionCommands, ...templates, ...skills]`
      三类（技能名带 `skill:` 前缀），pi-rs 此前只返回扩展注册的命令 ⇒
      模板与技能对扩展完全不可见。现由宿主在启动期（`loaded_templates` /
      `loaded_skills` 建好后）下发，runner 按 ref 顺序合并。
      **F70 修复 `pi.getAllTools()` 的语义偏差**：ref
      `agent-session.ts:1259-1267` 返回 `_toolDefinitions` **全部**（内置 + 扩展），
      而 runner 此前只返回扩展自己注册的那张表 ⇒ 内置 `bash`/`edit` 等永远不在列表里。
      `pi-secrets`（用户已装）在 `session_start` 里统计
      `getAllTools().filter(name => name === "bash")`，那行日志恒为 `bashCount: 0`。
      现由宿主经 `set_ctx_builtin_tools` 下发内置清单（`sourceInfo.kind = "builtin"`），
      runner 合并两来源、**扩展同名覆盖内置**（ref 的工具注册表以名字为键）。
      ~~`signal`~~ **F69 已实现**：`ctx.signal` 是 getter，返回**当前操作**的
      `AbortSignal`；`ctx.abort()` 先同步 abort 本地控制器、再通知宿主。
      **生命周期**按 ref `runner.ts:858-861`/`agent-session.ts:3100` 的语义：
      每次构造 ctx（= 一次新操作）时若上次已 abort 则换新控制器，避免
      "abort 一次、此后所有回合都读到 aborted"（那会让
      `pi-agent-resilience` 的取消保护永久生效）。
      **已知偏差（工程判断）**：ref 的 signal 由 agent 每次操作新建、粒度精确到
      单次请求；pi-rs 以"事件边界"近似，同一事件处理期间共享一个控制器。
      （`isIdle`/`isProjectTrusted`/`abort`/`hasPendingMessages`/`shutdown`
      已在 F60/5 实现——`pi-agent-resilience` 真的依赖 `ctx.abort()`，
      缺失时它的 `try/catch` 会把 TypeError 吞掉、自动恢复静默失效。）
    - `ExtensionUIContext`（`types.ts:137-300`）：
      **已实现**：`select`、`confirm`、`input`、`notify`、`setStatus`、`setWidget`、
      `setFooter`、`setHeader`、`setTitle`、`custom`、`getToolsExpanded`、
      `setToolsExpanded`。
      **未实现**（15 个）：`onTerminalInput`、`setWorkingMessage`、`setWorkingVisible`、
      `setWorkingIndicator`、`setHiddenThinkingLabel`、`pasteToEditor`、`setEditorText`、
      `getEditorText`、`editor`、`addAutocompleteProvider`、`setEditorComponent`、
      `getEditorComponent`、`getAllThemes`、`getTheme`、`setTheme`。
      **为什么不做**：本机已装的 **11 个**扩展里，对 `ctx.ui.<名字>` 的**调用**逐一 grep 均为 0
      （`getTheme`/`editor` 命中的是扩展内部同名参数与注释，不是 `ctx.ui` 调用）。
      按"不为无使用方的场景扩大范围"暂不实现；`getAllThemes`/`setTheme` 另有一层原因：
      pi-rs 只有 `dark`/`light` 两个内置主题、没有自定义主题目录，
      返回固定两项或切换主题的价值都有限。
    - ~~`after_provider_response` 事件未实现~~ **F64/9 已实现**：`Stream` 带出
      `status`/`headers`（`response_meta`），`EventSink::after_provider_response`
      在收到响应、消费流之前派发。`before_provider_request` 已实现（F60/10，
      只通知不改写载荷——两个使用方都只计数/遥测）。
    - `ExtensionAPI`（`:1349-1600`）**未实现**：`registerShortcut`、
      `registerMessageRenderer`、`registerEntryRenderer`、`registerProvider`、
      `unregisterProvider`、`sendMessage`、`exec`、`setModel`。
      **已实现**（各见下）：`setThinkingLevel`/`getThinkingLevel`（F67）、
      `setSessionName`/`getSessionName`（F66）、`setActiveTools`（F157）、
      `setLabel`（F162）。
      `setModel` 单独说明：它要按 `provider/modelId` 重建 provider，而 pi-rs 的
      provider 在构造期解析一次（`runtime::build_provider` 需要 `ResolvedModel`）；
      运行期切换需要把模型注册表一并保存下来。**已装的 11 个扩展对 `setModel`/`getModel`
      代码级 grep 为 0**，因此按"不为无使用方的场景扩大范围"暂不实现。
      ~~`setThinkingLevel`~~ / ~~`getThinkingLevel`~~ **F67 已实现**：此前前者是空实现、
      后者恒返回 `"off"`（假值）⇒ 扩展读写档位全部无效。现走
      `host.thinking.setLevel`，宿主侧复用 `append_thinking_level_change`
      （写条目 + `thinking_level_changed` + `thinking_level_select` 一并完成）。
      上面列出的**未实现**成员都经 `runner.mjs` 的 `noteNotImplemented` 打
      `[ext-host] … 未实现` 警告（不静默）。`pi.unregisterProvider` 此前是
      唯一的**静默空实现**（`unregisterProvider() {}`），已按同一规则改为警告；
      由 `crates/pi-ext-host/tests/p9_f165_no_silent_noop.rs` 双向钉住
      （未实现的必须警告、已实现的不许报未实现）。
      ~~`setSessionName`~~ **F66 已实现**（连同 `getSessionName`）：此前 runner 侧是
      空实现（`setSessionName() {}`、`getSessionName()` 恒 `undefined`，且**重复定义**
      两次、后者覆盖前者）⇒ 扩展改名静默无效。现走 `host.session.setName`，
      复用 `/name` 与 `/resume` 改名的同一写入器（`append_session_info_to`）；
      `getSessionName` 读宿主下发的 `ctx.sessionName`，两个
      `append_session_info*` 出口都会同步刷新它。
      `registerMarkdownTransformer` 已在 F60/6 实现（`pi-secrets` 靠它脱敏密钥，
      此前只记录不应用 ⇒ 密钥会画到屏幕上）。
    推翻条件：某个已装扩展开始依赖上述任一项时，按 ref 逐个补齐并补对拍用例。
27a. ~~**F60/13 `session_before_tree` 未实现**~~ **F61/38 已实现（已验证）**：
     `ext_tools::emit_session_before_tree` 在 `/tree` 导航与摘要生成**之前**派发
     （ref `agent-session.ts:3641-3656`），返回 `{cancel, summary}`：
     `cancel` 取消导航并给状态行，`summary` 直接写 `branch_summary` 并**跳过模型调用**。
     一并修掉 runner 侧一个更基础的缺陷：通用分支此前 `await handler(...)` **丢弃返回值**，
     导致**所有**"有返回值语义的其余事件"失效——`session_before_switch` 的
     `{cancelled:true}` 其实一直无效（`emit_session_before_switch` 永远返回 `false`），
     只是一直没有用例钉住它。现按 ref 语义合并多个 handler 的非空字段。
     用例 `crates/pi-cli/tests/p9_f61_before_tree.rs`（2 项，含 `before_switch`）。
     推翻条件：ref 后续改事件返回语义时同步。

27. **F60/11 `ctx.ui.custom` 面板的按键覆盖面**：宿主把按键经 `ext.custom.input`
    原样转给扩展组件（`interactive-mode.ts:2858-2900` 同语义），当前映射覆盖
    `字符 / Enter / Ctrl+J / Esc / Backspace / Tab / Shift+Tab / ↑↓←→`。
    **未覆盖**：`Home`、`End`、`Delete`、`PageUp`、`PageDown`——用户已装扩展在
    custom 面板里没有用到它们（全仓 grep `Key.home|Key.end|Key.delete|Key.pageUp|
    Key.pageDown` 为 0），因此不属"已装扩展用到的未实现项"。
    推翻条件：某个已装扩展的 custom 组件开始用 `matchesKey(data, Key.home)` 之类，
    届时在 `key_bytes`（`crates/pi-cli/src/tui.rs`）与 `pi_tui::input` 两侧同时补映射。
27b. **F60/19 未派发的扩展事件**：~~11 个未派发~~ → **F61/F64 已全部派发**
     （文件开头 27b 有逐项状态表）。page 首那份清单停留在 F60/19 当时的快照，
     保留在此仅为对照历史；以开头表格为准。

## F63/D：`settings.json` 的**加载期**校验（用户要求的偏离）

ref 只对 JSON 语法/读写错误报警（`settings-manager.ts:591-593` 的 `recordError`
→ `settings-diagnostics.ts:4-9` 的 `Invalid settings file <path>: <msg>`），
**不校验**枚举与名称存在性（逐个 grep 确认无对应代码）。

pi-rs 额外实现了以下检查，均为"用户要求的偏离"：**只产生 warning、
不阻断启动、不改文件**，沿用既有 `Warning:` 样式。

| 检查项 | 触发条件 | 文案前缀 |
|---|---|---|
| `defaultThinkingLevel` 枚举 | 不在 `off/minimal/low/medium/high/xhigh/max` | `settings.json: defaultThinkingLevel "<v>" is not one of …` |
| `defaultProjectTrust` 枚举 | 不在 `always/never/ask` | `settings.json: defaultProjectTrust "<v>" is not one of …` |
| `defaultProvider`/`defaultModel` | 组合不在模型注册表 | `settings.json: defaultProvider/defaultModel "<p>/<m>" is not in the model registry` |
| `defaultTools` 名称 | 名称不是已知工具 | `settings.json: defaultTools entry "<n>" is not a known tool` |

**`hideThinkingBlock` 类型**（非布尔）**不在上表**：它由 serde 反序列化直接覆盖，
报的是 ref 原样的 `Invalid settings file <path>: invalid type: …`（`settings-diagnostics.ts:7`），
属"与 ref 一致"而非偏离。实测 `"hideThinkingBlock":"yes"` 输出该文案并回落默认值。

**JSON 语法**一项与 ref 一致（不是偏离）：坏文件只 warning、用默认值继续。

**呈现位置**：TUI 模式下这些诊断**不进 stderr**（inline 模式会把首屏挤下一行），
而是入进程级队列、由 `tui::run` 作为会话区的 warning 行呈现（ref
`interactive-mode.ts:1129-1137` 的 `showWarning` 位置）。`--print`/`--mode rpc`
仍走 stderr。

**F65 补齐**：`Could not restore model <p>/<m>`（会话里的 provider/模型已不存在）
此前也是 `eprintln!`，屏幕上根本看不到；现同样入队、作为 warning 行呈现。
用例 `p9_f65_session_history.rs` 的 `restore_failure_warning_lands_in_the_transcript`。

**TUI 装配不再有致命失败路径**：`build_tui_agent_parts` 返回 `Result` 但**无 `Err`
分支**（所有致命配置都在内部降级为"无模型进 TUI"，见 B/C 两项），因此那里不需要
兜底刷出。剩下唯一可提前退出的是 tokio runtime 创建失败（环境性故障，非配置），
该路径会把队列刷到 stderr。

**`enabledModels` 条目校验已移除（F80）**：ref 对这类条目的处理是
`No models match pattern "<pattern>"`（`model-resolver.ts:325/349`），
pi-rs 已由另一条路径产出同样提示。再报一条 `enabledModels entry … is not in
the model registry` 只是重复刷屏，且把 `T80-model-scope-invalid`（专门测这个情形）
的屏幕多顶一行 ⇒ 该场景与 golden 不同。

**对 `real-parity-linux` 的影响（已实测）**：这五项校验在**配置确实错配**时必然
多出一行 `Warning: settings.json: …`，而 ref 静默接受 ⇒ 那些矩阵格必然 different。
实测（F99 复核，默认矩阵 `cols{80,110}-{ext,ne}-keys` × `{none,ctrl+o}`）：
**八格全部 different，首要差异都是这同一条
`defaultProvider/defaultModel … is not in the model registry`**，只是行号随
`ctrl+o` 是否展开而不同（11/13/14/16）。
（F80 当时记的是"四格、ctrl+o 那四格不受影响"——那次折叠态下行位置恰好被别的行
对齐掉了，展开后才显现。**以 F99 的实测为准：八格同源**。）
**这是本偏离的已知代价，不是回归**——
把该项去掉，这四格会恢复 same，但那样就丢掉了用户要的校验。

**运行期告警同理（F116）**：扩展宿主回调线程（`setThinkingLevel`/`setSessionName`/
`appendEntry` 等的写失败与"等待会话路径超时"）共 6 处此前直接 `eprintln!`，
运行期触发时会把 TUI 首屏顶掉。现统一走 `warn_runtime`（TUI 入诊断队列、
其它模式写 stderr）。
**这 6 处没有端到端用例**：它们的触发条件是"扩展宿主写会话失败"或"会话路径
10 秒未就绪"，实测连 `--no-session` 下 `SESSION_FILE` 也被设置、`appendEntry`
正常写入，构造不出可稳定触发的输入。改动仅由既有测试全绿与代码审阅支撑。
**能验证的部分已验证**：正常回合（含 `--no-session`）跑下来 stderr 均为 **0 行**，
即至少没有残留的泄漏路径。

**扩展加载失败同理（F115）**：`host.load_errors()` 那条此前**同时** `eprintln!` 与
`record_issue`——TUI 里会先在 `[Extension issues]` 段显示一次，stderr 那行又把首屏
挤下去一次（与 F63/K、F75 同类）。现按「UI 通道是否已注册」区分：TUI 不写 stderr，
`--print`/`rpc` 仍写（那里不写就等于丢信息）。

**呈现位置**：该警告同样走诊断队列（TUI 模式），不写 stderr —— 此前
`resolve_startup_model` 里是裸 `eprintln!`，在 `tui::run` 之前执行，**直接把
`Model scope:` 那一行顶掉**（矩阵里表现为首行不同）。现已改为与
`load_settings` 的 `warn_once` 同一处理。

**推翻条件**：ref 后续版本加入同类校验时，改用 ref 的文案与触发条件；
或用户要求去掉这些额外警告。

## 部署产物的验证状态（F104，实测）

以下都是**实际跑出来的**，不是按设计推断：

- `cargo tree -i openssl-sys` ⇒ `did not match any packages`（依赖树里没有 openssl）。
- `cargo build --release --target x86_64-unknown-linux-musl` 成功；产物
  `file` 报 `ELF 64-bit ... static-pie linked ... stripped`、`ldd` 报 `statically linked`。
- 该二进制 `--version` 输出 `pi-rs 0.87.1` 且 commit 与当时 HEAD 一致；
  用本地 mock provider 跑 `--print` 正常产出 `Hello world`。

**用真实配置副本跑过 TUI**（`~/.pi/agent/` 的 settings/models/auth/extensions
复制到临时目录，不碰原目录）：正常启动、`Model scope:` 在首行、正常退出。
该配置里 `defaultProvider`/`defaultModel` **确实写反了**
（`deepseek-v4.1-flash` 是模型名而不是 provider、`opencode-go` 是 provider 而不是模型），
F63/D 的那条告警因此在真机上真实触发——这既是校验存在的理由，也解释了
`real-parity-linux` 八格差异的来源（矩阵夹具沿用了同一份错配）。
