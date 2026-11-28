# W-P8e-15（Windows 侧）：光标可见性、粘贴后多行编辑、产物复验

测试日期 2026-10-30；WezTerm `20240203-110809-5046fc22`（`E:\compiler\WezTerm`）；
被测二进制 `commit e4ff244` / sha256 `e4db7fe338b64a2b…` / `built 2026-10-30T12:32:52Z`（win-fixes 分支自编）。
全部测试文本为合成内容（`SYN-*` / 连续 `x`），不含用户数据。

## W1：硬件光标可见性对终端光标坐标的影响

`PI_TUI_CURSOR_DUMP` 记录我们请求的 `(hw_row, col)`，`wezterm cli list --format json` 是终端实际
`(cursor_x, cursor_y)`。配置 A = 默认（`showHardwareCursor` 缺省 false），配置 B = 同一份配置副本加
`"showHardwareCursor": true`。

| 模式 | 步骤 | 终端实测 (x, y) | pi-rs 算出 (col, hw_row) | 可见性 |
|---|---|---|---|---|
| A 默认（隐藏） | boot | (129, 22) | (0, 14) | Hidden |
| A | 输入 `abc` | (129, 19) | (3, 14) | Hidden |
| A | 粘贴 3 行 | (129, 25) | (0, 17) | Hidden |
| A | `Shift+Tab` | (129, 27) | (0, 19) | Hidden |
| B 显示 | boot | (0, 15) | (0, 9) | Visible |
| B | 输入 `abc` | (3, 15) | (3, 9) | Visible |
| B | 粘贴 3 行 | (0, 18) | (0, 12) | Visible |
| B | `Shift+Tab` | (0, 21) | (0, 15) | Visible |

结论：
- **显示时列完全一致、行差恒定 origin=6**（面板上方有 cmd 的输出），逐帧无漂移；
- **隐藏时终端报告的坐标停在"最后一次写屏"的位置**（x 恒为最右列 129，y 与编辑器所在行无关，
  且四种状态间偏移不恒定：8/5/8/8）⇒ 输入法候选框会落到错误位置。这就是 W1c 要在文档里写明的机制。
- 默认值仍是 `false`（与 ref `packages/tui/src/tui.ts:478` 相同），本批不改默认。

## W1b：ref 取值与 pi-rs 的对照

| 项 | ref | pi-rs（本批后） |
|---|---|---|
| 设置项 | `core/settings-manager.ts:151` `showHardwareCursor?: boolean` | 同名键（`#[serde(rename = "showHardwareCursor")]`） |
| 取值顺序 | `:1369-1370` `settings.showHardwareCursor ?? process.env.PI_HARDWARE_CURSOR === "1"` | 同（本批补齐 env 兜底；此前只在 `PI_CODING_AGENT_DIR` 显式设置时读 settings） |
| 默认值 | `tui.ts:478` `false` | `false`（未改） |
| 生效 | `interactive-mode.ts:580` → TUI `setShowHardwareCursor` | 每帧写 `ESC[?25h` / `ESC[?25l` |

## W2：粘贴后 Up/Down/Home/End（真实 SendInput）

### 3 行粘贴（编辑框 4 行，光标在末行行首）

| 步骤 | 结果 (hw_row, col) | 与 ref `editor.ts:923-960` 对照 |
|---|---|---|
| 粘贴后 | (12, 0) | 末行行首 |
| Up ×3 | (11,0) → (10,0) → (9,0) | `moveCursor(-1,0)` 保持列 ✓ |
| End | (9, 5) | `moveToLineEnd` ✓（首行 `SYN-A` 长 5） |
| 首行再 Up | (9, 0) | 首行且列 >0 ⇒ `moveToLineStart` ✓ |
| 再 Up | (9, 0) | 列 0 且无历史 ⇒ 不动 ✓ |
| Down ×3 | (10,0) → (11,0) → (12,0) | 逐行下移 ✓ |
| 末行 Down / Home | (12,0) / (12,0) | `moveToLineEnd` / `moveToLineStart` ✓ |

### 12 行折叠（编辑框只有 `[paste #1 +13 lines]` 一行）

| 步骤 | 结果 | 说明 |
|---|---|---|
| 粘贴后 | (9, 20) | 标记行末 |
| Up | (9, 0) | 首行且列 >0 ⇒ 跳到行首 ✓ |
| 再 Up | (9, 0) | 无历史 ⇒ 不动 ✓ |
| Down | (9, 20) | 末行 ⇒ `moveToLineEnd` ✓ |
| Home / End | (9,0) / (9,20) | ✓ |

### 本轮修掉的缺陷

`marker_apply`（`crates/pi-tui/src/lib.rs`）把光标标记**一律画在编辑器最后一行**，只按
`state.cursor - 最后一行起点` 算列 ⇒ Up/Down 把光标移到上一行时，硬件光标/输入法框仍停在最后一行
（真机表现为"上移后光标不动"）。改为按**光标所在逻辑行**定位（行号 + 该行内列），新增
`w2_marker_line_tests`（3 行：末行行首、上移一行/两行、行内右移）与
`w2_multiline_cursor_tests`（Up 后落在目标行行首，符合 `moveCursor` 的保列语义）。

## W3：`ESC[201~` 残留

本轮所有真实按键运行（W1 8 次 + W2 2×11 步 + 复验若干）都同时开了 `PI_TUI_EVENT_DUMP`：
**事件日志里 `200~` / `201~` 出现次数均为 0**（每次 46–182 行，全是 `Key(...)`）。
仍无法复现，按批要求停止追查，登记为"未复现，待用户在最新 exe 上用 `PI_TUI_EVENT_DUMP` 再抓一次"。

## W4：产物与 RECHECK

- 产物：`commit e4ff244`、sha256 `e4db7fe338b64a2b…`、`built 2026-10-30T12:32:52Z`（无 `-dirty`）；
- RECHECK 1–11 项在该产物上全部通过：启动无垃圾字符 / `abc` 不翻倍 / `Ctrl+D` 退出码 0 /
  `Ctrl+P` 切换 / `Shift+Tab` 档位 / 工具一轮 `Took 0.4s` / 退出提示 / footer `200k`+`(provider)` /
  3 行粘贴不提交、12 行折叠 / 1242 字符单行折叠 `[paste #1 1242 chars]` /
  `Shift+Enter` 与 `Ctrl+Enter` 换行且不提交；
- `tools/windows/RECHECK.md` 的 sha256 与 commit 已按此产物更新。
