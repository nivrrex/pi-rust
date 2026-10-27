# 已知限制

本文件记录**与上游（pi 0.87.1）行为存在差异**或**尚未实现**的地方。所有条目都对应上游客
户端代码中的具体位置，便于后续补齐。

## 已知差异（P8 收尾遗留）

1. **帧切分与行对齐**：同一场景下本实现与上游的**帧数、每帧写入的行集合不同**
   （例如 T73 上游 7 帧、本实现 8 帧；上游首帧只画启动头部，本实现首帧几乎画完）。
   由此派生出"工具块整体差 1–2 行"与若干样式级残差。已用字节对照、上游 `PI_TUI_DEBUG`
   日志、逐帧 `changedRows` 统计三种方法确认上游原因；修复需要改渲染循环的批量策略，未实施。
2. **工具调用头为单色**：上游 `renderers/ls.ts:15-22`、`renderers/bash.ts:41-48` 是"工具名加粗 +
   参数 accent"的行内分段，本实现整行同色。需要 span 通道按**条目行区间**定位调用头（按内容匹配
   会命中错误的行，已实测）。
3. **选择器面板配色**：上游 `theme.ts:1209-1217` 的 `getSelectListTheme` 有五条规则
   （选中前缀/文字 accent、描述 muted、滚动提示 muted、无匹配 muted）。本实现只覆盖选中行与
   `No matching`，其余未做。
4. **markdown 元素色**：标题/列表符号/行内代码/代码块围栏与内容尚未按 `md*` 主题键着色；
   需要在 `render_markdown` 内部按 pulldown-cmark 的 Tag 附样式。
5. **`T06` 缺真彩 golden**：上游在 `COLORTERM=truecolor` 的 pty 下不产出工具行（多次复现），
   无法录制基线。

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
