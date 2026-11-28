# P8e-12 V2：WezTerm（真实括号粘贴）下的场景矩阵

实测日期 2026-10-30；WezTerm 20240203-110809-5046fc22（`E:\compiler\WezTerm`），
`wezterm cli spawn/send-text/get-text` 驱动，被测 `target/x86_64-pc-windows-gnu/release/pi-rs.exe`
（本机自编 `d04d3f6-dirty`，含 V1 的 `WindowsInput` 改动）。

## 方法

1. `wezterm cli spawn` 开一个交互式 cmd 面板，`send-text --no-paste` 输入 `run-pi.cmd` 启动 pi-rs；
2. 用 `wezterm cli send-text`（**不加** `--no-paste`）注入文本——WezTerm 对已开 `?2004h` 的应用
   会以 `ESC[200~ … ESC[201~` 的括号粘贴发送，这正是 V2 要覆盖的真实路径；
3. `wezterm cli get-text` 读回面板内容判定是否提交（footer 百分比是否变化、是否出现 `Working`）。

## 矩阵

| # | 场景 | 结果 | 判定 |
|---|---|---|---|
| 1 | 粘贴 3 行（`L1⏎L2⏎L3⏎`） | 编辑框内显示 3 行，footer 仍 `0.0%`、无 `Working` | **不提交**（与 ref 一致） |
| 2 | 粘贴 12 行（`l1…l12`） | 折叠为 `[paste #1 +13 lines]`，footer 仍 `0.0%` | **不提交**；折叠阈值与 ref `editor.ts:1296-1312`（>10 行）一致 |
| 3 | 单行带换行（`single⏎`） | 编辑框 `single` + 一个空行，未提交 | **不提交**（换行并入文本） |
| 4 | 场景 2 之后再按回车 | 整段 12 行作为**一条**用户消息发出，出现 `Working`、footer `↑… ↓…` | **整段提交** |

## 结论

- WezTerm 的括号粘贴在 Windows 上能完整到达 pi-rs（V1 的 `BracketPaste` 双标记状态机 + `PasteBurst`
  组合正确），折叠阈值、换行并入、回车整段提交三项行为都符合 ref；
- 与 conhost 的差异是**输入通道**：conhost 不发标记（见 `p8e12-v3-paste-markers.md`），靠启发式
  `PasteBurst` 兜底；WezTerm 发标记，走 `BracketPaste`。两条路径都已实测。

## 未覆盖

- `Ctrl+V`：`wezterm cli send-text` 发送的是文本/字节，无法模拟"终端自身的粘贴键"；
  需要真人按键。不过 WezTerm 的粘贴通道本身已由 1–4 覆盖。
- Windows Terminal：本机未安装，未测。
