# `1~` 残留：复核结论与标记重组修复（win-fixes）

日期 2026-10-31；被测 `commit 47ae544`（修复前）与 `win-fixes`（本文件对应的修复后）。
测试文本全为合成内容（`PAYLOAD` / 横幅四行）。

## 一、按评审要求做的三件事

### 1) 用 `PI_TUI_EVENT_DUMP` 抓 `1~`：**未能复现**

| 通道 | 运行次数 | 事件日志里 `Esc` 事件 | 结果 |
|---|---|---|---|
| `Shift+Insert`（SendInput，真实按键） | 3 | **0** | input = 期望值，无 `1~` |
| 右键粘贴（SendInput 真实鼠标） | 3 | **0** | 同上 |
| `Ctrl+V`（SendInput） | 2 | **0** | 本机 WezTerm 未把 Ctrl+V 绑到粘贴（input = 0） |

事件日志里**没有任何 `Esc`/`[`/`2`/`0`/`1`/`~` 的标记序列**：本机这条 ConPTY + WezTerm 组合把
括号标记完全吞掉了，`BracketPaste` 根本没有被触发 ⇒ 无法在本机抓到 `1~` 那一次。

⇒ 没有真机捕获就不下"根因"结论。现有依据只有两条：用户截图（`1~` 落在正文末尾）+ Linux 侧
纯逻辑矩阵（拆分点/间隔穷举）。修复按"防御性正确"做，不宣称复现。

### 2) 通道与内容区分

- `Shift+Insert` 与右键粘贴都能完整送达（无 tab 时 `input=238`，与粘贴文本长度一致）；
- 含 tab 时修复后 `input=242`（= 238 + 4 − 1，tab 展开成 4 空格），修复前 `238`（tab 被删）；
- `Ctrl+V` 在本机 WezTerm 不产生粘贴（与 pi-rs 无关）。

### 3) 是不是 tab 修复带来的回归：**不是**

| 构建 | 通道 | 内容 | input | 期望 | `1~` |
|---|---|---|---|---|---|
| `25ca523`（47ae544 之前） | Shift+Insert | 横幅 4 行 | 238 | 238 | 无 |
| `25ca523` | 右键 | 横幅 4 行 | 238 | 238 | 无 |
| `25ca523` | Shift+Insert / 右键 | 横幅 + tab | 238 | 239 | 无 |
| `47ae544` | Shift+Insert / 右键 | 横幅 4 行 | 238 | 238 | 无 |
| `47ae544` | Shift+Insert / 右键 | 横幅 + tab | 242 | 239 | 无 |

⇒ 两版都没有 `1~`；tab 修复只改变 tab 的渲染（+3 个空格），不引入该残留。

## 二、Linux 侧矩阵落成的夹具（事件类型 + 间隔，无内容）

`crates/pi-cli/src/bracket_paste.rs` 的 `split_marker_matrix_tests`，每条都在相邻事件间按 10 ms
粒度驱动 `flush()`（模拟主循环定时器）：

| 用例 | 拆分点 | 间隔 | 断言 |
|---|---|---|---|
| `start_marker_split_after_esc_does_not_leak` | `Esc` 与 `[200~…` 之间 | 41 / 119 / 121 ms | 正文恰好 `PAYLOAD` |
| `start_marker_split_after_bracket_does_not_leak` | `Esc[` 之后 | 121 ms | 同上 |
| `end_marker_split_after_20_does_not_leak_tail` | 结束标记 `Esc[20` 之后（`1~` 之前） | 121 ms | 正文 `PAYLOAD`、恰好一次 `Paste`、**零** `EscReleased` |
| `lone_esc_then_marker_still_delivers_payload` | 真 Esc 放行后再来完整标记 | 41 ms | 正文 `PAYLOAD` |

## 三、修复（`crates/pi-cli/src/bracket_paste.rs`）

旧实现两处"半途而废"：
1. 前缀按 `Esc` 的时刻算 120 ms 窗口，超时 **只 `seq.clear()`**、不记已丢弃 ⇒ 后续 `1~` 当正文；
2. `Esc` 在 40 ms 放行后，迟到的 `[` 不再被认作标记起点 ⇒ `[200~…` 整段落进正文。

新实现：
- 窗口改按**最后一次匹配**算（`seq_at` + `SEQ_IDLE_MS=500`），正文冲刷造成的大间隔不再丢前缀；
- 记录 `released_esc_at`：`Esc` 放行后 120 ms 内到达的 `[` 仍按标记起点处理；
- 已匹配前缀若确认不是标记，**原样按正文交回**（`Paste(prefix)`），不再静默丢字——
  这条也顺手修掉了旧测试 `escape_then_unrelated_bracket_does_not_lose_typing` 里
  "`Esc`+`[A` 丢掉 `[`"的隐藏丢字。

## 四、修复后真机回归

| 用例 | 结果 |
|---|---|
| 横幅 4 行（Shift+Insert / 右键） | `input=238`（= 文本长度） |
| 横幅 4 行 + tab | `input=242`（tab 展开 4 空格） |
| `AAA⏎\tXYZ` | `input=11`、`col=7` |
| 1242 字符单行 | 仍折叠 `[paste #1 1242 chars]` |
| `PI_TUI_EVENT_DUMP` | 三条通道仍 0 个 Esc 事件（说明本机没走标记路径，修复对它们无副作用） |
