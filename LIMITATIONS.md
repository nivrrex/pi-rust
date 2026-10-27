# 已知限制

本文件记录**与上游（pi 0.87.1）行为存在差异**或**尚未实现**的地方。所有条目都对应上游客
户端代码中的具体位置，便于后续补齐。

## 已知差异（P8 出口后遗留）

样式级基准：真彩 `golden-tui-truecolor/`、256 `golden-tui/`，判定工具
`tools/difftest/compare-style.mjs`（逐单元格 字符/前景/背景/粗体/斜体/暗淡/反显）。
**P8e-5 批末实测**（7 个场景，两侧文本均 same）：真彩合计 11 格、256 合计 13 格。

1. **T05 的 8 格：列表符号与行内代码** —— 上游按 span 染色（`dark.json:54` `mdCode`、
   `dark.json:60` `mdListBullet`，均为 accent `#8abeb7`），我们目前只有**整行**样式通道，
   整行染色会把正文一起染掉（实测更差），因此保留差异。已试过两种方法：
   ①整行样式（会把 `- ` 之后正文也染色）；②在 `render_markdown_styled` 里产出 span 并按行索引
   叠到帧 `spans`（首版把文本也改坏了：跨帧缓存路径没有携带 span，wrap 后偏移不匹配）。
2. **T01/T77 的 3 格 + T28 的 2 格：版本行 ` pi v0.87.1` 的属性** —— 属**上游录制自身不稳定**：
   同一份 TS 产物、同一场景，`run-1` 与 `run-2` 彼此就差这 3 格（`r1 c0` 的 `dim`）；
   T28 在 256 模式下 TS 的 `pi` 不加粗、真彩下加粗（模式相关）。不是本实现可稳定对齐的目标。
3. **首次渲染的帧切分与上游不同**（上游首帧只画启动头部，我们首帧基本画完）——
   已用 tmux 真实终端状态机回放证明**终态一致**（两侧 ` ls .` 都在第 12 行、非空行数一致），
   文本级 189 个场景全 same，登记为"时序差异，终态一致"，不做批量策略改造。
4. **`T06` 无真彩 golden**：上游在 `COLORTERM=truecolor` 的 pty 下不产出工具行（多次复现），
   该场景只有 256 基准。
5. **Windows 真机未验证**（仅交叉编译通过）；真机 IME 崩溃未复现；退出残留 500ms 有界。

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
