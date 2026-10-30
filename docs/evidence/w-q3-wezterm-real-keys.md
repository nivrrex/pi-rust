# W-Q3：WezTerm 真实按键路径（SendInput Shift+Insert）实测

测试日期 2026-10-30；机器 Windows 10 LTSC 19044；WezTerm `20240203-110809-5046fc22`
（`E:\compiler\WezTerm`）；被测 `target/x86_64-pc-windows-gnu/release/pi-rs.exe`
（`built 2026-10-30T09:58:17Z commit 5710731`，本机自编，工作区仅 SMB 权限位差异）。
全部测试文本为合成内容（`SYN-LINE-NN` / 连续 `x`），不涉及任何用户数据或个人文本。

## 方法

1. `wezterm cli spawn` 开面板 → `send-text --no-paste` 输入 `set PI_TUI_EVENT_DUMP=… && pi-rs.exe`；
2. 驱动把剪贴板设成合成文本，`AttachThreadInput + SetForegroundWindow` 聚焦 **WezTerm 窗口**，
   用 `SendInput` 发真实的 **Shift+Insert**（终端自己执行粘贴，而不是 `send-text` 走 bracketed paste）；
3. `wezterm cli get-text` 读回屏幕；`PI_TUI_EVENT_DUMP` 的日志只解析**事件类型与间隔**。

## 事件形态（与用户日志一致）

每次粘贴到达应用的形状都是：

- **逐键** `Key(KeyEvent { code: Char(..), kind: Press })` + 对应 `Release`，间隔 ≲ 1–2 ms；
- 行间换行是 **`Key(Enter, modifiers: CONTROL)`**；
- **没有** `Event::Paste`，**没有** `ESC[200~` / `ESC[201~`（50 次运行全部如此）。

即：WezTerm 的 Shift+Insert 不开启括号粘贴，`ConPTY` 把整段文本还原成普通按键流 —— 这正是用户报告的形态。

## 矩阵（每格 10 次）

| 场景 | 折叠 | 误提交 | `ESC[200~` / `ESC[201~` | `Paste` 事件 |
|---|---|---|---|---|
| 3 行 | 0/10（应不折叠） | 0/10 | 0 | 0 |
| 8 行 | 0/10（应不折叠） | 0/10 | 0 | 0 |
| 11 行 | **10/10** | 0/10 | 0 | 0 |
| 30 行 | **10/10** | 0/10 | 0 | 0 |
| 1200 字符（单行） | **10/10** | 0/10 | 0 | 0 |

## 本轮发现并修复的三处（均带单测）

1. **单行 >1000 字符永不折叠**（首轮 1200 字符 5 次里 1 次、修复前 4/5 折叠）
   `PasteBurst::take()` 原先只在"含换行"时产出 `Paste`，而 ref `editor.ts:1296-1312` 的阈值是
   "**>10 行 或 >1000 字符**"。补 `PASTE_FOLD_THRESHOLD = 1000`。
2. **1200 字符偶发被切成两段**（两段都不到 1000 ⇒ 不折叠）
   粘贴近乎每 1 ms 一个键，但偶发 >200 ms 的调度停顿会把一段切开。
   现在用**速率判定**：`chars >= 64` 且平均间隔 `<= 5 ms` ⇒ 认定为粘贴，空闲窗口放宽到 600 ms。
   人类打字达不到 5 ms/字，故不影响正常输入。
3. **Shift+Enter 被当成提交**（复核 Q3-d 时发现）
   ref `packages/tui/src/keybindings.ts:143`：`tui.input.newLine = ["shift+enter", "ctrl+j"]`。
   我们此前只把 `Ctrl+Enter`/`Ctrl+J` 当换行，`Shift+Enter` 落到 `Enter` ⇒ 直接提交。
   已改为 `Enter + (CONTROL | SHIFT) ⇒ Newline`，并在 WezTerm 用真实按键验证：
   `aaa` + Shift+Enter + `bbb` + Ctrl+Enter + `ccc` 三行都在编辑框内，未提交。

## 与用户日志的差异

用户日志里除"逐键 + Ctrl+Enter"外还提到**结尾残留 `ESC[201~` 字符**。本轮 50 次真实粘贴
**一次都没出现**（`marker_201` 全 0）。可能原因：用户的粘贴经带括号标记的通道
（如 Windows Terminal 或其它终端的 Ctrl+V），或旧版本；WezTerm 版本/配置也可能不同。
未复现，按要求记录差异而不是改代码。

## 单测

- `q3_wezterm_paste_tests`：1200 字符单行 → 一次 `Paste`；3 行逐键 + `Ctrl+Enter` → 一次 `Paste` 且无 `Enter`；
- `q3_newline_binding_tests`：`Shift+Enter` / `Ctrl+Enter` / `Ctrl+J` → `Newline`，裸 `Enter` → `Enter`；
- 既有 `windows_input_tests` 全绿；`pi-cli --lib` **126/0**，`pi-tui --lib` 74/0。

## 未覆盖

- Q2d（光标逐帧对照）需 Linux 侧 Q5 完成后同口径录制，本批未做；
- 用户的 WezTerm 版本/配置、输入法状态未记录，无法对齐"残留标记"的具体触发条件。
