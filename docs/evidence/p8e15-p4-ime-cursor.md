# P4：输入法/硬件光标线索（只读取证，P8e-15）

## ref 的三处事实
| 项 | 位置 | 值 |
|---|---|---|
| 设置项 | `core/settings-manager.ts:151` | `showHardwareCursor?: boolean`（注释：*Show terminal cursor while still positioning it for IME*） |
| 读取 | `core/settings-manager.ts:1369-1370` | `this.settings.showHardwareCursor ?? process.env.PI_HARDWARE_CURSOR === "1"` |
| 生效 | `modes/interactive/interactive-mode.ts:580` | 传给 TUI：`showHardwareCursor: this.settingsManager.getShowHardwareCursor()` |
| TUI 默认 | `packages/tui/src/tui.ts:478` | `private showHardwareCursor = false`（**默认关**） |
| 关掉时 | `tui.ts:528-534` | `setShowHardwareCursor(false)` ⇒ `terminal.hideCursor()` |
| 光标标记 | `packages/tui/src/tui.ts:168` | `CURSOR_MARKER = "\x1b_pi:c\x07"` —— 组件在"希望硬件光标落在这里"的位置输出该标记，`layout.ts:122` 扫描它得出光标行 |

## pi-rs 现状
- 设置模型里已有 `show_hardware_cursor: Option<bool>`（`crates/pi-cli/tests/t3_31_settings.rs` 引用），
  **默认值也是关**，与 ref 一致；
- 但我们**没有** `PI_HARDWARE_CURSOR=1` 环境变量覆盖，也**没有**在隐藏硬件光标时的
  "仍为 IME 定位光标"逻辑：我们是始终隐藏硬件光标、只把真实光标定位到编辑框位置
  （`ESC[?25l` + 每帧 `ESC[<row>;<col>H`）。

## 结论与风险
- 默认值一致（都关），**本轮不改默认值**；
- 差异点：ref 允许用户 `showHardwareCursor=true` 时**显示**终端光标以便输入法候选框跟随；
  pi-rs 目前没有这条路径 ⇒ 在**候选框不跟随**的终端/输入法组合下，用户没有开关可试。
  这属于"缺一个可选开关"，不影响默认行为，登记在 `LIMITATIONS.md`（下一窗口补实现与设置项）。
