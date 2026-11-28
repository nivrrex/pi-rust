# W-Q2d（Windows 半边）：pi-rs 自算光标 vs WezTerm 实际光标

测试日期 2026-10-30；WezTerm `20240203-110809-5046fc22`；被测
`target/x86_64-pc-windows-gnu/release/pi-rs.exe`（`built 2026-10-30T10:20:36Z commit 5710731`）。
测试文本全为合成内容。

## 为什么需要 `showHardwareCursor: true`

`PI_TUI_CURSOR_DUMP` 记录的是**我们请求的**行列（`hw_row` / `col`），要跟终端实际光标对照，
得让终端光标可见。实测（最小复现）：

| 场景 | `wezterm cli list` 的 `cursor_x` |
|---|---|
| `?25l`（隐藏）+ `ESC[5G` | **0**（不反映 G 的列） |
| `?25h`（显示）+ `ESC[5G` | 4（正确） |

所以对照必须用 `settings.json` 的 `showHardwareCursor: true`（本次用一份拷贝的配置目录，
`PI_CODING_AGENT_DIR` 指过去），否则 `cursor_x` 是无意义的旧值。

## 方法

1. WezTerm 面板里以 `PI_TUI_CURSOR_DUMP=<log>` + `PI_CODING_AGENT_DIR=<cfg-cursor>` 启动 pi-rs；
2. 每个动作后停顿 0.8–1.2 s，用 `wezterm cli list --format json` 采样 4 次取 `cursor_x/cursor_y`；
3. 取 pi-rs 日志里**最后一帧**的 `hw_row/col` 与之对照。

## 结果（5 个状态）

| 步骤 | WezTerm (x, y) | pi-rs (col, hw_row) | x 相等 | y − hw_row |
|---|---|---|---|---|
| boot | (0, 15) | (0, 9) | ✓ | 6 |
| 输入 `abc` | (3, 15) | (3, 9) | ✓ | 6 |
| 再输入 `defg` | (7, 15) | (7, 9) | ✓ | 6 |
| `Shift+Tab`（真实按键） | (7, 18) | (7, 12) | ✓ | 6 |
| 粘贴 3 行（真实 Shift+Insert） | (0, 21) | (0, 15) | ✓ | 6 |

**结论**：列完全一致；行差一个**恒定 origin=6**（pi-rs 在主屏上从当前光标处开始绘制，
该面板前面有 cmd 的几行输出），逐帧没有漂移 ⇒ 我们请求的光标位置与终端实际落点一致。

## 未覆盖

- 与 **TS 侧**逐帧对照（Q2d 的原意）仍需 Linux 侧 Q5 用同一矩阵录制；本文件只验证
  "pi-rs 请求的位置 == 终端实际位置"这一半。
- `PI_TUI_CURSOR_DUMP` 本轮新增 `ms=<相对毫秒>` 字段，供跨进程对齐时间轴。
