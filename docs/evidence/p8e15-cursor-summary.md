# Q2d 收口：光标位置两侧对照（P8e-15）

## 结论
**Linux（tmux）与 Windows（`PI_TUI_CURSOR_DUMP`）两侧，pi-rs 的光标落点都与 TS / 终端实际
落点一致**；12 行粘贴后的差异（此前 4 行）已由 P8e-13 的"一条多行提示"修复消除。

## Linux：tmux 同口径（`#{cursor_x},#{cursor_y}`）
12 行括号粘贴后逐动作比较（详见 `docs/evidence/p8e12-v6-cursor.md`）：

| 动作 | TS | pi-rs |
|---|---|---|
| 粘贴后 | `20,17` | `20,17` |
| Up / Down | `0,17` / `20,17` | `0,17` / `20,17` |
| Left×3 / Home / End | `0,17` / `0,17` / `20,17` | 同 |
| 输入 4 个全角字 | `28,17` | `28,17` |
| 退格×2 | `24,17` | `24,17` |

## Windows：`PI_TUI_CURSOR_DUMP`（真实 conhost，120×30）
Windows 侧采到的帧（`d42693a` 落地、`3318cf6` 增补列）：

```
frame=1 size=120x30 hw_row=14 col=0  input=0 busy=false thinking=high
frame=2 size=120x30 hw_row=14 col=2  input=2 busy=false thinking=high
frame=3 size=120x30 hw_row=16 col=2  input=2 busy=false thinking=off
frame=4 size=120x30 hw_row=16 col=0  input=0 busy=false thinking=off
```

- `Shift+Tab`（frame2→3）：`hw_row` 14→16（状态行插入）、`thinking` high→off，与 TS 一致；
- `Ctrl+C`（frame3→4）：`col`/`input` 归零；
- `col` 与 `input` 同步 ⇒ 我们写出的 `ESC[<col+1>G` 与编辑框内部光标一致。

## 已知前提
- **隐藏硬件光标**（`?25l`）时，部分终端不会更新光标坐标；此时只能比较"我们写出的目标位置"
  与实际渲染文本的位置，`tmux`/ConPTY 报的坐标可能滞后一帧。
- Windows 侧数据来自真实 conhost；Windows Terminal 下未采（本机未安装）。
